//! Goose load test example for a server sent events (SSE) endpoint.
//!
//! Each user opens `GET /events` as an event stream, reads events until it has seen
//! `EVENTS_PER_CONNECTION` of them, then closes the connection and opens another.
//! The `events` request is timed to the response headers, as every Goose request is;
//! the time from sending the request to the last of those events is recorded as a
//! second request, `SSE events (stream)`.
//!
//! An event stream never ends, so the body is taken from Goose with `into_inner()`:
//! Goose reads to the end any body a transaction drops unread, which on a stream
//! would never finish.
//!
//! ```text
//! cargo run --example sse -- --host http://localhost:8080 --users 10 --run-time 1m
//! ```
//!
//! ## License
//!
//! Copyright 2020-2026 Jeremy Andrews
//!
//! Licensed under the Apache License, Version 2.0 (the "License");
//! you may not use this file except in compliance with the License.
//! You may obtain a copy of the License at
//!
//! <http://www.apache.org/licenses/LICENSE-2.0>
//!
//! Unless required by applicable law or agreed to in writing, software
//! distributed under the License is distributed on an "AS IS" BASIS,
//! WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//! See the License for the specific language governing permissions and
//! limitations under the License.

use goose::prelude::*;
use std::time::{Duration, Instant};

/// The path of the event stream.
const EVENTS_PATH: &str = "/events";
/// How many events each user reads from a connection before closing it.
const EVENTS_PER_CONNECTION: usize = 10;
/// How long a user waits for the next event before failing the request.
const EVENT_TIMEOUT: Duration = Duration::from_secs(30);
/// The most bytes a user holds of an event not yet complete, so a server that never
/// ends an event cannot grow the buffer without bound.
const MAX_EVENT_BYTES: usize = 64 * 1024;
/// The UTF-8 byte order mark a stream may start with.
const BOM: &[u8] = b"\xEF\xBB\xBF";

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    GooseAttack::initialize()?
        .register_scenario(sse_scenario())
        .execute()
        .await?;

    Ok(())
}

/// One scenario, with one transaction that runs repeatedly as long as the load test
/// is running. There is no wait time: a user opens its next stream as soon as it has
/// closed the last one.
fn sse_scenario() -> Scenario {
    scenario!("SseUser").register_transaction(transaction!(read_events).set_name("events"))
}

/// Opens the event stream, checks it is one, and reads `EVENTS_PER_CONNECTION` events.
async fn read_events(user: &mut GooseUser) -> TransactionResult {
    let started = Instant::now();

    // Ask for an event stream, uncompressed: Goose asks for gzip unless the request
    // sets its own Accept-Encoding, and a body taken with `into_inner()` is never
    // decompressed.
    let request_builder = user
        .get_request_builder(&GooseMethod::Get, EVENTS_PATH)?
        .header("Accept", "text/event-stream")
        .header("Accept-Encoding", "identity");
    let goose_request = GooseRequest::builder()
        .set_request_builder(request_builder)
        .name("events")
        .build();
    let mut goose = user.request(goose_request).await?;

    // Take the body out of Goose's hands, so dropping it closes the connection. A
    // request that got no response, which Goose recorded as failed, fails the
    // transaction too.
    let response = goose.response?.into_inner();

    if response.status() != 200 {
        return user.set_failure("status not 200", &mut goose.request, None, None);
    }
    let content_type = response
        .headers()
        .get("Content-Type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if !is_event_stream(content_type) {
        return user.set_failure("not text/event-stream", &mut goose.request, None, None);
    }
    if response.headers().contains_key("Content-Encoding") {
        return user.set_failure("stream is encoded", &mut goose.request, None, None);
    }

    // Read the events, then drop the response, which closes the connection.
    let result = read_stream(response).await;
    user.record_custom_request_duration(
        "SSE",
        "events (stream)",
        started.elapsed(),
        result.is_ok(),
        Some(200),
        result.err(),
    )
    .await?;
    if let Err(tag) = result {
        return user.set_failure(tag, &mut goose.request, None, None);
    }

    Ok(())
}

/// Whether a `Content-Type` is `text/event-stream`, in any case, with or without
/// parameters such as `charset=utf-8`.
fn is_event_stream(content_type: &str) -> bool {
    let media_type = content_type.split(';').next().unwrap_or("");
    media_type.trim().eq_ignore_ascii_case("text/event-stream")
}

/// Reads the stream until it has seen `EVENTS_PER_CONNECTION` events, or returns why
/// it could not. Only counts and errors are reported: events can carry tokens or user
/// data, so their contents are never logged.
async fn read_stream(mut response: reqwest::Response) -> Result<(), &'static str> {
    // The bytes of the event not yet complete; each event is drained once it is.
    let mut buffer: Vec<u8> = Vec::with_capacity(4096);
    // How far `buffer` has been searched for the end of an event.
    let mut scanned = 0;
    let mut events = 0;
    let mut last_event = Instant::now();
    // Whether a byte order mark at the start of the stream may still be to come.
    let mut at_start = true;

    while events < EVENTS_PER_CONNECTION {
        // The timeout runs from the last event, so a server that sends only keep
        // alives still times out.
        let wait = EVENT_TIMEOUT.saturating_sub(last_event.elapsed());
        let chunk = match tokio::time::timeout(wait, response.chunk()).await {
            Ok(Ok(Some(chunk))) => chunk,
            Ok(Ok(None)) => return Err("stream ended early"),
            Ok(Err(_)) => return Err("stream read error"),
            Err(_) => return Err("timed out waiting for an event"),
        };
        buffer.extend_from_slice(&chunk);

        // A stream may start with a UTF-8 byte order mark, which is not part of the
        // first event's first line.
        if at_start {
            if buffer.len() < BOM.len() && BOM.starts_with(&buffer) {
                continue;
            }
            if buffer.starts_with(BOM) {
                buffer.drain(..BOM.len());
            }
            at_start = false;
        }

        let split = split_events(&buffer, scanned);
        if split.events > 0 {
            events += split.events;
            last_event = Instant::now();
        }
        buffer.drain(..split.consumed);
        scanned = split.scanned - split.consumed;

        // Once enough events have arrived, what follows them does not matter.
        if events < EVENTS_PER_CONNECTION && buffer.len() > MAX_EVENT_BYTES {
            return Err("no event end within MAX_EVENT_BYTES");
        }
    }

    Ok(())
}

/// What `split_events` found.
#[derive(Debug, PartialEq)]
struct Split {
    /// Complete events with at least one `data` line.
    events: usize,
    /// Bytes from the start of the buffer to the end of its last complete event,
    /// counted or not: these can be dropped.
    consumed: usize,
    /// How far the buffer was searched; the next search starts here.
    scanned: usize,
}

/// Finds the complete events in `buffer`, searching from `from`, a `scanned` that an
/// earlier call returned for the same bytes (0 the first time).
///
/// An event ends at a blank line: a line ending right after another line ending,
/// where a line ends with `\r\n`, `\n` or `\r`, so `\n\n`, `\r\n\r\n` and `\r\r` among
/// others. An event counts if it has a `data` field; one of comments only (lines that
/// start with `:`), such as a keep alive, does not.
fn split_events(buffer: &[u8], from: usize) -> Split {
    let mut split = Split {
        events: 0,
        consumed: 0,
        scanned: from,
    };
    let mut i = from;
    while i < buffer.len() {
        let first = match line_ending(buffer, i) {
            Some(Some(length)) => length,
            // Not a line ending: keep searching.
            Some(None) => {
                i += 1;
                continue;
            }
            // A `\r` at the end, which may be the start of `\r\n`: wait for more.
            None => break,
        };
        let next = i + first;
        if next == buffer.len() {
            break;
        }
        let second = match line_ending(buffer, next) {
            Some(Some(length)) => length,
            Some(None) => {
                i = next;
                continue;
            }
            // A `\r` at the end ends the blank line whatever comes next; a `\n` after
            // it is an empty line before the next event, which does no harm.
            None => 1,
        };
        // A blank line: the event ends after it.
        let end = next + second;
        if has_data(&buffer[split.consumed..end]) {
            split.events += 1;
        }
        split.consumed = end;
        i = end;
    }
    split.scanned = i.max(split.consumed);
    split
}

/// The length of the line ending at `buffer[i]`: `Some(None)` if there is none, and
/// `None` if it is a `\r` with nothing after it yet.
fn line_ending(buffer: &[u8], i: usize) -> Option<Option<usize>> {
    match buffer[i] {
        b'\n' => Some(Some(1)),
        b'\r' => match buffer.get(i + 1) {
            Some(b'\n') => Some(Some(2)),
            Some(_) => Some(Some(1)),
            None => None,
        },
        _ => Some(None),
    }
}

/// Whether an event has a `data` field: a line `data`, or one that starts `data:`.
fn has_data(event: &[u8]) -> bool {
    event
        .split(|byte| *byte == b'\n' || *byte == b'\r')
        .any(|line| line == b"data" || line.starts_with(b"data:"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use goose::config::GooseConfiguration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    /// Splits `chunks` as `read_stream` does, returning the events counted and the
    /// bytes left in the buffer.
    fn split_chunks(chunks: &[&[u8]]) -> (usize, Vec<u8>) {
        let mut buffer = Vec::new();
        let mut scanned = 0;
        let mut events = 0;
        for chunk in chunks {
            buffer.extend_from_slice(chunk);
            let split = split_events(&buffer, scanned);
            events += split.events;
            buffer.drain(..split.consumed);
            scanned = split.scanned - split.consumed;
        }
        (events, buffer)
    }

    #[test]
    fn splits_on_lf() {
        assert_eq!(split_chunks(&[b"data: a\n\ndata: b\n\n"]), (2, vec![]));
        assert_eq!(
            split_events(b"data: a\n\ndata: b", 0),
            Split {
                events: 1,
                consumed: 9,
                scanned: 16
            }
        );
    }

    #[test]
    fn splits_on_crlf() {
        assert_eq!(
            split_chunks(&[b"data: a\r\n\r\ndata: b\r\n\r\n"]),
            (2, vec![])
        );
    }

    #[test]
    fn splits_on_cr() {
        assert_eq!(split_chunks(&[b"data: a\r\rdata: b\r\r"]), (2, vec![]));
    }

    #[test]
    fn splits_mixed_line_endings() {
        assert_eq!(split_chunks(&[b"data: a\r\n\ndata: b\n\r\n"]), (2, vec![]));
    }

    #[test]
    fn event_split_across_chunks() {
        assert_eq!(split_chunks(&[b"data: hel", b"lo\n", b"\n"]), (1, vec![]));
        // Split inside `\r\n\r\n`: the event ends at the second `\r`, and the `\n`
        // after it is left as an empty line.
        assert_eq!(
            split_chunks(&[b"data: a\r", b"\n\r", b"\ndata"]),
            (1, b"\ndata".to_vec())
        );
        // `\r\r` ends an event even if a `\n` follows in the next chunk.
        assert_eq!(
            split_chunks(&[b"data: a\r\r", b"\ndata: b\n\n"]),
            (2, vec![])
        );
        // An incomplete event stays in the buffer.
        assert_eq!(
            split_chunks(&[b"data: a\n", b"data: b"]),
            (0, b"data: a\ndata: b".to_vec())
        );
    }

    #[test]
    fn comment_only_block_is_not_an_event() {
        assert_eq!(split_chunks(&[b": keep alive\n\n"]), (0, vec![]));
        assert_eq!(split_chunks(&[b":\n: x\n\ndata: a\n\n"]), (1, vec![]));
        // Other fields alone are not an event either.
        assert_eq!(split_chunks(&[b"event: x\nid: 1\n\n"]), (0, vec![]));
    }

    #[test]
    fn multi_line_data_is_one_event() {
        assert_eq!(
            split_chunks(&[b"event: update\ndata: one\ndata: two\ndata\nid: 7\n\n"]),
            (1, vec![])
        );
    }

    #[test]
    fn scans_only_new_bytes() {
        let split = split_events(b"data: abc", 0);
        assert_eq!(split.scanned, 9);
        // A `\r` at the end is not searched past until the next byte arrives.
        assert_eq!(split_events(b"data: a\r", 0).scanned, 7);
        assert_eq!(split_events(b"data: a\n", 0).scanned, 7);
    }

    #[test]
    fn checks_the_media_type() {
        assert!(is_event_stream("text/event-stream"));
        assert!(is_event_stream("Text/Event-Stream"));
        assert!(is_event_stream("text/event-stream; charset=utf-8"));
        assert!(is_event_stream(" TEXT/EVENT-STREAM ;charset=utf-8"));
        assert!(!is_event_stream("text/event-streaming"));
        assert!(!is_event_stream("text/plain"));
        assert!(!is_event_stream(""));
    }

    /// A response whose whole body arrives as one chunk, then ends.
    fn one_chunk(body: Vec<u8>) -> reqwest::Response {
        reqwest::Response::from(http::Response::new(body))
    }

    /// `count` events of one `data` line each.
    fn events_bytes(count: usize) -> Vec<u8> {
        (0..count)
            .flat_map(|n| format!("data: {n}\n\n").into_bytes())
            .collect()
    }

    #[tokio::test]
    async fn reads_enough_events_followed_by_a_large_unfinished_one() {
        let mut body = events_bytes(EVENTS_PER_CONNECTION);
        body.extend(vec![b'a'; MAX_EVENT_BYTES + 1]);
        assert_eq!(read_stream(one_chunk(body)).await, Ok(()));
    }

    #[tokio::test]
    async fn fails_a_large_unfinished_event_before_enough_events() {
        let mut body = events_bytes(EVENTS_PER_CONNECTION - 1);
        body.extend(vec![b'a'; MAX_EVENT_BYTES + 1]);
        assert_eq!(
            read_stream(one_chunk(body)).await,
            Err("no event end within MAX_EVENT_BYTES")
        );
    }

    #[tokio::test]
    async fn counts_the_first_event_after_a_byte_order_mark() {
        let mut body = BOM.to_vec();
        body.extend(events_bytes(EVENTS_PER_CONNECTION));
        assert_eq!(read_stream(one_chunk(body)).await, Ok(()));
        // Without enough events, the stream ends early.
        let mut body = BOM.to_vec();
        body.extend(events_bytes(EVENTS_PER_CONNECTION - 1));
        assert_eq!(
            read_stream(one_chunk(body)).await,
            Err("stream ended early")
        );
    }

    /// Serves `GET` with a `text/event-stream` response that never ends, whose body
    /// `body` writes; returns the server's address.
    async fn serve<F, Fut>(body: F) -> String
    where
        F: Fn(TcpStream) -> Fut + Send + Copy + 'static,
        Fut: std::future::Future<Output = std::io::Result<()>> + Send,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    // Read the request's headers.
                    let mut request = Vec::new();
                    let mut buf = [0u8; 1024];
                    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        match stream.read(&mut buf).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => request.extend_from_slice(&buf[..n]),
                        }
                    }
                    let headers = "HTTP/1.1 200 OK\r\n\
                        Content-Type: text/event-stream\r\n\
                        Cache-Control: no-cache\r\n\
                        Transfer-Encoding: chunked\r\n\r\n";
                    if stream.write_all(headers.as_bytes()).await.is_ok() {
                        let _ = body(stream).await;
                    }
                });
            }
        });
        format!("http://{address}")
    }

    /// Writes `bytes` as one HTTP chunk.
    async fn write_chunk(stream: &mut TcpStream, bytes: &[u8]) -> std::io::Result<()> {
        stream
            .write_all(format!("{:x}\r\n", bytes.len()).as_bytes())
            .await?;
        stream.write_all(bytes).await?;
        stream.write_all(b"\r\n").await
    }

    /// An event every 20 ms, with a keep alive comment between them.
    async fn events(mut stream: TcpStream) -> std::io::Result<()> {
        for n in 0u64.. {
            write_chunk(&mut stream, format!("id: {n}\ndata: {n}\n\n").as_bytes()).await?;
            tokio::time::sleep(Duration::from_millis(10)).await;
            write_chunk(&mut stream, b": keep alive\n\n").await?;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Ok(())
    }

    /// Bytes forever, with no blank line to end an event.
    async fn no_blank_line(mut stream: TcpStream) -> std::io::Result<()> {
        let bytes = [b'a'; 1024];
        loop {
            write_chunk(&mut stream, &bytes).await?;
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }

    /// Runs the example's scenario with 1 user for 2 seconds against `host`, failing
    /// if it takes 10 seconds or more.
    async fn run(host: &str) -> GooseMetrics {
        let configuration = GooseConfiguration::parse_args_default(&[
            "--host",
            host,
            "--users",
            "1",
            "--run-time",
            "2",
            "--no-telnet",
            "--quiet",
        ])
        .unwrap();
        let attack = GooseAttack::initialize_with_config(configuration)
            .unwrap()
            .register_scenario(sse_scenario());
        tokio::time::timeout(Duration::from_secs(10), attack.execute())
            .await
            .expect("the load test did not end within 10 seconds")
            .unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reads_an_endless_stream() {
        let host = serve(events).await;
        let metrics = run(&host).await;

        let request = metrics.requests.get("GET events").unwrap();
        assert!(request.success_count > 0);
        assert_eq!(request.fail_count, 0);
        let stream = metrics.requests.get("SSE events (stream)").unwrap();
        assert!(stream.success_count > 0);
        assert_eq!(stream.fail_count, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fails_the_transaction_when_the_connection_is_refused() {
        // A port that was free a moment ago, with nothing listening on it.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let metrics = run(&format!("http://{address}")).await;

        let request = metrics.requests.get("GET events").unwrap();
        assert!(request.fail_count > 0);
        assert_eq!(request.success_count, 0);
        let transaction = &metrics.transactions[0][0];
        assert!(transaction.fail_count > 0);
        assert_eq!(transaction.success_count, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fails_a_stream_with_no_blank_line() {
        let host = serve(no_blank_line).await;
        let metrics = run(&host).await;

        let request = metrics.requests.get("GET events").unwrap();
        assert!(request.fail_count > 0);
        assert_eq!(request.success_count, 0);
    }
}
