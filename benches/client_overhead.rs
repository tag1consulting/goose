//! CPU time per request of Goose, reqwest and bare hyper against a local server.
//!
//! The Goose Book's "Raw Hyper Client" page explains the numbers this prints.
//!
//! ```text
//! # Every mode completes requests, and rejects a body of the same length but
//! # other bytes, a shorter body, and requests that fail (add --tls for HTTPS):
//! cargo bench --bench client_overhead --features raw-hyper -- self-check
//! # Every client mode, 5 runs each, interleaved, with a median and spread:
//! cargo bench --bench client_overhead --features raw-hyper -- suite
//! # Options, with their defaults:
//! cargo bench --bench client_overhead --features raw-hyper -- suite --users 16 \
//!     --workers 4 --server-threads 8 --warmup 2 --seconds 10 --runs 5 \
//!     [--modes a,b,...] [--pin] [--tls]
//! ```
//!
//! Without `--features raw-hyper` the `goose-raw` modes are left out of the
//! default list, and fail when named.
//!
//! The server, a hyper HTTP/1.1 server answering every request with the same
//! 13 byte body, runs in a process of its own, and each run of a client mode in a
//! fresh process, so the CPU time a client process spends (user plus system, from
//! `getrusage`) between the end of its warm up and the end of its measured
//! seconds is the client's alone. Each user is a task of its own with one
//! keep-alive connection, sending its next request as soon as it has read the
//! last response: a closed loop, as fast as the client goes.
//!
//! Client modes:
//!
//! - `goose`: a Goose load test, each user's transaction `user.get("/")` and
//!   its body read to the end, as `discard_body()` reads it, and compared with
//!   the body the server sends.
//! - `goose-tuned`: the same, each user's client replaced at start with the tuned
//!   reqwest client below, through `GooseUser::set_client_builder`.
//! - `goose-raw`: the same, each request sent with `user.raw_get("/")` over the
//!   raw hyper client; build with `--features raw-hyper`.
//! - `goose-post`, `goose-raw-post`: `user.post("/", body)` and
//!   `user.raw_post("/", body)` of a 32 byte body.
//! - `+headers` after a Goose mode: each GET built on a reqwest `RequestBuilder`
//!   with two headers of the test's own (`GooseRequestBuilder::raw` for
//!   `goose-raw`).
//! - `+cookie` after a Goose mode: each user first requests `/login` over reqwest,
//!   which sets a cookie, so every request after it carries that cookie.
//! - `reqwest`: a bare loop over a reqwest client per user, built as Goose builds
//!   each user's client (user agent, 60 s timeout, cookie store, Goose's own
//!   `Accept-Encoding: gzip`, reqwest's own decompression off).
//! - `reqwest+<knob>+...`: the same with reqwest settings Goose does not use:
//!   `no-cookies`, `no-redirect`, `no-retry`, `no-proxy`, `no-referer`,
//!   `no-timeout`. `reqwest-tuned` is all of them but `no-timeout`.
//! - `hyper`: a bare loop over a `hyper_util` legacy client per user (its pool
//!   holds the user's one connection): no cookies, redirects, decompression,
//!   proxies or timeout.
//! - `hyper-conn`: a bare loop over one `hyper::client::conn::http1` connection
//!   per user, with no pool.
//! - `+timeout` after `hyper` or `hyper-conn`: each request under a 60 s
//!   `tokio::time::timeout`, as reqwest's total timeout is.
//!
//! `--pin` runs the server and each client under `taskset` with fixed cores,
//! on Linux; macOS has no way to pin a thread to a core.
//!
//! `--tls` serves HTTPS, with the test certificate in `tests/fixtures/tls`,
//! offering HTTP/1.1 by ALPN, for the Goose modes only; each runs with
//! `--accept-invalid-certs`, as the certificate's CA is not trusted. The
//! handshake happens once per user; what is measured is each request's cost
//! over an open TLS connection.

#[cfg(not(unix))]
fn main() {
    eprintln!("client_overhead reads CPU time with getrusage, so runs on Unix only");
}

#[cfg(unix)]
fn main() {
    bench::main();
}

#[cfg(unix)]
mod bench {
    use bytes::Bytes;
    use goose::config::GooseConfiguration;
    use goose::prelude::*;
    use http_body_util::{BodyExt, Empty, Full};
    use hyper::header::{HeaderValue, ACCEPT_ENCODING, HOST, USER_AGENT};
    use hyper::service::service_fn;
    use hyper::{Request, Response, Uri};
    use hyper_util::client::legacy::Client as LegacyClient;
    use hyper_util::rt::{TokioExecutor, TokioIo};
    use std::collections::BTreeMap;
    use std::convert::Infallible;
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    /// What the server answers every request with.
    const BODY: &[u8] = b"Hello, world!";
    /// What the POST modes send.
    const POST_BODY: &str = "name=goose&value=0123456789abcdef";
    /// The user agent Goose sends.
    const GOOSE_USER_AGENT: &str = concat!("goose/", env!("CARGO_PKG_VERSION"));
    /// Goose's default request timeout.
    const TIMEOUT: Duration = Duration::from_secs(60);

    /// Requests completed by every user, counted as each completes.
    static COMPLETED: AtomicU64 = AtomicU64::new(0);
    /// Requests a Goose mode saw fail, or return a body other than `BODY`; any
    /// fails the run. A bare loop panics on such a request instead.
    static FAILED: AtomicU64 = AtomicU64::new(0);
    /// Set with `--tls`, for a client a Goose mode builds itself.
    static ACCEPT_INVALID_CERTS: AtomicBool = AtomicBool::new(false);
    /// Set when the measured seconds are over, to stop a bare loop.
    static STOP: AtomicBool = AtomicBool::new(false);

    /// The options every subcommand takes, with their defaults.
    #[derive(Clone, Debug)]
    struct Options {
        users: usize,
        workers: usize,
        server_threads: usize,
        warmup: u64,
        seconds: u64,
        runs: usize,
        modes: Vec<String>,
        addr: String,
        pin: bool,
        body: Option<String>,
        tls: bool,
        /// Whether `--modes` was given; `self-check` checks every mode otherwise.
        modes_given: bool,
    }

    impl Options {
        fn parse(args: &[String]) -> Options {
            let mut options = Options {
                users: 16,
                workers: 4,
                server_threads: 8,
                warmup: 2,
                seconds: 10,
                runs: 5,
                modes: [
                    "goose",
                    "goose-tuned",
                    #[cfg(feature = "raw-hyper")]
                    "goose-raw",
                    "reqwest",
                    "reqwest-tuned",
                    "reqwest-tuned+no-timeout",
                    "hyper",
                    "hyper+timeout",
                    "hyper-conn",
                    "hyper-conn+timeout",
                ]
                .iter()
                .map(|mode| mode.to_string())
                .collect(),
                addr: String::new(),
                pin: false,
                body: None,
                tls: false,
                modes_given: false,
            };
            let mut args = args.iter();
            while let Some(arg) = args.next() {
                let mut value = || {
                    args.next()
                        .unwrap_or_else(|| panic!("{} needs a value", arg))
                        .clone()
                };
                match arg.as_str() {
                    "--users" => options.users = value().parse().expect("--users"),
                    "--workers" => options.workers = value().parse().expect("--workers"),
                    "--server-threads" => {
                        options.server_threads = value().parse().expect("--server-threads")
                    }
                    "--warmup" => options.warmup = value().parse().expect("--warmup"),
                    "--seconds" => options.seconds = value().parse().expect("--seconds"),
                    "--runs" => options.runs = value().parse().expect("--runs"),
                    "--modes" => {
                        options.modes = value().split(',').map(str::to_string).collect();
                        options.modes_given = true;
                    }
                    "--addr" => options.addr = value(),
                    "--pin" => options.pin = true,
                    "--body" => options.body = Some(value()),
                    "--tls" => options.tls = true,
                    // cargo bench passes --bench to every bench target.
                    "--bench" => {}
                    other => panic!("unknown option {}", other),
                }
            }
            options
        }

        /// This process's options as arguments for a client process.
        fn client_args(&self) -> Vec<String> {
            let mut args = vec![
                "--addr".into(),
                self.addr.clone(),
                "--users".into(),
                self.users.to_string(),
                "--workers".into(),
                self.workers.to_string(),
                "--warmup".into(),
                self.warmup.to_string(),
                "--seconds".into(),
                self.seconds.to_string(),
            ];
            if self.tls {
                args.push("--tls".into());
            }
            args
        }

        /// The server's arguments, as this process's options ask for it.
        fn server_args(&self) -> Vec<String> {
            let mut args = vec![
                "server".into(),
                "--server-threads".into(),
                self.server_threads.to_string(),
            ];
            if self.tls {
                args.push("--tls".into());
            }
            args
        }
    }

    pub(super) fn main() {
        let args: Vec<String> = std::env::args().skip(1).collect();
        match args.first().map(String::as_str) {
            Some("server") => server(Options::parse(&args[1..])),
            Some("run") => {
                let mode = args.get(1).expect("run needs a mode").clone();
                run(&mode, Options::parse(&args[2..]));
            }
            Some("suite") => suite(Options::parse(&args[1..])),
            Some("self-check") => self_check(Options::parse(&args[1..])),
            Some("cookie-cost") => cookie_cost(),
            // `cargo bench` with no arguments runs every bench target, this one
            // with only --bench: a short suite, so it ends in a reasonable time.
            Some("--bench") | None => suite(Options::parse(&[
                "--runs".into(),
                "1".into(),
                "--seconds".into(),
                "3".into(),
                "--warmup".into(),
                "1".into(),
            ])),
            Some(other) => panic!(
                "unknown subcommand {}: use suite, self-check, cookie-cost, server or run",
                other
            ),
        }
    }

    /// CPU time, user plus system, this process has used.
    fn cpu_time() -> Duration {
        // SAFETY: getrusage writes the struct it is given, which is valid.
        let usage = unsafe {
            let mut usage: libc::rusage = std::mem::zeroed();
            assert_eq!(libc::getrusage(libc::RUSAGE_SELF, &mut usage), 0);
            usage
        };
        let micros = |tv: libc::timeval| tv.tv_sec as u64 * 1_000_000 + tv.tv_usec as u64;
        Duration::from_micros(micros(usage.ru_utime) + micros(usage.ru_stime))
    }

    /// A multi-threaded tokio runtime with `workers` worker threads.
    fn runtime(workers: usize) -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(workers)
            .enable_all()
            .build()
            .expect("tokio runtime")
    }

    /// The server's TLS config, with `--tls`: the test certificate for
    /// `localhost`, offering HTTP/1.1 by ALPN.
    fn tls_acceptor() -> tokio_rustls::TlsAcceptor {
        use rustls::pki_types::pem::PemObject;
        use rustls::pki_types::{CertificateDer, PrivateKeyDer};
        let certs =
            CertificateDer::pem_slice_iter(include_bytes!("../tests/fixtures/tls/localhost.pem"))
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
        let key =
            PrivateKeyDer::from_pem_slice(include_bytes!("../tests/fixtures/tls/localhost.key"))
                .unwrap();
        let mut config = rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .unwrap();
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(config))
    }

    /// Serves `BODY`, or the `--body` given, over HTTP/1.1 with keep-alive, plain
    /// or with `--tls` over TLS, and prints the address it listens on. It reads
    /// each request's body before it answers, and sets a cookie on `/login`.
    fn server(options: Options) {
        let body: &'static [u8] = match options.body.clone() {
            Some(body) => Box::leak(body.into_bytes().into_boxed_slice()),
            None => BODY,
        };
        runtime(options.server_threads).block_on(async {
            let addr = if options.addr.is_empty() {
                "127.0.0.1:0"
            } else {
                &options.addr
            };
            let listener = tokio::net::TcpListener::bind(addr).await.expect("bind");
            println!("LISTENING {}", listener.local_addr().unwrap());
            let acceptor = if options.tls {
                Some(tls_acceptor())
            } else {
                None
            };
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(accepted) => accepted,
                    Err(_) => continue,
                };
                let _ = stream.set_nodelay(true);
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    let service =
                        service_fn(move |request: Request<hyper::body::Incoming>| async move {
                            let login = request.uri().path() == "/login";
                            let _ = request.into_body().collect().await;
                            let mut response = Response::new(Full::new(Bytes::from_static(body)));
                            if login {
                                response.headers_mut().insert(
                                    hyper::header::SET_COOKIE,
                                    HeaderValue::from_static("session=bench; Path=/"),
                                );
                            }
                            Ok::<_, Infallible>(response)
                        });
                    let builder = hyper::server::conn::http1::Builder::new();
                    match acceptor {
                        Some(acceptor) => {
                            if let Ok(stream) = acceptor.accept(stream).await {
                                let _ = builder
                                    .serve_connection(TokioIo::new(stream), service)
                                    .await;
                            }
                        }
                        None => {
                            let _ = builder
                                .serve_connection(TokioIo::new(stream), service)
                                .await;
                        }
                    }
                });
            }
        });
    }

    /// Runs `mode` for the warm up and measured seconds, and prints what it
    /// measured: `RESULT <mode> <requests> <cpu µs> <wall µs>`.
    fn run(mode: &str, options: Options) {
        let mut parts = mode.split('+');
        let base = parts.next().unwrap().to_string();
        let mut knobs: Vec<String> = parts.map(str::to_string).collect();
        if base == "reqwest-tuned" {
            knobs.extend(
                [
                    "no-cookies",
                    "no-redirect",
                    "no-retry",
                    "no-proxy",
                    "no-referer",
                ]
                .iter()
                .map(|knob| knob.to_string()),
            );
        }
        let base_url = format!("http://{}/", options.addr);

        // Measures from a thread of its own, so the clients' tasks never wait on it.
        let warmup = Duration::from_secs(options.warmup);
        let seconds = Duration::from_secs(options.seconds);
        let mode_name = mode.to_string();
        let measure = std::thread::spawn(move || {
            std::thread::sleep(warmup);
            let (cpu_start, requests_start, wall_start) =
                (cpu_time(), COMPLETED.load(Ordering::SeqCst), Instant::now());
            std::thread::sleep(seconds);
            let (cpu_end, requests_end, wall) = (
                cpu_time(),
                COMPLETED.load(Ordering::SeqCst),
                wall_start.elapsed(),
            );
            STOP.store(true, Ordering::SeqCst);
            println!(
                "RESULT {} {} {} {}",
                mode_name,
                requests_end - requests_start,
                (cpu_end - cpu_start).as_micros(),
                wall.as_micros()
            );
        });

        let rt = runtime(options.workers);
        if options.tls && !base.starts_with("goose") {
            panic!("--tls measures the Goose modes only, not {}", mode);
        }
        match base.as_str() {
            "goose" | "goose-tuned" | "goose-raw" | "goose-post" | "goose-raw-post" => {
                rt.block_on(run_goose(&base, &knobs, &options))
            }
            "reqwest" | "reqwest-tuned" => rt.block_on(users(options.users, || {
                let client = reqwest_client(&knobs);
                let url = reqwest::Url::parse(&base_url).unwrap();
                async move {
                    while !STOP.load(Ordering::Relaxed) {
                        let response = client
                            .get(url.clone())
                            .header(ACCEPT_ENCODING, HeaderValue::from_static("gzip"))
                            .send()
                            .await
                            .expect("reqwest request");
                        let body = response.bytes().await.expect("reqwest body");
                        assert_eq!(&body[..], BODY);
                        COMPLETED.fetch_add(1, Ordering::Relaxed);
                    }
                }
            })),
            "hyper" => {
                let timeout = knobs.iter().any(|knob| knob == "timeout");
                rt.block_on(users(options.users, || {
                    let client =
                        LegacyClient::builder(TokioExecutor::new()).build_http::<Empty<Bytes>>();
                    let uri: Uri = base_url.parse().unwrap();
                    async move {
                        while !STOP.load(Ordering::Relaxed) {
                            let request = Request::get(uri.clone())
                                .header(USER_AGENT, HeaderValue::from_static(GOOSE_USER_AGENT))
                                .header(ACCEPT_ENCODING, HeaderValue::from_static("gzip"))
                                .body(Empty::new())
                                .unwrap();
                            let exchange = async {
                                let response = client.request(request).await.expect("hyper");
                                response.into_body().collect().await.unwrap().to_bytes()
                            };
                            let body = if timeout {
                                tokio::time::timeout(TIMEOUT, exchange).await.unwrap()
                            } else {
                                exchange.await
                            };
                            assert_eq!(&body[..], BODY);
                            COMPLETED.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }))
            }
            "hyper-conn" => {
                let timeout = knobs.iter().any(|knob| knob == "timeout");
                let addr = options.addr.clone();
                rt.block_on(users(options.users, || {
                    let addr = addr.clone();
                    async move {
                        let stream = tokio::net::TcpStream::connect(&addr).await.unwrap();
                        stream.set_nodelay(true).unwrap();
                        let (mut sender, connection) =
                            hyper::client::conn::http1::handshake(TokioIo::new(stream))
                                .await
                                .unwrap();
                        tokio::spawn(connection);
                        let host = HeaderValue::from_str(&addr).unwrap();
                        while !STOP.load(Ordering::Relaxed) {
                            let request = Request::get("/")
                                .header(HOST, host.clone())
                                .header(USER_AGENT, HeaderValue::from_static(GOOSE_USER_AGENT))
                                .header(ACCEPT_ENCODING, HeaderValue::from_static("gzip"))
                                .body(Empty::<Bytes>::new())
                                .unwrap();
                            let exchange = async {
                                sender.ready().await.unwrap();
                                let response = sender.send_request(request).await.unwrap();
                                response.into_body().collect().await.unwrap().to_bytes()
                            };
                            let body = if timeout {
                                tokio::time::timeout(TIMEOUT, exchange).await.unwrap()
                            } else {
                                exchange.await
                            };
                            assert_eq!(&body[..], BODY);
                            COMPLETED.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }))
            }
            other => panic!("unknown mode {}", other),
        }
        measure.join().unwrap();
        rt.shutdown_timeout(Duration::from_secs(1));
    }

    /// Spawns one task per user from `user`, and waits for all of them.
    async fn users<F, Fut>(count: usize, mut user: F)
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let tasks: Vec<_> = (0..count).map(|_| tokio::spawn(user())).collect();
        for task in tasks {
            task.await.unwrap();
        }
    }

    /// A reqwest client as Goose builds each user's, less what `knobs` turn off.
    fn reqwest_builder(knobs: &[String]) -> reqwest::ClientBuilder {
        let on = |knob: &str| !knobs.iter().any(|k| k == knob);
        let mut builder = reqwest::Client::builder()
            .user_agent(GOOSE_USER_AGENT)
            .gzip(false);
        if on("no-timeout") {
            builder = builder.timeout(TIMEOUT);
        }
        #[cfg(feature = "cookies")]
        if on("no-cookies") {
            builder = builder.cookie_store(true);
        }
        if !on("no-redirect") {
            builder = builder.redirect(reqwest::redirect::Policy::none());
        }
        if !on("no-retry") {
            builder = builder.retry(reqwest::retry::never());
        }
        if !on("no-proxy") {
            builder = builder.no_proxy();
        }
        if !on("no-referer") {
            builder = builder.referer(false);
        }
        builder
    }

    fn reqwest_client(knobs: &[String]) -> reqwest::Client {
        reqwest_builder(knobs).build().expect("reqwest client")
    }

    /// Runs a Goose load test for the warm up and measured seconds and a second more.
    /// It writes no files: no option asks for a report or a log.
    async fn run_goose(mode: &str, knobs: &[String], options: &Options) {
        let host = format!(
            "{}://{}/",
            if options.tls { "https" } else { "http" },
            options.addr
        );
        let users = options.users.to_string();
        let run_time = (options.warmup + options.seconds + 1).to_string();
        let mut args = vec![
            "--host",
            &host,
            "--users",
            &users,
            "--increase-rate",
            "100000",
            "--run-time",
            &run_time,
            "--co-mitigation",
            "disabled",
            "--quiet",
            "--no-print-metrics",
            "--no-telnet",
        ];
        if options.tls {
            args.push("--accept-invalid-certs");
            ACCEPT_INVALID_CERTS.store(true, Ordering::Relaxed);
        }
        let configuration = GooseConfiguration::parse_args_default(&args).expect("options");
        let headers = knobs.iter().any(|knob| knob == "headers");
        let transaction = match (mode, headers) {
            ("goose", false) | ("goose-tuned", false) => transaction!(get_index),
            ("goose", true) | ("goose-tuned", true) => transaction!(get_index_with_headers),
            ("goose-post", false) => transaction!(post_index),
            #[cfg(feature = "raw-hyper")]
            ("goose-raw", false) => transaction!(raw_get_index),
            #[cfg(feature = "raw-hyper")]
            ("goose-raw", true) => transaction!(raw_get_index_with_headers),
            #[cfg(feature = "raw-hyper")]
            ("goose-raw-post", false) => transaction!(raw_post_index),
            #[cfg(not(feature = "raw-hyper"))]
            ("goose-raw", _) | ("goose-raw-post", _) => {
                panic!("{} needs --features raw-hyper", mode)
            }
            _ => panic!("unknown Goose mode {}+{}", mode, knobs.join("+")),
        };
        let mut scenario = scenario!("Bench").register_transaction(transaction);
        if mode == "goose-tuned" {
            scenario = scenario.register_transaction(transaction!(tune_client).set_on_start());
        }
        if knobs.iter().any(|knob| knob == "cookie") {
            scenario = scenario.register_transaction(transaction!(login).set_on_start());
        }
        GooseAttack::initialize_with_config(configuration)
            .expect("goose")
            .register_scenario(scenario)
            .execute()
            .await
            .expect("goose run");
        let failed = FAILED.load(Ordering::SeqCst);
        if failed > 0 {
            eprintln!("{} requests failed or returned another body", failed);
            std::process::exit(1);
        }
    }

    /// Counts the request as completed only as the bare loops do: a success, with
    /// the whole body read and equal to `BODY`. Anything else, a request that got
    /// no response included, is counted as failed, which fails the run.
    async fn count(result: Result<goose::goose::GooseResponse, Box<TransactionError>>) {
        let completed = match result {
            Ok(goose) if goose.request.success => match goose.response {
                Ok(mut response) => body_is_expected(&mut response).await,
                Err(_) => false,
            },
            _ => false,
        };
        if completed {
            COMPLETED.fetch_add(1, Ordering::Relaxed);
        } else {
            FAILED.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Reads the body chunk by chunk, as `discard_body` does, and compares it with
    /// `BODY` as it goes, with no copy.
    async fn body_is_expected(response: &mut goose::goose::GooseHttpResponse) -> bool {
        let mut read = 0;
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    if BODY.get(read..read + chunk.len()) != Some(&chunk[..]) {
                        return false;
                    }
                    read += chunk.len();
                }
                Ok(None) => return read == BODY.len(),
                Err(_) => return false,
            }
        }
    }

    async fn get_index(user: &mut GooseUser) -> TransactionResult {
        count(user.get("/").await).await;
        Ok(())
    }

    /// A GET built on a reqwest `RequestBuilder`, with two headers of its own.
    fn with_headers(user: &GooseUser) -> Result<reqwest::RequestBuilder, Box<TransactionError>> {
        Ok(user
            .get_request_builder(&GooseMethod::Get, "/")?
            .header("x-bench-one", "one")
            .header("x-bench-two", "two"))
    }

    async fn get_index_with_headers(user: &mut GooseUser) -> TransactionResult {
        let request = GooseRequest::builder()
            .set_request_builder(with_headers(user)?)
            .build();
        count(user.request(request).await).await;
        Ok(())
    }

    async fn post_index(user: &mut GooseUser) -> TransactionResult {
        count(user.post("/", POST_BODY).await).await;
        Ok(())
    }

    #[cfg(feature = "raw-hyper")]
    async fn raw_get_index(user: &mut GooseUser) -> TransactionResult {
        count(user.raw_get("/").await).await;
        Ok(())
    }

    #[cfg(feature = "raw-hyper")]
    async fn raw_get_index_with_headers(user: &mut GooseUser) -> TransactionResult {
        let request = GooseRequest::builder()
            .set_request_builder(with_headers(user)?)
            .raw(true)
            .build();
        count(user.request(request).await).await;
        Ok(())
    }

    #[cfg(feature = "raw-hyper")]
    async fn raw_post_index(user: &mut GooseUser) -> TransactionResult {
        count(user.raw_post("/", POST_BODY).await).await;
        Ok(())
    }

    /// Requests `/login` over reqwest, which sets a cookie, before the user's
    /// other transactions: not counted.
    async fn login(user: &mut GooseUser) -> TransactionResult {
        let _ = user.get("/login").await?.discard_body().await;
        Ok(())
    }

    async fn tune_client(user: &mut GooseUser) -> TransactionResult {
        let knobs: Vec<String> = [
            "no-cookies",
            "no-redirect",
            "no-retry",
            "no-proxy",
            "no-referer",
        ]
        .iter()
        .map(|knob| knob.to_string())
        .collect();
        // With --tls, accept the test certificate, as Goose's own client does with
        // --accept-invalid-certs.
        let builder = reqwest_builder(&knobs)
            .danger_accept_invalid_certs(ACCEPT_INVALID_CERTS.load(Ordering::Relaxed));
        user.set_client_builder(builder).await?;
        Ok(())
    }

    /// One mode's runs: requests, CPU µs and wall µs of each.
    type Samples = Vec<(u64, u64, u64)>;

    /// Starts the server, runs every mode `runs` times, interleaved, and prints a
    /// table of the median and spread of CPU time per request.
    fn suite(mut options: Options) {
        let exe = std::env::current_exe().unwrap();
        let cores = |first: usize, count: usize| format!("{}-{}", first, first + count - 1);
        let pin = options.pin;
        let command = |cores: String| {
            if pin {
                let mut command = Command::new("taskset");
                command.arg("-c").arg(cores).arg(&exe);
                command
            } else {
                Command::new(&exe)
            }
        };
        if options.pin && !cfg!(target_os = "linux") {
            panic!("--pin uses taskset, on Linux only");
        }

        let server = command(cores(options.workers, options.server_threads))
            .args(options.server_args())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("server");
        // Stops the server when the suite ends, or panics.
        struct Server(std::process::Child);
        impl Drop for Server {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let mut server = Server(server);
        let mut line = String::new();
        BufReader::new(server.0.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        options.addr = line
            .trim()
            .strip_prefix("LISTENING ")
            .expect("server address")
            .to_string();
        eprintln!(
            "server on {}{}; {} users, {} client worker threads, {} server threads, {} s \
             warm up, {} s measured, {} runs{}",
            options.addr,
            if options.tls { " (TLS)" } else { "" },
            options.users,
            options.workers,
            options.server_threads,
            options.warmup,
            options.seconds,
            options.runs,
            if options.pin { ", pinned" } else { "" }
        );

        let mut samples: BTreeMap<String, Samples> = BTreeMap::new();
        for run in 1..=options.runs {
            for mode in &options.modes {
                let (requests, cpu, wall) =
                    run_client(command(cores(0, options.workers)), mode, &options)
                        .unwrap_or_else(|e| panic!("{}", e));
                eprintln!(
                    "run {run} {mode}: {requests} requests, {:.2} µs CPU each, {:.0}/s",
                    cpu as f64 / requests as f64,
                    requests as f64 * 1e6 / wall as f64
                );
                samples
                    .entry(mode.clone())
                    .or_default()
                    .push((requests, cpu, wall));
            }
        }
        drop(server);

        println!();
        println!(
            "| Client | CPU µs/request, median | min | max | requests/s, median | CPU cores busy |"
        );
        println!("|---|---:|---:|---:|---:|---:|");
        for mode in &options.modes {
            let runs = &samples[mode];
            let mut per_request: Vec<f64> = runs
                .iter()
                .map(|(requests, cpu, _)| *cpu as f64 / *requests as f64)
                .collect();
            let mut rate: Vec<f64> = runs
                .iter()
                .map(|(requests, _, wall)| *requests as f64 * 1e6 / *wall as f64)
                .collect();
            let mut busy: Vec<f64> = runs
                .iter()
                .map(|(_, cpu, wall)| *cpu as f64 / *wall as f64)
                .collect();
            println!(
                "| `{}` | {:.2} | {:.2} | {:.2} | {:.0} | {:.2} |",
                mode,
                median(&mut per_request),
                per_request[0],
                per_request[per_request.len() - 1],
                median(&mut rate),
                median(&mut busy),
            );
        }
    }

    /// Runs `mode` in a client process from `command`, and returns its requests,
    /// CPU µs and wall µs; an error if the process failed, as it does when a request
    /// fails or returns another body, even after it printed its measurement.
    fn run_client(
        mut command: Command,
        mode: &str,
        options: &Options,
    ) -> Result<(u64, u64, u64), String> {
        let output = command
            .arg("run")
            .arg(mode)
            .args(options.client_args())
            .stderr(Stdio::inherit())
            .output()
            .map_err(|e| format!("{} did not start: {}", mode, e))?;
        if !output.status.success() {
            return Err(format!("{} failed: {}", mode, output.status));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let result = stdout
            .lines()
            .find_map(|line| line.strip_prefix("RESULT "))
            .ok_or_else(|| format!("{} printed no result: {}", mode, stdout))?;
        let fields: Vec<u64> = result
            .split_whitespace()
            .skip(1)
            .map(|field| field.parse().unwrap())
            .collect();
        Ok((fields[0], fields[1], fields[2]))
    }

    /// Every client mode this build can run, for `self-check` with no `--modes`.
    fn every_mode(tls: bool) -> Vec<String> {
        let mut modes = vec![
            "goose",
            "goose-tuned",
            "goose+headers",
            "goose+cookie",
            "goose+headers+cookie",
            "goose-post",
        ];
        #[cfg(feature = "raw-hyper")]
        modes.extend([
            "goose-raw",
            "goose-raw+headers",
            "goose-raw+cookie",
            "goose-raw+headers+cookie",
            "goose-raw-post",
        ]);
        if !tls {
            modes.extend([
                "reqwest",
                "reqwest-tuned",
                "reqwest-tuned+no-timeout",
                "hyper",
                "hyper+timeout",
                "hyper-conn",
                "hyper-conn+timeout",
            ]);
        }
        modes.into_iter().map(str::to_string).collect()
    }

    /// Starts a server answering `body`, or `BODY`, as `options` ask, and returns
    /// it with the address it listens on.
    fn start_server(options: &Options, body: Option<&str>) -> (std::process::Child, String) {
        let exe = std::env::current_exe().unwrap();
        let mut command = Command::new(&exe);
        command.args(options.server_args());
        if let Some(body) = body {
            command.args(["--body", body]);
        }
        let mut server = command
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("server");
        let mut line = String::new();
        BufReader::new(server.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let addr = line
            .trim()
            .strip_prefix("LISTENING ")
            .expect("server address")
            .to_string();
        (server, addr)
    }

    /// Checks that every mode counts only what it should. Each mode must first run
    /// against the right server, so a mode that can't run at all fails the check
    /// here rather than passing below. Then each must reject a run against a server
    /// answering another body of `BODY`'s length, one answering a shorter body, and
    /// an address where nothing listens, so no run with a failed request, or a
    /// short or wrong body, is ever counted. With no `--modes`, it checks every mode
    /// this build can run.
    fn self_check(mut options: Options) {
        let exe = std::env::current_exe().unwrap();
        if !options.modes_given {
            options.modes = every_mode(options.tls);
        }
        options.warmup = 0;
        options.seconds = 1;

        let (mut server, addr) = start_server(&options, None);
        options.addr = addr;
        let mut broken = Vec::new();
        for mode in &options.modes {
            match run_client(Command::new(&exe), mode, &options) {
                Ok((requests, _, _)) if requests > 0 => {
                    eprintln!("runs, as it should: {mode}, {requests} requests")
                }
                Ok(_) => broken.push(format!("{mode}: no request completed")),
                Err(e) => broken.push(e),
            }
        }
        let _ = server.kill();
        let _ = server.wait();
        assert!(
            broken.is_empty(),
            "these modes failed against a server answering the right body: {:?}",
            broken
        );

        // The same length as BODY, so only a check of its bytes rejects it.
        let same_length: String = String::from_utf8(BODY.to_vec())
            .unwrap()
            .chars()
            .rev()
            .collect();
        assert_eq!(same_length.len(), BODY.len());
        let mut accepted = Vec::new();
        for (what, body) in [
            (
                "another body of the same length",
                Some(same_length.as_str()),
            ),
            ("a shorter body", Some("wrong")),
            ("no server", None),
        ] {
            let server = match body {
                Some(body) => {
                    let (server, addr) = start_server(&options, Some(body));
                    options.addr = addr;
                    Some(server)
                }
                None => {
                    // A port nothing listens on: bound, then closed.
                    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
                    options.addr = listener.local_addr().unwrap().to_string();
                    None
                }
            };
            for mode in &options.modes {
                match run_client(Command::new(&exe), mode, &options) {
                    Ok(_) => accepted.push(format!("{mode} against {what}")),
                    Err(e) => eprintln!("rejected against {what}, as it should be: {e}"),
                }
            }
            if let Some(mut server) = server {
                let _ = server.kill();
                let _ = server.wait();
            }
        }
        assert!(
            accepted.is_empty(),
            "these runs counted requests that failed or returned the wrong body: {:?}",
            accepted
        );
        println!(
            "self-check passed: every mode runs, and rejects a wrong body, a short body \
             and failed requests"
        );
    }

    /// What the cookie path adds to each raw request, on one thread: reading the
    /// `Cookie` header for the URL from a reqwest cookie jar, empty and holding one
    /// cookie, and the check for `Set-Cookie` on a response that sets none. Prints
    /// the CPU time of each, per request, the median of 9 runs of a million.
    #[cfg(feature = "cookies")]
    fn cookie_cost() {
        use reqwest::cookie::{CookieStore, Jar};
        let url = reqwest::Url::parse("http://127.0.0.1:8080/").unwrap();
        let empty = Jar::default();
        let one = Jar::default();
        one.add_cookie_str("session=bench; Path=/", &url);
        let headers = hyper::HeaderMap::new();
        const N: u32 = 1_000_000;
        let time = |f: &dyn Fn() -> bool| {
            let mut runs: Vec<f64> = (0..9)
                .map(|_| {
                    let start = cpu_time();
                    let mut hits = 0u32;
                    for _ in 0..N {
                        hits += std::hint::black_box(f()) as u32;
                    }
                    std::hint::black_box(hits);
                    (cpu_time() - start).as_nanos() as f64 / f64::from(N) / 1000.0
                })
                .collect();
            median(&mut runs)
        };
        let lookup_empty = time(&|| empty.cookies(std::hint::black_box(&url)).is_some());
        let lookup_one = time(&|| one.cookies(std::hint::black_box(&url)).is_some());
        let set_cookie_check = time(&|| {
            std::hint::black_box(&headers)
                .get_all(hyper::header::SET_COOKIE)
                .iter()
                .next()
                .is_some()
        });
        println!("| Cookie path step | CPU µs per request, median of 9 |");
        println!("|---|---:|");
        println!("| `Cookie` from an empty jar | {:.3} |", lookup_empty);
        println!(
            "| `Cookie` from a jar holding one cookie | {:.3} |",
            lookup_one
        );
        println!("| `Set-Cookie` check, none set | {:.3} |", set_cookie_check);
    }

    #[cfg(not(feature = "cookies"))]
    fn cookie_cost() {
        panic!("cookie-cost needs Goose's cookies feature");
    }

    /// The median of `values`, which this sorts.
    fn median(values: &mut [f64]) -> f64 {
        values.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let middle = values.len() / 2;
        if values.len() % 2 == 1 {
            values[middle]
        } else {
            (values[middle - 1] + values[middle]) / 2.0
        }
    }
}
