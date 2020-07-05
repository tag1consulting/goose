use async_std::sync::Sender;
use tokio::time;

use crate::GooseConfiguration;

/// This throttle thread limits the maximum number of requests that can be made across
/// all GooseUser threads. It uses a async-std channel as this allows single-sender
/// max-receiver communication. When enabled, GooseUser threads must grab a token
/// from the channel before making a request, and this thread limits how many tokens
/// are available per second.
pub async fn throttle_main(configuration: GooseConfiguration, throttle_sender: Sender<bool>) {
    let sleep_duration =
        time::Duration::from_secs_f32(1.0 / configuration.throttle_requests.unwrap() as f32);
    info!("throttle allowing 1 request every {:?}", sleep_duration);

    // Loop until parent thread exits, adding one element
    loop {
        // @TODO: if `sleep_duration` is <10ms, add multiple tokens at once as `delay_for` has
        // millisecond granularity.
        time::delay_for(sleep_duration).await;
        // @TODO: when adding auto-tuning suggestions this should be converted to `try_send`.
        debug!("parent throttle sending value");
        // @TODO: if load test is ending, fill queue with `false` so all GooseUser threads exit.
        let _ = throttle_sender.send(true).await;
    }
}
