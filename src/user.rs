use rand::Rng;
use std::time::{self, Duration};

use crate::goose::{GooseUser, GooseUserCommand, Scenario, TransactionFunction, TransactionName};
use crate::logger::GooseLog;
use crate::metrics::{GooseMetric, ScenarioMetric, TransactionMetric, METRICS_BATCH_MAX_AGE};

pub(crate) async fn user_main(
    thread_number: usize,
    thread_scenario: Scenario,
    mut thread_user: GooseUser,
    thread_receiver: flume::Receiver<GooseUserCommand>,
) {
    info!(
        "[user {thread_number}]: launching user from {}",
        thread_scenario.name
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
                "[user {thread_number}]: launching on_start {} transaction from {}",
                thread_transaction_name.name_for_transaction(),
                thread_scenario.name
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
        // Flush any pending metrics from on_start transactions (they won't be combined
        // into a GooseMetric::All since on_start doesn't produce scenario metrics).
        let _ = thread_user.flush_pending_metrics();
        let _ = thread_user.flush_batch();
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
                    thread_number,
                    thread_transaction_name.name_for_transaction(),
                    thread_scenario.name
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
                    let mut wait_time = rand::rng().random_range(min..=max).as_millis();
                    // Send the batch before a wait it would outlast: it is held
                    // until something is added, so its seconds would read short.
                    if wait_time >= METRICS_BATCH_MAX_AGE.as_millis() {
                        let _ = thread_user.flush_batch_before_wait();
                    }
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
                            "[user {thread_number}]: from {} sleeping {sleep_duration:?}",
                            thread_scenario.name
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
                &mut thread_user,
                scenario_started.elapsed(),
            )
            .await;

            // Check if configured to exit after a certain number of iterations, and exit if
            // that number of iterations have run.
            if thread_user.config.iterations() > 0
                && thread_user.iterations >= thread_user.config.iterations()
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
                    "[user {thread_number}]: completed {} {pluralize} of {}",
                    thread_user.iterations, thread_scenario.name,
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
                "[user {thread_number}]: launching on_stop {} transaction from {}",
                thread_transaction_name.name_for_transaction(),
                thread_scenario.name
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

    // Flush any remaining pending metrics before the user thread exits.
    let _ = thread_user.flush_pending_metrics();
    // Flush any remaining batch data.
    let _ = thread_user.flush_batch();

    // Optional debug output when exiting.
    info!(
        "[user {thread_number}]: exiting from {}",
        thread_scenario.name
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
                debug!("ignoring unexpected GooseUserCommand: {command:?}");
            }
        }
        message = thread_receiver.try_recv();
    }
    // GooseUserCommand::Exit not received.
    false
}

// Send scenario metric to parent and logger when enabled. When batching is
// active, all metrics (request, transaction, scenario) are accumulated into
// the local batch and flushed based on size/time thresholds. When batching
// is inactive, the GooseMetric::All optimization is used.
async fn record_scenario(
    thread_scenario: &Scenario,
    thread_user: &mut GooseUser,
    run_time: Duration,
) -> Result<(), Box<flume::SendError<Option<GooseLog>>>> {
    if !thread_user.config.no_scenario_metrics && !thread_user.config.no_metrics {
        let raw_scenario = ScenarioMetric::new(
            thread_user.started.elapsed().as_millis(),
            thread_scenario.name.clone(),
            thread_user.scenarios_index,
            run_time,
            thread_user.weighted_users_index,
        );
        if thread_user.metrics_batch.is_some() {
            // Batch path: accumulate all pending metrics into the batch.
            let _ = thread_user.flush_pending_metrics();
            thread_user.accumulate_scenario(&raw_scenario);
            let _ = thread_user.maybe_flush_batch();
        } else if let Some(metrics_channel) = thread_user.metrics_channel.clone() {
            // Non-batch path: use the GooseMetric::All optimization.
            if let (Some(request), Some(transaction)) = (
                thread_user.pending_request.take(),
                thread_user.pending_transaction.take(),
            ) {
                // Save the atomic counter's key and the outcome before moving request.
                thread_user.write_counter_key(request.raw.method_label(), &request.name);
                let success = request.success;
                let _ = metrics_channel.send(GooseMetric::All {
                    request: Box::new(request),
                    transaction,
                    scenario: Box::new(raw_scenario.clone()),
                });
                // Increment at actual send time so metrics reset can't zero the counter
                // before the parent processes this request.
                thread_user.increment_counter_at_key(success);
            } else {
                // Fall back to individual sends when the pending metrics don't align.
                let _ = thread_user.flush_pending_metrics();
                let _ = metrics_channel.send(GooseMetric::Scenario(raw_scenario.clone()));
            }
        }
        // If scenario-log is enabled, send a copy of the raw scenario metric to the logger thread.
        if thread_user.config.scenario_log().is_some() {
            if let Some(logger) = thread_user.logger.as_ref() {
                logger.send(Some(GooseLog::Scenario(raw_scenario)))?;
            }
        }
    } else {
        // Scenario metrics disabled, but still flush any pending request/transaction metrics.
        let _ = thread_user.flush_pending_metrics();
    }
    Ok(())
}

// Invoke the transaction function, collecting transaction metrics.
async fn invoke_transaction_function(
    function: &TransactionFunction,
    thread_user: &mut GooseUser,
    thread_transaction_index: usize,
    thread_transaction_name: &TransactionName,
) -> Result<(), Box<flume::SendError<Option<GooseLog>>>> {
    // Flush any previously buffered transaction metric (from a prior transaction in this
    // scenario iteration). This ensures earlier transaction metrics are sent before we
    // start collecting new ones.
    thread_user.flush_pending_transaction();

    let started = time::Instant::now();
    let mut raw_transaction = TransactionMetric::new(
        thread_user.started.elapsed().as_millis(),
        thread_user.scenarios_index,
        thread_transaction_index,
        thread_transaction_name.name_for_transaction_arc(),
        thread_user.weighted_users_index,
    );
    // Store details about the currently running transaction for logging purposes.
    thread_user
        .transaction_name
        .replace(thread_transaction_name.clone());
    // The index is optional: `None` means no transaction, `Some(0)` is the first.
    thread_user.transaction_index = Some(thread_transaction_index);

    let result = function(thread_user).await;
    raw_transaction.set_duration(started.elapsed(), result.is_ok());

    // Only a failed transaction formats its error. A transaction error is
    // recorded, as a request error is, whether or not transaction metrics are.
    let error = match &result {
        Err(error) => error.transaction_log_message(),
        Ok(()) => String::new(),
    };
    if !thread_user.config.no_metrics
        && result
            .as_ref()
            .is_err_and(|error| error.is_transaction_error())
    {
        thread_user.record_transaction_error(&raw_transaction, &error)?;
    }

    // Exit if all metrics or transaction metrics are disabled.
    if thread_user.config.no_metrics || thread_user.config.no_transaction_metrics {
        // Still flush any pending request metrics even if transaction metrics are disabled.
        let _ = thread_user.flush_pending_request();
        return Ok(());
    }

    // If transaction-log is enabled, send a copy of the raw transaction metric to the logger thread.
    // A failed transaction's line says why it failed.
    if thread_user.config.transaction_log().is_some() {
        if let Some(logger) = thread_user.logger.as_ref() {
            if error.is_empty() {
                logger.send(Some(GooseLog::Transaction(raw_transaction.clone())))?;
            } else {
                logger.send(Some(GooseLog::FailedTransaction {
                    transaction: raw_transaction.clone(),
                    error,
                }))?;
            }
        }
    }

    // Buffer the transaction metric instead of sending immediately. It will be
    // combined with the pending request and scenario metrics into a single
    // GooseMetric::All message when the scenario iteration completes.
    thread_user.pending_transaction = Some(raw_transaction);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::GooseConfiguration;
    use crate::goose::{GooseMethod, Transaction, TransactionResult};
    use crate::metrics::{
        GooseMetricBatch, GooseRawRequest, GooseRequestMetric, TransactionDetail,
        METRICS_BATCH_MAX_AGE,
    };
    use crate::transaction;
    use std::sync::Arc;

    /// What a transaction that makes one request leaves behind: the request's
    /// metric, pending until the transaction ends.
    async fn one_request(user: &mut GooseUser) -> TransactionResult {
        let raw = GooseRawRequest::new(GooseMethod::Get, "http://localhost/", vec![], "");
        let detail = TransactionDetail {
            scenario_index: 0,
            scenario_name: Arc::from("waits"),
            transaction_index: Some(0),
            transaction_name: TransactionName::InheritNameByRequests(Arc::from("one")),
        };
        user.pending_request = Some(GooseRequestMetric::new(raw, detail, "/", 0, 0));
        Ok(())
    }

    /// A user that waits 2 s after its transaction sends the transaction's
    /// request before the wait, not after it.
    #[tokio::test(start_paused = true)]
    async fn batch_is_sent_before_a_wait() {
        let configuration = GooseConfiguration::parse_args_default(&Vec::<&str>::new()).unwrap();
        let mut user =
            GooseUser::single("http://localhost:8080".parse().unwrap(), &configuration).unwrap();
        let (tx, rx) = flume::unbounded();
        user.metrics_channel = Some(tx);
        user.metrics_batch = Some(GooseMetricBatch::new(0));
        let mut scenario = Scenario::new("waits")
            .register_transaction(transaction!(one_request))
            .set_wait_time(Duration::from_secs(2), Duration::from_secs(2))
            .unwrap();
        scenario.weighted_transactions = vec![(0, scenario.transactions[0].name.clone())];
        let (exit_tx, exit_rx) = flume::unbounded();

        let started = tokio::time::Instant::now();
        let user = tokio::spawn(user_main(0, scenario, user, exit_rx));
        let batch = match rx.recv_async().await.unwrap() {
            GooseMetric::Batch(batch) => batch,
            other => panic!("expected a batch, got {:?}", other),
        };
        // The wait sleeps in chunks of at most 500 ms.
        let limit = METRICS_BATCH_MAX_AGE + Duration::from_millis(500);
        assert!(
            started.elapsed() <= limit,
            "the request was sent {:?} after the transaction, more than {limit:?}",
            started.elapsed()
        );
        assert!(batch.requests.contains_key("GET /"));
        exit_tx.send(GooseUserCommand::Exit).unwrap();
        user.await.unwrap();
    }
}
