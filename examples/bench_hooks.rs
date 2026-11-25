//! Benchmark harness for the request-hooks design investigation
//! (`src/docs/goose-book/src/design/request-hooks.md`). Not part of the
//! public API, not a load test: measures Goose's own CPU time per request
//! and throughput, against a tiny local HTTP/1.1 server returning a small
//! fixed body, across the arms the design doc's "The bar" section
//! compares. Linux only (reads `/proc/self/stat` for process CPU time).
//!
//! The server runs in a **separate child process** (not a thread in this
//! process), so the client's `/proc/self/stat` reflects only the client's
//! own CPU time, never the server's.
//!
//! Run with, e.g.:
//! ```text
//! cargo run --release --example bench_hooks --features request-hooks -- b 20000
//! ```
//! Arms: a (plain `GooseUser::request`, no hook machinery reachable — only
//! meaningful built without `request-hooks`), b (feature on, no hooks), c
//! (one sync before_send hook), d (one sync on_response hook), e (five
//! hooks), f (one async before_send hook), g (one on_response hook that
//! retries once on a 503 the server returns every tenth request).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use goose::config::GooseConfiguration;
use goose::goose::GooseUser;
use url::Url;

#[cfg(feature = "request-hooks")]
use goose::goose::hooks::{
    BeforeSendOutcome, HookRequestParts, HookResponseParts, ResponseOutcome, RetryPolicy,
    TransportError,
};

const BODY: &[u8] = b"ok";

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

fn handle_connection(mut stream: TcpStream, fail_every_tenth: bool, counter: Arc<AtomicU64>) {
    stream.set_nodelay(true).ok();
    let mut read_buf = [0u8; 8192];
    let mut pending: Vec<u8> = Vec::new();
    loop {
        while find_header_end(&pending).is_none() {
            match stream.read(&mut read_buf) {
                Ok(0) => return,
                Ok(n) => pending.extend_from_slice(&read_buf[..n]),
                Err(_) => return,
            }
        }
        let header_end = find_header_end(&pending).expect("just checked Some");
        pending.drain(..header_end);

        let n = counter.fetch_add(1, Ordering::Relaxed) + 1;
        let response: Vec<u8> = if fail_every_tenth && n.is_multiple_of(10) {
            b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: keep-alive\r\n\r\n"
                .to_vec()
        } else {
            let mut r = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
                BODY.len()
            )
            .into_bytes();
            r.extend_from_slice(BODY);
            r
        };
        if stream.write_all(&response).is_err() {
            return;
        }
    }
}

/// Runs this process as the server: binds an ephemeral port, prints it as
/// the first stdout line (so the parent can read it), then serves forever,
/// a thread per connection, until killed.
fn run_as_server(fail_every_tenth: bool) -> ! {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    println!("{}", addr.port());
    std::io::stdout().flush().ok();
    let counter = Arc::new(AtomicU64::new(0));
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let counter = counter.clone();
        std::thread::spawn(move || handle_connection(stream, fail_every_tenth, counter));
    }
    unreachable!("listener.incoming() never ends");
}

/// Spawns this same binary in server mode as a child process, and waits for
/// it to report its port. Keeping the server in its own process means the
/// client's `/proc/self/stat` never includes the server's CPU time.
fn spawn_server_process(fail_every_tenth: bool) -> (Child, u16) {
    let exe = std::env::current_exe().expect("current_exe");
    let mut child = Command::new(exe)
        .arg("--server")
        .arg(if fail_every_tenth { "1" } else { "0" })
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server process");
    let stdout = child.stdout.take().expect("server stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).expect("read server port");
    let port: u16 = line.trim().parse().expect("server port");
    (child, port)
}

/// Clock ticks per second, for converting `/proc/self/stat`'s CPU fields
/// (in ticks) to seconds. Read once via `getconf`, rather than hardcoding
/// the common 100 Hz default.
fn clk_tck() -> f64 {
    let out = std::process::Command::new("getconf")
        .arg("CLK_TCK")
        .output()
        .expect("getconf CLK_TCK");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(100.0)
}

/// This process's own total (user + system) CPU time so far, in seconds —
/// never the server's, which lives in a different process.
fn process_cpu_seconds(clk_tck: f64) -> f64 {
    let stat = std::fs::read_to_string("/proc/self/stat").expect("read /proc/self/stat");
    // The `comm` field (2nd) can itself contain spaces or parentheses, so
    // split on the LAST ')' first; everything after it is space-separated,
    // starting at the `state` field (3rd overall).
    let after_comm = stat.rsplit_once(')').expect("stat has a comm field").1;
    let fields: Vec<&str> = after_comm.split_whitespace().collect();
    // fields[0] is `state` (field 3); utime is field 14 (index 11 here),
    // stime is field 15 (index 12 here).
    let utime: f64 = fields[11].parse().expect("utime");
    let stime: f64 = fields[12].parse().expect("stime");
    (utime + stime) / clk_tck
}

fn base_config() -> GooseConfiguration {
    GooseConfiguration::parse_args_default(&[] as &[&str]).expect("default configuration")
}

#[cfg(feature = "request-hooks")]
fn add_header_hook(request: &mut HookRequestParts, _user: &mut GooseUser) -> BeforeSendOutcome {
    request
        .headers_mut()
        .insert("x-bench-trace", "1".parse().unwrap());
    BeforeSendOutcome::Continue
}

#[cfg(feature = "request-hooks")]
fn accept_hook(
    _response: Result<&HookResponseParts, &TransportError>,
    _user: &mut GooseUser,
) -> ResponseOutcome {
    ResponseOutcome::Accept
}

#[cfg(feature = "request-hooks")]
fn retry_once_on_503(
    response: Result<&HookResponseParts, &TransportError>,
    _user: &mut GooseUser,
) -> ResponseOutcome {
    match response {
        Ok(r) if r.status().as_u16() == 503 => ResponseOutcome::Retry {
            hide_this_attempt: true,
        },
        _ => ResponseOutcome::Accept,
    }
}

fn build_user(base_url: Url, arm: &str) -> GooseUser {
    let config = base_config();
    #[allow(unused_mut)]
    let mut user = GooseUser::single(base_url, &config).expect("build user");
    #[cfg(feature = "request-hooks")]
    {
        user = match arm {
            "b" | "a" => user,
            "c" => user.with_before_send_hook(add_header_hook),
            "d" => user.with_on_response_hook(accept_hook, RetryPolicy::default()),
            "e" => user
                .with_before_send_hook(add_header_hook)
                .with_before_send_hook(add_header_hook)
                .with_on_response_hook(accept_hook, RetryPolicy::default())
                .with_on_response_hook(accept_hook, RetryPolicy::default())
                .with_on_response_hook(accept_hook, RetryPolicy::default()),
            "f" => user.with_before_send_hook_async(add_header_hook_async),
            "g" => user.with_on_response_hook(retry_once_on_503, RetryPolicy::default()),
            other => panic!("unknown arm {}", other),
        };
    }
    #[cfg(not(feature = "request-hooks"))]
    let _ = arm;
    user
}

#[cfg(feature = "request-hooks")]
fn add_header_hook_async(
    mut headers: http::HeaderMap,
) -> futures::future::BoxFuture<'static, (http::HeaderMap, BeforeSendOutcome)> {
    headers.insert("x-bench-trace", "1".parse().unwrap());
    Box::pin(async move { (headers, BeforeSendOutcome::Continue) })
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Invoked as the server (a child process of a client run below).
    if args.get(1).map(String::as_str) == Some("--server") {
        let fail_every_tenth = args.get(2).map(String::as_str) == Some("1");
        run_as_server(fail_every_tenth);
    }

    let arm = args.get(1).cloned().unwrap_or_else(|| "a".to_string());
    let iterations: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20_000);

    let fail_every_tenth = arm == "g";
    let (mut server, port) = spawn_server_process(fail_every_tenth);

    let base_url = Url::parse(&format!("http://127.0.0.1:{port}")).expect("url");
    let mut user = build_user(base_url, &arm);

    // Warm up the connection pool and let any one-time setup cost (DNS,
    // first connect) happen outside the measured window.
    for _ in 0..50 {
        let _ = user.get("/").await;
    }

    let clk_tck = clk_tck();
    let cpu_before = process_cpu_seconds(clk_tck);
    let wall_before = Instant::now();

    for _ in 0..iterations {
        let _ = user.get("/").await;
    }

    let wall_elapsed = wall_before.elapsed();
    let cpu_elapsed = process_cpu_seconds(clk_tck) - cpu_before;

    let cpu_per_request_us = (cpu_elapsed / iterations as f64) * 1_000_000.0;
    let rps = iterations as f64 / wall_elapsed.as_secs_f64();

    println!(
        "arm={} iterations={} cpu_total_s={:.4} cpu_per_request_us={:.3} wall_s={:.4} rps={:.1}",
        arm,
        iterations,
        cpu_elapsed,
        cpu_per_request_us,
        wall_elapsed.as_secs_f64(),
        rps
    );

    let _ = server.kill();
    let _ = server.wait();
}
