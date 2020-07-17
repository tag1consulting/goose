use tokio::sync::mpsc::Receiver;
use tokio::time;

/// This throttle thread limits the maximum number of requests that can be made across
/// all GooseUser threads. When enabled, GooseUser threads must add a token to the
/// bounded channel before making a request, and this thread limits how frequently
/// tokens are removed thereby throttling how fast requests can be made. It is an
/// implementation of the leaky bucket algorithm as a queue: instead of leaking the
/// overflow we asynchronously block. More information on the leaky bucket algorithm
/// can be found at: https://en.wikipedia.org/wiki/Leaky_bucket
pub async fn throttle_main(
    throttle_requests: usize,
    mut throttle_receiver: Receiver<bool>,
    mut parent_receiver: Receiver<bool>,
) {
    // Use microseconds to allow configurations up to 1,000,000 requests per second.
    let mut sleep_duration = time::Duration::from_micros(1_000_000 / throttle_requests as u64);
    let tokens_per_duration;

    let ten_milliseconds = time::Duration::from_millis(10);
    debug!(
        "sleep_duration: {:?} ten_milliseconds: {:?}",
        sleep_duration, ten_milliseconds
    );

    // Keep sleep_duration at least ~10ms as `delay_for` has millisecond granularity.
    if sleep_duration < ten_milliseconds {
        tokens_per_duration = (ten_milliseconds.as_nanos() / sleep_duration.as_nanos()) as u32;
        sleep_duration *= tokens_per_duration;
    } else {
        tokens_per_duration = 1;
    }

    info!(
        "throttle allowing {} request(s) every {:?}",
        tokens_per_duration, sleep_duration
    );

    // Loop and remove tokens from channel at controlled rate until load test ends.
    loop {
        debug!(
            "throttle removing {} token(s) from channel",
            tokens_per_duration
        );
        time::delay_for(sleep_duration).await;

        // A message will be received when the load test is over.
        if parent_receiver.try_recv().is_ok() {
            // Close throttle channel to prevent any further requests.
            info!("load test complete, closing throttle channel");
            throttle_receiver.close();
            break;
        }

        // Remove tokens from the channel, freeing spots for request to be made.
        for token in 0..tokens_per_duration {
            // If the channel is empty, we will get an error, so stop trying to remove tokens.
            if throttle_receiver.try_recv().is_err() {
                debug!("empty channel, exit after removing {} tokens", token);
                break;
            }
        }
    }
}
