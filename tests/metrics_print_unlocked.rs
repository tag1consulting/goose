//! Goose formats its metrics before it prints them, so no thread holds
//! stdout while the formatting logs. A print that held stdout while the
//! metrics were formatted deadlocked with any other thread logging then: the
//! logger takes its own lock, and then stdout's.
//!
//! A test of its own, without the test harness, which would capture stdout:
//! the logger here is a probe that, as Goose logs from inside the metrics'
//! formatting, checks from another thread that stdout is free.

use httpmock::{Method::GET, MockServer};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use goose::config::GooseConfiguration;
use goose::prelude::*;

/// Set when stdout was held while the metrics logged.
static HELD: AtomicBool = AtomicBool::new(false);

/// The metrics' percentile log lines checked.
static CHECKED: AtomicUsize = AtomicUsize::new(0);

/// A logger that checks stdout is free each time the metrics log.
struct Probe;

impl log::Log for Probe {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        // The line the metrics' formatting logs, for each percentile.
        if !record.target().starts_with("goose::metrics")
            || !record.args().to_string().starts_with("percentile:")
            || CHECKED.load(Ordering::SeqCst) >= 20
        {
            return;
        }
        CHECKED.fetch_add(1, Ordering::SeqCst);
        let (locked, free) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _stdout = std::io::stdout().lock();
            let _ = locked.send(());
        });
        if free.recv_timeout(Duration::from_secs(2)).is_err() {
            HELD.store(true, Ordering::SeqCst);
        }
    }

    fn flush(&self) {}
}

async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/").await?;
    Ok(())
}

fn main() {
    log::set_boxed_logger(Box::new(Probe)).expect("the probe is the only logger");
    log::set_max_level(log::LevelFilter::Trace);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/");
                then.status(200).body("ok");
            })
            .await;
        let host = server.base_url();
        let configuration = GooseConfiguration::parse_args_default(&[
            "--host",
            &host,
            "--users",
            "1",
            "--run-time",
            "1",
            "--no-telnet",
            "--no-websocket",
            // Goose's own logger, refused as the probe is installed, still
            // sets the level it asks for: debug, for the percentile lines.
            "-v",
        ])
        .unwrap();
        GooseAttack::initialize_with_config(configuration)
            .unwrap()
            .register_scenario(scenario!("Index").register_transaction(transaction!(get_index)))
            .execute()
            .await
            .unwrap();
    });
    assert!(
        CHECKED.load(Ordering::SeqCst) > 0,
        "the metrics logged nothing to check"
    );
    assert!(
        !HELD.load(Ordering::SeqCst),
        "stdout was held while the metrics were formatted"
    );
    println!("test metrics_print_unlocked ... ok");
}
