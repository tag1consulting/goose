use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::time;

use crate::util;

/// How often the throttle drains tokens: `delay_for` has millisecond granularity.
const TICK: time::Duration = time::Duration::from_millis(10);

/// How far a running throttle can rise past the one its run started with:
/// the channel holds one second of the starting rate, and a tick drains at
/// most what it holds, so 100 ticks a second reach 100 times that.
pub(crate) const MAX_THROTTLE_RISE: usize = (1_000 / TICK.as_millis()) as usize;

/// The live throttle's rate, shared by the drain task and the main loop, which
/// changes it while the run goes on.
pub(crate) type ThrottleRate = Arc<AtomicUsize>;

/// This throttle thread limits the maximum number of requests that can be made across
/// all GooseUser threads. When enabled, GooseUser threads must add a token to the
/// bounded channel before making a request, and this thread limits how frequently
/// tokens are removed thereby throttling how fast requests can be made. It is an
/// implementation of the leaky bucket algorithm as a queue: instead of leaking the
/// overflow we asynchronously block. More information on the leaky bucket algorithm
/// can be found at: https://en.wikipedia.org/wiki/Leaky_bucket
///
/// The rate is read from `rate` each tick, so a Controller or the dashboard
/// changes it without touching the users. The channel's capacity, one second
/// of the starting rate, never changes. The burst follows the rate: after a
/// quiet spell users may send one second of the current rate at once, never
/// more than the channel holds. The thread keeps its own filler tokens in the
/// channel so no more than that is free, which with an unchanged rate is
/// none: the channel then behaves exactly as a fixed throttle.
pub async fn throttle_main(
    capacity: usize,
    rate: ThrottleRate,
    filler: flume::Sender<bool>,
    throttle_receiver: flume::Receiver<bool>,
    parent_receiver: flume::Receiver<bool>,
) {
    info!(
        "[throttle]: allowing {} request(s) a second",
        rate.load(Ordering::Relaxed)
    );

    // The tokens owed since the last tick, a fraction of one at low rates.
    let mut owed = 0.0_f64;
    // The throttle_drift variable tracks how much time is spent on everything
    // else, and is subtracted from the time spent sleeping.
    let mut last_tick = time::Instant::now();
    let mut throttle_drift = time::Instant::now();

    // Loop and remove tokens from channel at controlled rate until load test ends.
    loop {
        throttle_drift = util::sleep_minus_drift(TICK, throttle_drift).await;

        // A message will be received when the load test is over.
        if parent_receiver.try_recv().is_ok() {
            // Close throttle channel to prevent any further requests.
            info!("[throttle]: load test complete, closing throttle channel");
            drop(throttle_receiver);
            break;
        }

        let now = time::Instant::now();
        let per_second = rate.load(Ordering::Relaxed);
        // The time since the last wake, as wakes run late, but at most two
        // ticks: a thread held for long drains no faster than it would have.
        owed += per_second as f64 * now.duration_since(last_tick).min(TICK * 2).as_secs_f64();
        last_tick = now;
        // A tick drains at most what the channel holds.
        owed = owed.min(capacity as f64);
        let tokens = owed as usize;
        owed -= tokens as f64;
        debug!("[throttle]: removing {tokens} token(s) from channel");

        // Remove tokens from the channel, freeing spots for request to be made.
        for token in 0..tokens {
            // If the channel is empty, we will get an error, so stop trying to remove tokens.
            if throttle_receiver.try_recv().is_err() {
                debug!("[throttle]: empty channel, exit after removing {token} tokens");
                owed = 0.0;
                break;
            }
        }

        // Keep no more than one second of the current rate free.
        let held = capacity.saturating_sub(per_second.min(capacity));
        while throttle_receiver.len() < held {
            if filler.try_send(true).is_err() {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How many tokens a sender gets in at once: the burst.
    fn burst(sender: &flume::Sender<bool>) -> usize {
        let mut sent = 0;
        while sender.try_send(true).is_ok() {
            sent += 1;
        }
        sent
    }

    /// After a quiet spell, users may send one second of the current rate
    /// at once, never more than one second of the starting rate.
    #[tokio::test]
    async fn the_burst_follows_the_rate() {
        let test = async {
            let (sender, receiver) = flume::bounded(200);
            let (stop, stopped) = flume::bounded(1);
            let rate: ThrottleRate = Arc::new(AtomicUsize::new(200));
            tokio::spawn(throttle_main(
                200,
                rate.clone(),
                sender.clone(),
                receiver,
                stopped,
            ));
            // Full: no burst at the starting rate once the channel fills.
            burst(&sender);
            rate.store(50, Ordering::Relaxed);
            tokio::time::sleep(time::Duration::from_millis(1_500)).await;
            let low = burst(&sender);
            assert!(
                (40..=60).contains(&low),
                "a burst of {} at 50 a second",
                low
            );
            rate.store(200, Ordering::Relaxed);
            tokio::time::sleep(time::Duration::from_millis(1_500)).await;
            let high = burst(&sender);
            assert!(
                (150..=200).contains(&high),
                "a burst of {} at 200 a second",
                high
            );
            stop.send(true).unwrap();
        };
        tokio::time::timeout(time::Duration::from_secs(30), test)
            .await
            .expect("within 30 seconds");
    }
}
