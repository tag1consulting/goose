use lazy_static::lazy_static;
use nng::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::BufWriter;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time;

use crate::metrics::{
    self, GooseErrorMetricAggregate, GooseErrorMetrics, GooseRequestMetricAggregate,
    GooseRequestMetrics, TransactionMetricAggregate, TransactionMetrics,
};
use crate::util;
use crate::worker::GaggleMetrics;
use crate::{GooseAttack, GooseConfiguration, GooseUserCommand};

/// How long the manager will wait for all workers to stop after the load test ends.
const GRACEFUL_SHUTDOWN_TIMEOUT: usize = 30;

/// All elements required to initialize a user in a worker process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GooseUserInitializer {
    /// An index into the internal `GooseTest.scenarios` vector, indicating which Scenario is running.
    pub scenarios_index: usize,
    /// The base_url for this user thread.
    pub base_url: String,
    /// A local copy of the global GooseConfiguration.
    pub config: GooseConfiguration,
    /// Numerical identifier for worker.
    pub worker_id: usize,
}

// Mutable singleton globally tracking how many workers are currently being managed.
lazy_static! {
    static ref ACTIVE_WORKERS: AtomicUsize = AtomicUsize::new(0);
}

fn distribute_users(goose_attack: &GooseAttack) -> (usize, usize) {
    // Users and expect_workers is required to get here, so unwrap() is safe.
    let users_per_worker = goose_attack.configuration.users.unwrap()
        / (goose_attack.configuration.expect_workers.unwrap() as usize);
    let users_remainder = goose_attack.configuration.users.unwrap()
        % (goose_attack.configuration.expect_workers.unwrap() as usize);
    if users_remainder > 0 {
        info!(
            "each worker to start {} users, assigning 1 extra to {} workers",
            users_per_worker, users_remainder
        );
    } else {
        info!("each worker to start {} users", users_per_worker);
    }
    (users_per_worker, users_remainder)
}

fn pipe_closed(_pipe: Pipe, event: PipeEvent) {
    match event {
        PipeEvent::AddPost => {
            debug!("worker pipe added");
            ACTIVE_WORKERS.fetch_add(1, Ordering::SeqCst);
        }
        PipeEvent::RemovePost => {
            let active_workers = ACTIVE_WORKERS.fetch_sub(1, Ordering::SeqCst);
            info!("worker {} exited", active_workers);
        }
        _ => {}
    }
}

/// Merge per-user transaction metrics from user thread into global parent metrics
fn merge_transactions_from_worker(
    parent_transaction: &TransactionMetricAggregate,
    user_transaction: &TransactionMetricAggregate,
) -> TransactionMetricAggregate {
    // Make a mutable copy where we can merge things
    let mut merged_transaction = parent_transaction.clone();
    // Iterate over user times, and merge into global time
    merged_transaction.times =
        metrics::merge_times(merged_transaction.times, user_transaction.times.clone());
    // Increment total transaction time counter.
    merged_transaction.total_time += &user_transaction.total_time;
    // Increment count of how many transaction counters we've seen.
    merged_transaction.counter += &user_transaction.counter;
    // If user had new fastest transaction time, update global fastest transaction time.
    merged_transaction.min_time =
        metrics::update_min_time(merged_transaction.min_time, user_transaction.min_time);
    // If user had new slowest transaction time, update global slowest transaction time.
    merged_transaction.max_time =
        metrics::update_max_time(merged_transaction.max_time, user_transaction.max_time);
    // Increment total success counter.
    merged_transaction.success_count += &user_transaction.success_count;
    // Increment total fail counter.
    merged_transaction.fail_count += &user_transaction.fail_count;
    merged_transaction
}

/// Merge per-user request metrics from user thread into global parent metrics
fn merge_requests_from_worker(
    parent_request: &GooseRequestMetricAggregate,
    user_request: &GooseRequestMetricAggregate,
    status_codes: bool,
) -> GooseRequestMetricAggregate {
    // Make a mutable copy where we can merge things
    let mut merged_request = parent_request.clone();
    // Iterate over user response times, and merge into global response time
    merged_request.raw_data.times = metrics::merge_times(
        merged_request.raw_data.times,
        user_request.raw_data.times.clone(),
    );
    // Increment total response time counter.
    merged_request.raw_data.total_time += &user_request.raw_data.total_time;
    // Increment count of how many response counters we've seen.
    merged_request.raw_data.counter += &user_request.raw_data.counter;
    // If user had new fastest response time, update global fastest response time.
    merged_request.raw_data.minimum_time = metrics::update_min_time(
        merged_request.raw_data.minimum_time,
        user_request.raw_data.minimum_time,
    );
    // If user had new slowest response time, update global slowest response time.
    merged_request.raw_data.maximum_time = metrics::update_max_time(
        merged_request.raw_data.maximum_time,
        user_request.raw_data.maximum_time,
    );
    // Increment total success counter.
    merged_request.success_count += &user_request.success_count;
    // Increment total fail counter.
    merged_request.fail_count += &user_request.fail_count;
    // Only accrue overhead of merging status_code_counts if we're going to display the results
    if status_codes {
        for (status_code, count) in &user_request.status_code_counts {
            // Add user count into global count
            let new_count = if let Some(existing_status_code_count) =
                merged_request.status_code_counts.get(status_code)
            {
                *existing_status_code_count + *count
            }
            // No global count exists yet, so start with user count
            else {
                *count
            };
            merged_request
                .status_code_counts
                .insert(*status_code, new_count);
        }
    }
    merged_request
}

/// Merge per-Worker errors into global Manager metrics
fn merge_errors_from_worker(
    manager_error: &GooseErrorMetricAggregate,
    worker_error: &GooseErrorMetricAggregate,
) -> GooseErrorMetricAggregate {
    // Make a mutable copy where we can merge things
    let mut merged_error = manager_error.clone();
    // Add in how many additional times this happened on the Worker.
    merged_error.occurrences += worker_error.occurrences;
    // Nothing else changes, so return the merged error.
    merged_error
}

/// Helper to send GooseUserCommand::Exit command to worker.
fn tell_worker_to_exit(server: &Socket) -> bool {
    let mut message = Message::new();
    serde_cbor::to_writer(&mut message, &GooseUserCommand::Exit)
        .map_err(|error| eprintln!("{:?}", error))
        .expect("failed to serialize user command");
    send_message_to_worker(server, message)
}

/// Helper to send message to worker.
fn send_message_to_worker(server: &Socket, message: Message) -> bool {
    // If there's an error, handle it.
    if let Err((_, e)) = server.try_send(message) {
        match e {
            Error::TryAgain => {
                if ACTIVE_WORKERS.load(Ordering::SeqCst) == 0 {
                    info!("all workers have exited");
                    return false;
                }
            }
            _ => {
                panic!("communication failure: {:?}", e);
            }
        }
    }
    true
}

/// Helper to merge in request metrics from Worker.
fn merge_request_metrics(goose_attack: &mut GooseAttack, requests: GooseRequestMetrics) {
    if !requests.is_empty() {
        debug!("requests metrics received: {:?}", requests.len());
        for (request_key, request) in requests {
            trace!("request_key: {}", request_key);
            let merged_request =
                if let Some(parent_request) = goose_attack.metrics.requests.get(&request_key) {
                    merge_requests_from_worker(
                        parent_request,
                        &request,
                        goose_attack.configuration.status_codes,
                    )
                } else {
                    // First time seeing this request, simply insert it.
                    request.clone()
                };
            goose_attack
                .metrics
                .requests
                .insert(request_key.to_string(), merged_request);
        }
    }
}

/// Helper to merge in transaction metrics from Worker.
fn merge_transaction_metrics(goose_attack: &mut GooseAttack, transactions: TransactionMetrics) {
    for scenario in transactions {
        for transaction in scenario {
            let merged_transaction = merge_transactions_from_worker(
                &goose_attack.metrics.transactions[transaction.scenario_index]
                    [transaction.transaction_index],
                &transaction,
            );
            goose_attack.metrics.transactions[transaction.scenario_index]
                [transaction.transaction_index] = merged_transaction;
        }
    }
}

/// Helper to merge in errors from the Worker.
fn merge_error_metrics(goose_attack: &mut GooseAttack, errors: GooseErrorMetrics) {
    if !errors.is_empty() {
        debug!("errors received: {:?}", errors.len());
        for (error_key, error) in errors {
            trace!("error_key: {}", error_key);
            let merged_error =
                if let Some(parent_error) = goose_attack.metrics.errors.get(&error_key) {
                    merge_errors_from_worker(parent_error, &error)
                } else {
                    // First time seeing this error, simply insert it.
                    error.clone()
                };
            goose_attack
                .metrics
                .errors
                .insert(error_key.to_string(), merged_error);
        }
    }
}

/// Main manager loop.
pub(crate) async fn manager_main(mut goose_attack: GooseAttack) -> GooseAttack {
    // Creates a TCP address.
    let address = format!(
        "tcp://{}:{}",
        goose_attack.configuration.manager_bind_host, goose_attack.configuration.manager_bind_port
    );
    debug!("preparing to listen for workers at: {}", &address);

    // Create a Rep0 reply socket.
    let server = Socket::new(Protocol::Rep0)
        .map_err(|error| eprintln!("{:?}", error))
        .expect("failed to create socket");

    // Set up callback function to receive pipe event notifications.
    server
        .pipe_notify(pipe_closed)
        .map_err(|error| eprintln!("{:?}", error))
        .expect("failed to set up pipe handler");

    // Listen for connections.
    server
        .listen(&address)
        .map_err(|error| eprintln!("{:?} (address = {})", error, address))
        .expect("failed to bind to socket");

    // Expect workers is reqiured so unwrap() is safe.
    info!(
        "manager listening on {}, waiting for {} workers",
        &address,
        goose_attack.configuration.expect_workers.unwrap(),
    );

    // Calculate how many users each worker will be responsible for.
    let (users_per_worker, mut users_remainder) = distribute_users(&goose_attack);

    // A mutable bucket of users to be assigned to workers.
    let mut available_users = goose_attack.weighted_gaggle_users.clone();

    // Track how many workers we've seen.
    let mut workers: HashSet<Pipe> = HashSet::new();

    // Track start time, we'll reset this when the test actually starts.
    goose_attack.started = Some(time::Instant::now());
    let mut running_metrics_timer = time::Instant::now();
    let mut exit_timer = time::Instant::now();
    let mut load_test_running = false;
    let mut load_test_finished = false;

    // Catch ctrl-c to allow clean shutdown to display metrics.
    let canceled = Arc::new(AtomicBool::new(false));
    util::setup_ctrlc_handler(&canceled);

    // Initialize the optional transaction metrics.
    goose_attack
        .metrics
        .initialize_transaction_metrics(
            &goose_attack.scenarios,
            &goose_attack.configuration,
            &goose_attack.defaults,
        )
        .expect("failed to initialize transaction metrics");

    // Update metrics, which doesn't happen automatically on the Master as we don't
    // invoke start_attack. Hatch rate is required here so unwrap() is safe.
    // Divide the number of new users to launch by the time configured to launch them.
    let hatch_rate: f32 = (goose_attack.test_plan.steps[0].0)
        as f32
        / goose_attack.test_plan.steps[0].1 as f32
        // Convert from milliseconds to seconds.
        * 1_000.0;
    let maximum_hatched = hatch_rate * goose_attack.test_plan.steps[1].1 as f32;
    if maximum_hatched < goose_attack.configuration.users.unwrap() as f32 {
        goose_attack.metrics.users = maximum_hatched as usize;
    } else {
        goose_attack.metrics.users = goose_attack.configuration.users.unwrap();
    }

    // Worker control loop.
    loop {
        // While running load test, check if any workers go away.
        if !load_test_finished {
            // If ACTIVE_WORKERS is less than the total workers seen, a worker went away.
            if ACTIVE_WORKERS.load(Ordering::SeqCst) < workers.len() {
                // If worked goes away during load test, exit gracefully.
                if load_test_running {
                    info!(
                        "worker went away, stopping gracefully after {} seconds...",
                        goose_attack.started.unwrap().elapsed().as_secs()
                    );
                    load_test_finished = true;
                    exit_timer = time::Instant::now();
                }
                // If a worker goes away during start up, exit immediately.
                else {
                    warn!("worker went away, stopping immediately...");
                    break;
                }
            }
        }
        if load_test_running {
            if !load_test_finished {
                // Test ran to completion or was canceled with ctrl-c.
                if util::ms_timer_expired(
                    goose_attack.started.unwrap(),
                    goose_attack.test_plan.steps[1].1,
                ) || canceled.load(Ordering::SeqCst)
                {
                    info!(
                        "stopping after {} seconds...",
                        goose_attack.started.unwrap().elapsed().as_secs()
                    );
                    goose_attack.metrics.duration =
                        goose_attack.started.unwrap().elapsed().as_secs() as usize;
                    load_test_finished = true;
                    exit_timer = time::Instant::now();
                }
            }

            // Aborting graceful shutdown, workers took too long to shut down.
            if load_test_finished && util::timer_expired(exit_timer, GRACEFUL_SHUTDOWN_TIMEOUT) {
                warn!("graceful shutdown timer expired, exiting...");
                break;
            }

            // When displaying running metrics, sync data from user threads first.
            if let Some(running_metrics) = goose_attack.configuration.running_metrics {
                if util::timer_expired(running_metrics_timer, running_metrics) {
                    // Reset timer each time we display metrics.
                    running_metrics_timer = time::Instant::now();
                    goose_attack.metrics.duration =
                        goose_attack.started.unwrap().elapsed().as_secs() as usize;
                    goose_attack.metrics.print_running();
                }
            }
        } else if canceled.load(Ordering::SeqCst) {
            info!("load test canceled, exiting");
            std::process::exit(1);
        }

        // Check for messages from workers.
        match server.try_recv() {
            Ok(mut msg) => {
                // Message received, grab the pipe to determine which worker it is.
                let pipe = msg.pipe().expect("fatal error getting worker pipe");

                // Workers always send a vector of GooseMetric objects.
                let mut gaggle_metrics: Vec<GaggleMetrics> =
                    serde_cbor::from_reader(msg.as_slice()).unwrap();

                // Check if we're seeing this worker for the first time.
                if !workers.contains(&pipe) {
                    // Check if we are expecting another worker. Expect workers is required
                    // so unwrap() is safe.
                    if workers.len() >= goose_attack.configuration.expect_workers.unwrap() as usize
                    {
                        warn!(
                            "telling extra worker ({} of {}) to exit",
                            workers.len() + 1,
                            goose_attack.configuration.expect_workers.unwrap()
                        );
                        // We already have enough workers, tell this extra one to
                        // GooseUserCommand::Exit.
                        if !tell_worker_to_exit(&server) {
                            // All workers have exited, shut down the
                            // load test.
                            break;
                        }
                    }
                    // We need another worker, accept the connection.
                    else {
                        // New worker has to send us a single
                        // GaggleMetrics::WorkerInit object or it's invalid.
                        if gaggle_metrics.len() != 1 {
                            warn!("invalid message from Worker, exiting load test");
                            // Invalid message, tell worker to GooseUserCommand::Exit.
                            if !tell_worker_to_exit(&server) {
                                // All workers have exited, shut down the
                                // load test.
                                break;
                            }
                        }

                        let goose_metric = gaggle_metrics.pop().unwrap();
                        if let GaggleMetrics::WorkerInit(load_test_hash) = goose_metric {
                            if load_test_hash != goose_attack.metrics.hash {
                                if goose_attack.configuration.no_hash_check {
                                    warn!("worker is running a different load test, ignoring");
                                } else {
                                    panic!("worker is running a different load test, set --no-hash-check to ignore");
                                }
                            }
                        } else {
                            // Unexpected object received, tell the worker
                            // to GooseUserCommand::Exit.
                            warn!("invalid object from Worker, exiting load test");
                            if !tell_worker_to_exit(&server) {
                                // All workers have exited, shut down the
                                // load test.
                                break;
                            }
                        }

                        workers.insert(pipe);
                        // Expect workers is required so unwrap() is safe.
                        info!(
                            "worker {} of {} connected",
                            workers.len(),
                            goose_attack.configuration.expect_workers.unwrap(),
                        );

                        // Send new worker a batch of users.
                        let mut user_batch = users_per_worker;
                        // If remainder, put extra user in this batch.
                        if users_remainder > 0 {
                            users_remainder -= 1;
                            user_batch += 1;
                        }
                        let mut users = Vec::new();

                        // Pop users from available_users vector and build worker initializer.
                        debug!("sending {} users to worker", user_batch);
                        for _ in 1..=user_batch {
                            let user = match available_users.pop() {
                                Some(u) => u,
                                None => {
                                    panic!("not enough available users!?");
                                }
                            };
                            // Build a vector of GooseUser initializers for next worker.
                            users.push(GooseUserInitializer {
                                scenarios_index: user.scenarios_index,
                                base_url: user.base_url.read().await.to_string(),
                                config: user.config.clone(),
                                worker_id: workers.len(),
                            });
                        }

                        // Prepare to serialize the list of users to send to the Worker.
                        let mut message = BufWriter::new(Message::new());

                        info!("serializing users with serde_cbor...");
                        serde_cbor::to_writer(&mut message, &users)
                            .map_err(|error| eprintln!("{:?}", error))
                            .expect("failed to serialize user initializers");

                        info!("sending {} users to worker {}", users.len(), workers.len());
                        if !send_message_to_worker(
                            &server,
                            message
                                .into_inner()
                                .expect("failed to extract nng message from buffer"),
                        ) {
                            // All workers have exited, shut down the load
                            // test.
                            break;
                        }

                        // Expect workers is required so unwrap() is safe.
                        if workers.len()
                            == goose_attack.configuration.expect_workers.unwrap() as usize
                        {
                            info!("gaggle distributed load test started");
                            // Reset start time, the distributed load test is truly starting now.
                            goose_attack.started = Some(time::Instant::now());
                            running_metrics_timer = time::Instant::now();
                            load_test_running = true;

                            // Run any configured test_start() functions.
                            goose_attack.run_test_start().await.unwrap();
                        }
                    }
                }
                // Received message from known Worker.
                else {
                    let mut message = Message::new();

                    // When starting a Gaggle, some Workers may start before others and
                    // will send regular heartbeats to the Manager to confirm the load
                    // test is still waiting to start.
                    if !load_test_running {
                        // Assume this is the Worker heartbeat, tell it to keep waiting.
                        serde_cbor::to_writer(&mut message, &GooseUserCommand::Wait)
                            .map_err(|error| eprintln!("{:?}", error))
                            .expect("failed to serialize user command");
                        if !send_message_to_worker(&server, message) {
                            // All workers have exited, shut down the load test.
                            break;
                        }
                        continue;
                    }

                    for metric in gaggle_metrics {
                        match metric {
                            // Merge in request metrics from Worker.
                            GaggleMetrics::Requests(requests) => {
                                merge_request_metrics(&mut goose_attack, requests)
                            }
                            // Merge in transaction metrics from Worker.
                            GaggleMetrics::Transactions(transactions) => {
                                merge_transaction_metrics(&mut goose_attack, transactions)
                            }
                            // Merge in error metrics from Worker.
                            GaggleMetrics::Errors(errors) => {
                                merge_error_metrics(&mut goose_attack, errors)
                            }
                            // Ignore Worker heartbeats.
                            GaggleMetrics::WorkerInit(_) => (),
                        }
                    }

                    if load_test_finished {
                        debug!("telling worker to exit");
                        serde_cbor::to_writer(&mut message, &GooseUserCommand::Exit)
                            .map_err(|error| eprintln!("{:?}", error))
                            .expect("failed to serialize user command");
                    }
                    // Notify the worker that the load test is still running.
                    else {
                        serde_cbor::to_writer(&mut message, &GooseUserCommand::Run)
                            .map_err(|error| eprintln!("{:?}", error))
                            .expect("failed to serialize user command");
                    }
                    if !send_message_to_worker(&server, message) {
                        // All workers have exited, shut down the load
                        // test.
                        break;
                    }
                }
            }
            Err(e) => {
                if e == Error::TryAgain {
                    if !workers.is_empty() && ACTIVE_WORKERS.load(Ordering::SeqCst) == 0 {
                        info!("all workers have exited");
                        break;
                    }
                    if !load_test_finished {
                        // Sleep a tenth of a second then return to the loop.
                        tokio::time::sleep(time::Duration::from_millis(100)).await;
                    }
                } else {
                    panic!("error receiving user message: {}", e);
                }
            }
        }
    }
    // Run any configured test_stop() functions.
    goose_attack.run_test_stop().await.unwrap();

    goose_attack
}

#[cfg(test)]
mod tests {
    use super::*;

    use gumdrop::Options;

    #[test]
    fn test_distribute_users() {
        let ten_users_two_workers: Vec<&str> = vec!["--users", "10", "--expect-workers", "2"];
        let config = GooseConfiguration::parse_args_default(&ten_users_two_workers).unwrap();
        let goose_attack = GooseAttack::initialize_with_config(config).unwrap();
        let (users_per_process, users_remainder) = distribute_users(&goose_attack);
        assert_eq!(users_per_process, 5);
        assert_eq!(users_remainder, 0);

        let one_user_one_worker: Vec<&str> = vec!["--users", "1", "--expect-workers", "1"];
        let config = GooseConfiguration::parse_args_default(&one_user_one_worker).unwrap();
        let goose_attack = GooseAttack::initialize_with_config(config).unwrap();
        let (users_per_process, users_remainder) = distribute_users(&goose_attack);
        assert_eq!(users_per_process, 1);
        assert_eq!(users_remainder, 0);

        let onehundred_users_twentyone_workers: Vec<&str> =
            vec!["--users", "100", "--expect-workers", "21"];
        let config =
            GooseConfiguration::parse_args_default(&onehundred_users_twentyone_workers).unwrap();
        let goose_attack = GooseAttack::initialize_with_config(config).unwrap();
        let (users_per_process, users_remainder) = distribute_users(&goose_attack);
        assert_eq!(users_per_process, 4);
        assert_eq!(users_remainder, 16);
    }
}
