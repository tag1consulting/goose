use rand::Rng;
use std::time::{self, Duration};

use crate::goose::{GooseUser, GooseUserCommand, Scenario, TransactionFunction};
use crate::logger::GooseLog;
use crate::metrics::{GooseMetric, ScenarioMetric, TransactionMetric};

pub(crate) async fn user_main(
    thread_number: usize,
    thread_scenario: Scenario,
    mut thread_user: GooseUser,
    thread_receiver: flume::Receiver<GooseUserCommand>,
) {
    info!(
        "launching user {} from {}...",
        thread_number, thread_scenario.name
    );

    // User is starting, first invoke the weighted on_start transactions.
    if !thread_scenario.weighted_on_start_transactions.is_empty() {
        // Transactions are already weighted and scheduled, execute each in order.
        for (thread_transaction_index, thread_transaction_name) in
            &thread_scenario.weighted_on_start_transactions
        {
            // Determine which transaction we're going to run next.
            let function = &thread_scenario.transactions[*thread_transaction_index].function;
            debug!(
                "[user {}]: launching on_start {} transaction from {}",
                thread_number, thread_transaction_name, thread_scenario.name
            );
            // Invoke the transaction function.
            let _todo = invoke_transaction_function(
                function,
                &mut thread_user,
                *thread_transaction_index,
                thread_transaction_name,
            )
            .await;
        }
    }

    // If normal transactions are defined, loop launching transactions until parent tells us to stop.
    if !thread_scenario.weighted_transactions.is_empty() {
        'launch_transactions: loop {
            // Tracks the time it takes to loop through all Transactions when Coordinated Omission
            // Mitigation is enabled.
            thread_user.update_request_cadence(thread_number).await;
            let scenario_started = time::Instant::now();

            for (thread_transaction_index, thread_transaction_name) in
                &thread_scenario.weighted_transactions
            {
                // Determine which transaction we're going to run next.
                let function = &thread_scenario.transactions[*thread_transaction_index].function;
                debug!(
                    "[user {}]: launching {} transaction from {}",
                    thread_number, thread_transaction_name, thread_scenario.name
                );
                // Invoke the transaction function.
                let _todo = invoke_transaction_function(
                    function,
                    &mut thread_user,
                    *thread_transaction_index,
                    thread_transaction_name,
                )
                .await;

                if received_exit(&thread_receiver) {
                    break 'launch_transactions;
                }

                // If the transaction_wait is defined, wait for a random time between transaction.
                if let Some((min, max)) = thread_scenario.transaction_wait {
                    // Total time left to wait before running the next transaction.
                    let mut wait_time = rand::thread_rng().gen_range(min..=max).as_millis();
                    // Track the time slept for Coordinated Omission Mitigation.
                    let sleep_timer = time::Instant::now();
                    // Never sleep more than 500 milliseconds, allowing a sleeping transaction to shut
                    // down quickly when the load test ends.
                    let maximum_sleep_time = 500;

                    while wait_time > 0 {
                        // Exit immediately if message received from parent.
                        if received_exit(&thread_receiver) {
                            break 'launch_transactions;
                        }

                        // Wake regularly to detect if the load test has shut down.
                        let sleep_duration = if wait_time > maximum_sleep_time {
                            wait_time -= maximum_sleep_time;
                            Duration::from_millis(maximum_sleep_time as u64)
                        } else {
                            let sleep_duration = Duration::from_millis(wait_time as u64);
                            wait_time = 0;
                            sleep_duration
                        };

                        debug!(
                            "user {} from {} sleeping {:?} ...",
                            thread_number, thread_scenario.name, sleep_duration
                        );

                        tokio::time::sleep(sleep_duration).await;
                    }
                    // Track how much time the GooseUser sleeps during this loop through all Transactions,
                    // used by Coordinated Omission Mitigation.
                    thread_user.slept += (time::Instant::now() - sleep_timer).as_millis() as u64;
                }
            }
            // Record a complete iteration running this Scenario.
            thread_user.iterations += 1;

            // Send scenario metrics to parent and logger if enabled, ignoring errors.
            let _ = record_scenario(
                &thread_scenario,
                &thread_user,
                scenario_started.elapsed().as_millis(),
            )
            .await;

            // Check if configured to exit after a certain number of iterations, and exit if
            // that number of iterations have run.
            if thread_user.config.iterations > 0
                && thread_user.iterations >= thread_user.config.iterations
            {
                // Pluralize the word "iteration" if more than one iteration completed.
                let pluralize = if thread_user.iterations == 0 {
                    "iteration"
                } else {
                    "iterations"
                };
                // Provide visual indication that a GooseUSer has completed the confifgured
                // number of iterations.
                info!(
                    "user {} completed {} {} of {}...",
                    thread_number, thread_user.iterations, pluralize, thread_scenario.name,
                );
                // Attempt to notify the parent this thread is shutting down.
                if let Some(shutdown_channel) = thread_user.shutdown_channel.clone() {
                    let _ = shutdown_channel.send(thread_number);
                }
                break 'launch_transactions;
            }
        }
    }

    // User is exiting, first invoke the weighted on_stop transactions.
    if !thread_scenario.weighted_on_stop_transactions.is_empty() {
        // Transactions are already weighted and scheduled, execute each in order.
        for (thread_transaction_index, thread_transaction_name) in
            &thread_scenario.weighted_on_stop_transactions
        {
            // Determine which transaction we're going to run next.
            let function = &thread_scenario.transactions[*thread_transaction_index].function;
            debug!(
                "[user: {}]: launching on_stop {} transaction from {}",
                thread_number, thread_transaction_name, thread_scenario.name
            );
            // Invoke the transaction function.
            let _todo = invoke_transaction_function(
                function,
                &mut thread_user,
                *thread_transaction_index,
                thread_transaction_name,
            )
            .await;
        }
    }

    // Optional debug output when exiting.
    info!(
        "exiting user {} from {}...",
        thread_number, thread_scenario.name
    );
}

// Determine if the parent has sent a GooseUserCommand::Exit message.
fn received_exit(thread_receiver: &flume::Receiver<GooseUserCommand>) -> bool {
    let mut message = thread_receiver.try_recv();
    while message.is_ok() {
        match message.unwrap() {
            // GooseUserCommand::Exit received.
            GooseUserCommand::Exit => {
                return true;
            }
            command => {
                debug!("ignoring unexpected GooseUserCommand: {:?}", command);
            }
        }
        message = thread_receiver.try_recv();
    }
    // GooseUserCommand::Exit not received.
    false
}

// Send scenario metric to parent and logger when enabled.
async fn record_scenario(
    thread_scenario: &Scenario,
    thread_user: &GooseUser,
    run_time: u128,
) -> Result<(), flume::SendError<Option<GooseLog>>> {
    if !thread_user.config.no_scenario_metrics && !thread_user.config.no_metrics {
        let raw_scenario = ScenarioMetric::new(
            thread_user.started.elapsed().as_millis(),
            &thread_scenario.name,
            thread_user.scenarios_index,
            run_time,
            thread_user.weighted_users_index,
        );
        if let Some(metrics_channel) = thread_user.metrics_channel.clone() {
            // Best effort metrics.
            let _ = metrics_channel.send(GooseMetric::Scenario(raw_scenario.clone()));
        }
        // If transaction-log is enabled, send a copy of the raw transaction metric to the logger thread.
        if !thread_user.config.scenario_log.is_empty() {
            if let Some(logger) = thread_user.logger.as_ref() {
                logger.send(Some(GooseLog::Scenario(raw_scenario)))?;
            }
        }
    }
    Ok(())
}

// Invoke the transaction function, collecting transaction metrics.
async fn invoke_transaction_function(
    function: &TransactionFunction,
    thread_user: &mut GooseUser,
    thread_transaction_index: usize,
    thread_transaction_name: &str,
) -> Result<(), flume::SendError<Option<GooseLog>>> {
    let started = time::Instant::now();
    let mut raw_transaction = TransactionMetric::new(
        thread_user.started.elapsed().as_millis(),
        thread_user.scenarios_index,
        thread_transaction_index,
        thread_transaction_name.to_string(),
        thread_user.weighted_users_index,
    );
    // Store details about the currently running transaction for logging purposes.
    if !thread_transaction_name.is_empty() {
        thread_user
            .transaction_name
            .replace(thread_transaction_name.to_string());
    } else {
        thread_user.transaction_name.take();
    }
    // As the index is optional, `""` is none, whereas `"0"` is the first index.
    thread_user.transaction_index = Some(thread_transaction_index.to_string());

    let success = function(thread_user).await.is_ok();
    raw_transaction.set_time(started.elapsed().as_millis(), success);

    // Exit if all metrics or transaction metrics are disabled.
    if thread_user.config.no_metrics || thread_user.config.no_transaction_metrics {
        return Ok(());
    }

    // If transaction-log is enabled, send a copy of the raw transaction metric to the logger thread.
    if !thread_user.config.transaction_log.is_empty() {
        if let Some(logger) = thread_user.logger.as_ref() {
            logger.send(Some(GooseLog::Transaction(raw_transaction.clone())))?;
        }
    }

    // Otherwise send metrics to parent.
    if let Some(metrics_channel) = thread_user.metrics_channel.clone() {
        // Best effort metrics.
        let _ = metrics_channel.send(GooseMetric::Transaction(raw_transaction));
    }

    Ok(())
}
