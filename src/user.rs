use futures::future::Fuse;
use futures::{pin_mut, select, FutureExt};
use rand::Rng;
use std::time::{self, Duration, Instant};

use crate::get_worker_id;
use crate::goose::{GooseTaskFunction, GooseTaskSet, GooseUser, GooseUserCommand};
use crate::logger::GooseLog;
use crate::metrics::{GooseMetric, GooseTaskMetric};

pub(crate) async fn user_main(
    thread_number: usize,
    mut thread_task_set: GooseTaskSet,
    mut thread_user: GooseUser,
    thread_receiver: flume::Receiver<GooseUserCommand>,
    worker: bool,
) {
    if worker {
        info!(
            "[{}] launching user {} from {}...",
            get_worker_id(),
            thread_number,
            thread_task_set.name
        );
    } else {
        info!(
            "launching user {} from {}...",
            thread_number, thread_task_set.name
        );
    }

    // User is starting, first invoke the weighted on_start tasks.
    if !thread_task_set.weighted_on_start_tasks.is_empty() {
        // Tasks are already weighted and scheduled, execute each in order.
        for (thread_task_index, thread_task_name) in &thread_task_set.weighted_on_start_tasks {
            // Determine which task we're going to run next.
            let function = &thread_task_set.tasks[*thread_task_index].function;
            debug!(
                "[user {}]: launching on_start {} task from {}",
                thread_number, thread_task_name, thread_task_set.name
            );
            // Invoke the task function.
            let _todo = invoke_task_function(
                function,
                &mut thread_user,
                *thread_task_index,
                thread_task_name,
            )
            .await;
        }
    }

    // If normal tasks are defined, loop launching tasks until parent tells us to stop.
    if !thread_task_set.weighted_tasks.is_empty() {
        let mut task_iter = thread_task_set.weighted_tasks.iter().cycle();
        let next_task_delay = Fuse::terminated();
        pin_mut!(next_task_delay);

        let task_wait = match thread_task_set.task_wait.take() {
            Some((min, max)) if min == max => min,
            Some((min, max)) => Duration::from_millis(
                rand::thread_rng().gen_range(min.as_millis()..max.as_millis()) as u64,
            ),
            None => Duration::from_millis(0),
        };

        next_task_delay.set(tokio::time::sleep(Duration::from_secs(0)).fuse());
        loop {
            select! {
                _ = next_task_delay => {
                    let (thread_task_index, thread_task_name) = task_iter.next().unwrap();
                    if *thread_task_index == 0 {
                        // Tracks the time it takes to loop through all GooseTasks when Coordinated Omission
                        // Mitigation is enabled.
                        thread_user.update_request_cadence(thread_number).await;
                    }

                    // Get a reference to the task function we're going to invoke next.
                    let function = &thread_task_set.tasks[*thread_task_index].function;
                    debug!(
                        "launching on_start {} task from {}",
                        thread_task_name, thread_task_set.name
                    );

                    let now = Instant::now();
                    // Invoke the task function.
                    let _ = invoke_task_function(
                                    function,
                                    &mut thread_user,
                                    *thread_task_index,
                                    thread_task_name,
                                )
                                .await;

                    let elapsed = now.elapsed();

                    if elapsed < task_wait {
                        next_task_delay.set(tokio::time::sleep(task_wait - elapsed).fuse());
                    } else {
                        next_task_delay.set(tokio::time::sleep(Duration::from_millis(0)).fuse());
                    }
                },
                message = thread_receiver.recv_async().fuse() => {
                    match message {
                        // Time to exit, break out of launch_tasks loop.
                        Err(_) | Ok(GooseUserCommand::Exit) => {
                            break ;
                        }
                        Ok(command) => {
                            debug!("ignoring unexpected GooseUserCommand: {:?}", command);
                        }
                    }
                }
            }
        }
    }

    // User is exiting, first invoke the weighted on_stop tasks.
    if !thread_task_set.weighted_on_stop_tasks.is_empty() {
        // Tasks are already weighted and scheduled, execute each in order.
        for (thread_task_index, thread_task_name) in &thread_task_set.weighted_on_stop_tasks {
            // Determine which task we're going to run next.
            let function = &thread_task_set.tasks[*thread_task_index].function;
            debug!(
                "[user: {}]: launching on_stop {} task from {}",
                thread_number, thread_task_name, thread_task_set.name
            );
            // Invoke the task function.
            let _todo = invoke_task_function(
                function,
                &mut thread_user,
                *thread_task_index,
                thread_task_name,
            )
            .await;
        }
    }

    // Optional debug output when exiting.
    if worker {
        info!(
            "[{}] exiting user {} from {}...",
            get_worker_id(),
            thread_number,
            thread_task_set.name
        );
    } else {
        info!(
            "exiting user {} from {}...",
            thread_number, thread_task_set.name
        );
    }
}

// Invoke the task function, collecting task metrics.
async fn invoke_task_function(
    function: &GooseTaskFunction,
    thread_user: &mut GooseUser,
    thread_task_index: usize,
    thread_task_name: &str,
) -> Result<(), flume::SendError<Option<GooseLog>>> {
    let started = time::Instant::now();
    let mut raw_task = GooseTaskMetric::new(
        thread_user.started.elapsed().as_millis(),
        thread_user.task_sets_index,
        thread_task_index,
        thread_task_name.to_string(),
        thread_user.weighted_users_index,
    );
    if !thread_task_name.is_empty() {
        thread_user.task_name.replace(thread_task_name.to_string());
    } else {
        thread_user.task_name.take();
    }

    let success = function(thread_user).await.is_ok();
    raw_task.set_time(started.elapsed().as_millis(), success);

    // Exit if all metrics or task metrics are disabled.
    if thread_user.config.no_metrics || thread_user.config.no_task_metrics {
        return Ok(());
    }

    // If tasks-file is enabled, send a copy of the raw task metric to the logger thread.
    if !thread_user.config.task_log.is_empty() {
        if let Some(logger) = thread_user.logger.as_ref() {
            logger.send(Some(GooseLog::Task(raw_task.clone())))?;
        }
    }

    // Otherwise send metrics to parent.
    if let Some(parent) = thread_user.channel_to_parent.clone() {
        // Best effort metrics.
        let _ = parent.send(GooseMetric::Task(raw_task));
    }

    Ok(())
}
