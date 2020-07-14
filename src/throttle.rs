use tokio::sync::mpsc::Receiver;
use tokio::time;

use crate::GooseConfiguration;

/// This throttle thread limits the maximum number of requests that can be made across
/// all GooseUser threads. When enabled, GooseUser threads must add a token to the
/// bounded channel before making a request, and this thread limits how frequently
/// tokens are removed thereby throttling how fast requests can be made. It is a variation
/// on the leaky bucket algorithm: instead of leaking the overflow we asynchronously block.
pub async fn throttle_main(
    configuration: GooseConfiguration,
    mut throttle_receiver: Receiver<bool>,
    mut parent_receiver: Receiver<bool>,
) {
    let sleep_duration =
        time::Duration::from_secs_f32(1.0 / configuration.throttle_requests.unwrap() as f32);
    info!("throttle allowing 1 request every {:?}", sleep_duration);

    // Loop until parent thread exits, adding one element
    loop {
        debug!("throttle removing token from channel");
        // @TODO: if `sleep_duration` is <10ms, remove multiple tokens at once as `delay_for` has
        // millisecond granularity.
        time::delay_for(sleep_duration).await;

        // Check if parent has informed us the load test is over.
        if parent_receiver.try_recv().is_ok() {
            info!("load test complete, closing throttle channel");
            throttle_receiver.close();
            break;
        }

        // Remove a token from the channel, freeing a spot for a request to be made.
        let _ = throttle_receiver.try_recv();
    }
}
