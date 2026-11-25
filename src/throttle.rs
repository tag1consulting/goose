use tokio::time;

use crate::util;

/// This throttle thread limits the maximum number of requests that can be made across
/// all GooseUser threads. When enabled, GooseUser threads must add a token to the
/// bounded channel before making a request, and this thread limits how frequently
/// tokens are removed thereby throttling how fast requests can be made. It is an
/// implementation of the leaky bucket algorithm as a queue: instead of leaking the
/// overflow we asynchronously block. More information on the leaky bucket algorithm
/// can be found at: https://en.wikipedia.org/wiki/Leaky_bucket
pub async fn throttle_main(
    throttle_requests: usize,
    throttle_receiver: flume::Receiver<bool>,
    parent_receiver: flume::Receiver<bool>,
) {
    // Use microseconds to allow configurations up to 1,000,000 requests per second.
    let mut sleep_duration = time::Duration::from_micros(1_000_000 / throttle_requests as u64);
    let tokens_per_duration;

    let ten_milliseconds = time::Duration::from_millis(10);
    debug!("sleep_duration: {sleep_duration:?} ten_milliseconds: {ten_milliseconds:?}");

    // Keep sleep_duration at least ~10ms as `delay_for` has millisecond granularity.
    if sleep_duration < ten_milliseconds {
        tokens_per_duration = (ten_milliseconds.as_nanos() / sleep_duration.as_nanos()) as u32;
        sleep_duration *= tokens_per_duration;
    } else {
        tokens_per_duration = 1;
    }

    info!("[throttle]: allowing {tokens_per_duration} request(s) every {sleep_duration:?}");

    // One or more token gets removed from the throttle_receiver bucket at regular
    // intervals. The throttle_drift variable tracks how much time is spent on
    // everything else, and is subtracted from the time spent sleeping.
    let mut throttle_drift = tokio::time::Instant::now();
    // The tokens owed since the last tick, a fraction of one when a wake ran
    // late by less than a whole tick.
    let mut owed = 0.0_f64;
    let mut last_tick = tokio::time::Instant::now();

    // Loop and remove tokens from channel at controlled rate until load test ends.
    loop {
        throttle_drift = util::sleep_minus_drift(sleep_duration, throttle_drift).await;

        // A message will be received when the load test is over.
        if parent_receiver.try_recv().is_ok() {
            // Close throttle channel to prevent any further requests.
            info!("[throttle]: load test complete, closing throttle channel");
            drop(throttle_receiver);
            break;
        }

        // Wakes run late, so drain for the time since the last wake rather
        // than for one sleep_duration, but at most two: a late tick is made
        // up, at most one, and a thread held for long drains no faster than
        // it would have.
        let now = tokio::time::Instant::now();
        let elapsed = now.duration_since(last_tick).min(sleep_duration * 2);
        last_tick = now;
        owed += tokens_per_duration as f64 * elapsed.as_secs_f64() / sleep_duration.as_secs_f64();
        let tokens = owed as u32;
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
    }
}
