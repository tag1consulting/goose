use gumdrop::Options;
use nng::*;
use serde::{Deserialize, Serialize};
use std::io::BufWriter;
use std::sync::atomic::Ordering;
use std::time;
use url::Url;

const EMPTY_ARGS: Vec<&str> = vec![];

use crate::goose::{GooseUser, GooseUserCommand};
use crate::manager::GooseUserInitializer;
use crate::metrics::{GooseErrorMetrics, GooseRequestMetrics, TransactionMetrics};
use crate::test_plan::TestPlan;
use crate::{get_worker_id, AttackMode, GooseAttack, GooseConfiguration, WORKER_ID};

/// Workers send GaggleMetrics to the Manager process to be aggregated together.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GaggleMetrics {
    /// Load test hash, used to ensure all Workers are running the same load test.
    WorkerInit(u64),
    /// Goose request metrics.
    Requests(GooseRequestMetrics),
    /// Goose transaction metrics.
    Transactions(TransactionMetrics),
    /// Goose error metrics.
    Errors(GooseErrorMetrics),
}

// If pipe closes unexpectedly, panic.
fn pipe_closed(_pipe: Pipe, event: PipeEvent) {
    if event == PipeEvent::RemovePost {
        panic!("[{}] manager went away, exiting", get_worker_id());
    }
}

// If pipe closes during shutdown, just log it.
fn pipe_closed_during_shutdown(_pipe: Pipe, event: PipeEvent) {
    if event == PipeEvent::RemovePost {
        info!("[{}] manager went away", get_worker_id());
    }
}

// Helper that registers the shutdown pipe handler, avoiding a panic when we
// expect the manager to exit.
pub fn register_shutdown_pipe_handler(manager: &Socket) {
    manager
        .pipe_notify(pipe_closed_during_shutdown)
        .map_err(|error| eprintln!("{:?} worker_id({})", error, get_worker_id()))
        .expect("failed to set up new pipe handler");
}

pub(crate) async fn worker_main(goose_attack: GooseAttack) -> GooseAttack {
    // Creates a TCP address.
    let address = format!(
        "tcp://{}:{}",
        goose_attack.configuration.manager_host, goose_attack.configuration.manager_port
    );
    info!("worker connecting to manager at {}", &address);

    // Create a request socket.
    let manager = Socket::new(Protocol::Req0)
        .map_err(|error| eprintln!("{:?} address({})", error, address))
        .expect("failed to create socket");

    manager
        .pipe_notify(pipe_closed)
        .map_err(|error| eprintln!("{:?}", error))
        .expect("failed to set up pipe handler");

    // Pause 1/10 of a second in case we're blocking on a cargo lock.
    tokio::time::sleep(time::Duration::from_millis(100)).await;
    // Connect to manager.
    let mut retries = 0;
    loop {
        match manager.dial(&address) {
            Ok(_) => break,
            Err(e) => {
                if retries >= 5 {
                    panic!("failed to communicate with manager at {}: {}.", &address, e);
                }
                debug!("failed to communicate with manager at {}: {}.", &address, e);
                let sleep_duration = time::Duration::from_millis(500);
                debug!(
                    "sleeping {:?} milliseconds waiting for manager...",
                    sleep_duration
                );
                tokio::time::sleep(sleep_duration).await;
                retries += 1;
            }
        }
    }

    // Send manager the hash of the load test we are ready to run.
    push_metrics_to_manager(
        &manager,
        vec![GaggleMetrics::WorkerInit(goose_attack.metrics.hash)],
        false,
    );

    let mut config: GooseConfiguration = GooseConfiguration::parse_args_default(&EMPTY_ARGS)
        .expect("failed to generate default configuration");
    let mut weighted_users: Vec<GooseUser> = Vec::new();

    // Wait for the manager to send user parameters.
    info!("waiting for instructions from manager");
    let msg = manager
        .recv()
        .map_err(|error| eprintln!("{:?}", error))
        .expect("error receiving manager message");

    let initializers: Vec<GooseUserInitializer> = match serde_cbor::from_reader(msg.as_slice()) {
        Ok(i) => i,
        Err(_) => {
            let command: GooseUserCommand = match serde_cbor::from_reader(msg.as_slice()) {
                Ok(c) => c,
                Err(e) => {
                    panic!("invalid message received: {}", e);
                }
            };
            match command {
                GooseUserCommand::Exit => {
                    panic!("unexpected GooseUserCommand::Exit from manager during startup");
                }
                other => {
                    panic!("unknown command from manager: {:?}", other);
                }
            }
        }
    };

    let mut worker_id: usize = 0;
    // Allocate a state for each user that will be spawned.
    info!("initializing user states...");
    for initializer in initializers {
        if worker_id == 0 {
            worker_id = initializer.worker_id;
        }
        let user = GooseUser::new(
            initializer.scenarios_index,
            Url::parse(&initializer.base_url).unwrap(),
            &initializer.config,
            goose_attack.metrics.hash,
        )
        .map_err(|error| eprintln!("{:?} worker_id({})", error, get_worker_id()))
        .expect("failed to create socket");

        // The initializer.config is the same for all users, only copy it one time.
        if weighted_users.is_empty() {
            config = initializer.config;
        }
        weighted_users.push(user);
    }
    WORKER_ID.store(worker_id, Ordering::Relaxed);
    info!(
        "[{}] initialized {} user states",
        get_worker_id(),
        weighted_users.len()
    );

    info!("[{}] waiting for go-ahead from manager", get_worker_id());

    // Wait for the manager to send go-ahead to start the load test.
    loop {
        // Push metrics to manager to force a reply, waiting for GooseUserCommand::Run.
        push_metrics_to_manager(
            &manager,
            vec![GaggleMetrics::WorkerInit(goose_attack.metrics.hash)],
            false,
        );
        let msg = manager
            .recv()
            .map_err(|error| eprintln!("{:?} worker_id({})", error, get_worker_id()))
            .expect("error receiving manager message");

        let command: GooseUserCommand = serde_cbor::from_reader(msg.as_slice())
            .map_err(|error| eprintln!("{:?} worker_id({})", error, get_worker_id()))
            .expect("invalid message received");

        match command {
            // Break out of loop and start the load test.
            GooseUserCommand::Run => break,
            // Exit worker process immediately.
            GooseUserCommand::Exit => {
                warn!(
                    "[{}] received GooseUserCommand::Exit command from manager",
                    get_worker_id()
                );
                std::process::exit(0);
            }
            // Sleep and then loop again.
            _ => {
                let sleep_duration = time::Duration::from_secs(1);
                debug!(
                    "[{}] sleeping {:?} second waiting for manager...",
                    get_worker_id(),
                    sleep_duration
                );
                tokio::time::sleep(sleep_duration).await;
            }
        }
    }

    // Worker is officially starting the load test.
    info!(
        "[{}] entering gaggle mode, starting load test",
        get_worker_id()
    );
    let mut worker_goose_attack = GooseAttack::initialize_with_config(config.clone())
        .map_err(|error| eprintln!("{:?} worker_id({})", error, get_worker_id()))
        .expect("failed to launch GooseAttack");

    worker_goose_attack.started = Some(time::Instant::now());
    worker_goose_attack.scenarios = goose_attack.scenarios.clone();
    worker_goose_attack.weighted_users = weighted_users;
    // This is a Worker instance, not a Manager instance.
    worker_goose_attack.configuration.manager = false;
    worker_goose_attack.configuration.worker = true;
    // The request_log option is configured on the Worker.
    worker_goose_attack.configuration.request_log =
        goose_attack.configuration.request_log.to_string();
    // The request_format option is configured on the Worker.
    worker_goose_attack.configuration.request_format =
        goose_attack.configuration.request_format.clone();
    // The transaction_log option is configured on the Worker.
    worker_goose_attack.configuration.transaction_log =
        goose_attack.configuration.transaction_log.to_string();
    // The transaction_format option is configured on the Worker.
    worker_goose_attack.configuration.transaction_format =
        goose_attack.configuration.transaction_format.clone();
    // The error_log option is configured on the Worker.
    worker_goose_attack.configuration.error_log = goose_attack.configuration.error_log.to_string();
    // The error_format option is configured on the Worker.
    worker_goose_attack.configuration.error_format =
        goose_attack.configuration.error_format.clone();
    // The debug_log option is configured on the Worker.
    worker_goose_attack.configuration.debug_log = goose_attack.configuration.debug_log.to_string();
    // The debug_format option is configured on the Worker.
    worker_goose_attack.configuration.debug_format =
        goose_attack.configuration.debug_format.clone();
    // The throttle_requests option is set on the Worker.
    worker_goose_attack.configuration.throttle_requests =
        goose_attack.configuration.throttle_requests;
    worker_goose_attack.attack_mode = AttackMode::Worker;
    worker_goose_attack.defaults = goose_attack.defaults.clone();
    worker_goose_attack.test_plan = TestPlan::build(&worker_goose_attack.configuration);

    worker_goose_attack
        .start_attack(Some(manager))
        .await
        .map_err(|error| eprintln!("{:?} worker_id({})", error, get_worker_id()))
        .expect("failed to launch GooseAttack")
}

// Push metrics to manager.
pub fn push_metrics_to_manager(
    manager: &Socket,
    metrics: Vec<GaggleMetrics>,
    get_response: bool,
) -> bool {
    debug!("[{}] pushing metrics to manager", get_worker_id(),);
    let mut message = BufWriter::new(Message::new());

    serde_cbor::to_writer(&mut message, &metrics)
        .map_err(|error| eprintln!("{:?} worker_id({})", error, get_worker_id()))
        .expect("failed to serialize GaggleMetrics");

    manager
        .try_send(
            message
                .into_inner()
                .expect("failed to extract nng message from buffer"),
        )
        .map_err(|error| eprintln!("{:?} worker_id({})", error, get_worker_id()))
        .expect("communication failure");

    if get_response {
        // Wait for server to reply.
        let msg = manager
            .recv()
            .map_err(|error| eprintln!("{:?} worker_id({})", error, get_worker_id()))
            .expect("error receiving manager message");

        let command: GooseUserCommand = serde_cbor::from_reader(msg.as_slice())
            .map_err(|error| eprintln!("{:?} worker_id({})", error, get_worker_id()))
            .expect("invalid message");

        if command == GooseUserCommand::Exit {
            info!(
                "[{}] received GooseUserCommand::Exit command from manager",
                get_worker_id()
            );
            // Shutting down, register shutdown pipe handler.
            register_shutdown_pipe_handler(manager);
            return false;
        }
    }
    true
}
