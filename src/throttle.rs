use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::time;

use crate::util;

/// How often the throttle drains tokens: `delay_for` has millisecond granularity.
const TICK: time::Duration = time::Duration::from_millis(10);

/// The largest throttle, in requests per second, that the command line, the
/// Controllers and the dashboard accept, and the capacity of every throttle's
/// channel, whatever rate its run starts at, so a running throttle can go
/// anywhere from 1 to this.
///
/// The channel holds `bool`s in one `VecDeque`, so full of fillers it holds
/// 1 MiB (1,048,576 bytes, the next power of two): measured by the ignored
/// test `a_full_channel_costs` in a release build, resident memory rose by
/// 1.3 MB and the first fill took 5 ms on an Apple silicon laptop, and by
/// 1.5 MB in 17.7 ms on a 4 vCPU cloud burst worker. The fill runs once, at
/// the start, before any user.
pub(crate) const MAX_THROTTLE: usize = 1_000_000;

/// The live throttle's rate, shared by the drain task and the main loop, which
/// changes it while the run goes on.
pub(crate) type ThrottleRate = Arc<AtomicUsize>;

/// A running throttle: the users' sender, the sender that tells the drain
/// task the load test is over, and the live rate.
pub(crate) struct Throttle {
    pub(crate) users: flume::Sender<bool>,
    pub(crate) stop: flume::Sender<bool>,
    pub(crate) rate: ThrottleRate,
}

/// Starts a throttle of `rate` requests a second: builds its channel, fills
/// all but one slot so the run does not start with a burst, and spawns the
/// drain task.
pub(crate) fn start(rate: usize) -> Throttle {
    let capacity = MAX_THROTTLE;
    // A bounded channel the users send a token into before each request.
    let (users, receiver) = flume::bounded(capacity);
    // The parent tells the drain task the load test is over on this one.
    // Even though only one message is sent, a oneshot channel won't do, as
    // the drain task must not block waiting for it.
    let (stop, stopped) = flume::bounded(1);
    let rate: ThrottleRate = Arc::new(AtomicUsize::new(rate));
    // We start from 1 instead of 0 to intentionally fill all but one slot in
    // the channel to avoid a burst of traffic during startup. The channel then
    // provides an implementation of the leaky bucket algorithm as a queue.
    for _ in 1..capacity {
        if users.try_send(true).is_err() {
            break;
        }
    }
    tokio::spawn(throttle_main(
        capacity,
        rate.clone(),
        users.clone(),
        receiver,
        stopped,
    ));
    Throttle { users, stop, rate }
}

/// This throttle thread limits the maximum number of requests that can be made across
/// all GooseUser threads. When enabled, GooseUser threads must add a token to the
/// bounded channel before making a request, and this thread limits how frequently
/// tokens are removed thereby throttling how fast requests can be made. It is an
/// implementation of the leaky bucket algorithm as a queue: instead of leaking the
/// overflow we asynchronously block. More information on the leaky bucket algorithm
/// can be found at: https://en.wikipedia.org/wiki/Leaky_bucket
///
/// The rate is read from `rate` each tick, so a Controller or the dashboard
/// changes it, anywhere from 1 to [`MAX_THROTTLE`], without touching the
/// users. The channel's capacity is [`MAX_THROTTLE`] whatever the rate, and
/// never changes, as the users hold senders to it. The burst follows the
/// rate: after a quiet spell users may send one second of the current rate at
/// once. The thread keeps its own filler tokens in the channel so no more than
/// that is free, which with an unchanged rate is none once users fill it: the
/// channel then behaves exactly as a fixed throttle of that rate.
async fn throttle_main(
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

        // Keep no more than one second of the current rate free: the
        // fillers first, as after the rate was lowered, so no user takes a
        // slot past that between this and the drain.
        let held = capacity.saturating_sub(per_second.min(capacity));
        while throttle_receiver.len() < held {
            if filler.try_send(true).is_err() {
                break;
            }
        }

        // Remove tokens from the channel, freeing spots for requests to be
        // made, but never below the fillers held: those slots stay taken, so
        // a quiet spell frees one second of the current rate and no more.
        // Users only add to the channel meanwhile, so it never goes below.
        // With the fillers held, the channel is as empty as a fixed throttle
        // of this rate is when it has nothing left to drain.
        let drainable = throttle_receiver.len().saturating_sub(held);
        if tokens > drainable {
            debug!("[throttle]: drained to the fillers held, after removing {drainable} tokens");
            owed = 0.0;
        }
        for _ in 0..tokens.min(drainable) {
            if throttle_receiver.try_recv().is_err() {
                owed = 0.0;
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

    /// How many tokens users sending as fast as they can get in over
    /// `seconds`, after taking any burst first.
    async fn sent_over(sender: &flume::Sender<bool>, seconds: f64) -> usize {
        burst(sender);
        let sender = sender.clone();
        let sending = tokio::spawn(async move {
            let mut sent = 0_usize;
            let end = time::Instant::now() + time::Duration::from_secs_f64(seconds);
            while let Ok(Ok(())) = time::timeout_at(end, sender.send_async(true)).await {
                sent += 1;
            }
            sent
        });
        sending.await.unwrap()
    }

    /// Asserts `got` is within `percent` of `want`.
    fn near(what: &str, got: usize, want: usize, percent: usize) {
        let slack = (want * percent).div_ceil(100).max(2);
        assert!(
            got + slack >= want && got <= want + slack,
            "{}: {}, want {} within {}%",
            what,
            got,
            want,
            percent
        );
    }

    const QUIET: time::Duration = time::Duration::from_millis(1_500);

    /// After a quiet spell, users may send one second of the current rate
    /// at once, whatever the rate the run started at: raised from 100 to
    /// 400 a second the burst is 400, and lowered to 50 after a quiet spell,
    /// with 400 slots free, the fillers take back all but 50 of them.
    #[tokio::test]
    // Measures a rate in real time: never beside the tests that build
    // clients on every CPU, which starve its filler.
    #[serial_test::serial(cpu)]
    async fn the_burst_follows_the_rate() {
        let test = async {
            let throttle = start(100);
            burst(&throttle.users);
            throttle.rate.store(400, Ordering::Relaxed);
            tokio::time::sleep(QUIET).await;
            // Exact: on this one thread a burst runs between two ticks, and
            // the drain never takes the channel below the fillers held.
            assert_eq!(burst(&throttle.users), 400, "a burst at 400 a second");
            tokio::time::sleep(QUIET).await;
            throttle.rate.store(50, Ordering::Relaxed);
            tokio::time::sleep(QUIET).await;
            assert_eq!(burst(&throttle.users), 50, "a burst at 50 a second");
            throttle.stop.send(true).unwrap();
        };
        tokio::time::timeout(time::Duration::from_secs(30), test)
            .await
            .expect("within 30 seconds");
    }

    /// A throttle started at 1 a second rises to 1,000, 1,000 times its
    /// start, past the 100 times a channel of one second of the starting
    /// rate allowed: both its rate and its burst follow.
    #[tokio::test]
    // Measures a rate in real time: never beside the tests that build
    // clients on every CPU, which starve its filler.
    #[serial_test::serial(cpu)]
    async fn a_throttle_rises_past_100_times_its_start() {
        let test = async {
            let throttle = start(1);
            burst(&throttle.users);
            throttle.rate.store(1_000, Ordering::Relaxed);
            near(
                "sent over 2 s at 1,000 a second",
                sent_over(&throttle.users, 2.0).await,
                2_000,
                5,
            );
            tokio::time::sleep(QUIET).await;
            assert_eq!(burst(&throttle.users), 1_000, "a burst at 1,000 a second");
            throttle.stop.send(true).unwrap();
        };
        tokio::time::timeout(time::Duration::from_secs(30), test)
            .await
            .expect("within 30 seconds");
    }

    /// With an unchanged rate the throttle behaves exactly as a fixed one:
    /// one slot free at the start, none once full, one second of the rate
    /// after a quiet spell, and the rate held while users send.
    #[tokio::test]
    // Measures a rate in real time: never beside the tests that build
    // clients on every CPU, which starve its filler.
    #[serial_test::serial(cpu)]
    async fn an_unchanged_rate_is_a_fixed_throttle() {
        let test = async {
            let throttle = start(200);
            assert_eq!(burst(&throttle.users), 1, "the start is all but full");
            assert_eq!(burst(&throttle.users), 0, "no free slots once full");
            tokio::time::sleep(QUIET).await;
            assert_eq!(burst(&throttle.users), 200, "a burst at 200 a second");
            near(
                "sent over 2 s at 200 a second",
                sent_over(&throttle.users, 2.0).await,
                400,
                5,
            );
            throttle.stop.send(true).unwrap();
        };
        tokio::time::timeout(time::Duration::from_secs(30), test)
            .await
            .expect("within 30 seconds");
    }

    /// This process's resident memory in bytes.
    fn rss() -> usize {
        if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
            let kb = status
                .lines()
                .find_map(|line| line.strip_prefix("VmRSS:"))
                .and_then(|kb| {
                    kb.trim()
                        .trim_end_matches("kB")
                        .trim()
                        .parse::<usize>()
                        .ok()
                });
            return kb.unwrap_or(0) * 1024;
        }
        let out = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output()
            .expect("ps");
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse::<usize>()
            .unwrap_or(0)
            * 1024
    }

    /// The memory of a channel full of fillers, and the time of the first
    /// fill: run with `--ignored --nocapture`, best in a release build.
    #[tokio::test]
    #[ignore]
    async fn a_full_channel_costs() {
        let before = rss();
        let began = std::time::Instant::now();
        let throttle = start(1);
        let filled = began.elapsed();
        let after = rss();
        println!(
            "a_full_channel_costs: first fill {:.1} ms; resident memory {} -> {} bytes (+{} bytes); {} tokens held",
            filled.as_secs_f64() * 1_000.0,
            before,
            after,
            after.saturating_sub(before),
            throttle.users.len()
        );
        throttle.stop.send(true).unwrap();
    }
}
