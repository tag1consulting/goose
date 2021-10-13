use rand::Rng;
use std::time::{self, Duration};

use crate::get_worker_id;
use crate::goose::{GooseTaskFunction, GooseTaskSet, GooseUser, GooseUserCommand};
use crate::logger::GooseLog;
use crate::metrics::{GooseMetric, GooseTaskMetric};

pub(crate) async fn user_main(
    thread_number: usize,
    thread_task_set: GooseTaskSet,
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
        // When there is a delay between tasks, wake every second to check for messages.
        let one_second = Duration::from_secs(1);

        'launch_tasks: loop {
            // Tracks the time it takes to loop through all GooseTasks when Coordinated Omission
            // Mitigation is enabled.
            thread_user.update_request_cadence(thread_number).await;

            for (thread_task_index, thread_task_name) in &thread_task_set.weighted_tasks {
                // Determine which task we're going to run next.
                let function = &thread_task_set.tasks[*thread_task_index].function;
                debug!(
                    "launching on_start {} task from {}",
                    thread_task_name, thread_task_set.name
                );
                // Invoke the task function.
                let _todo = invoke_task_function(
                    function,
                    &mut thread_user,
                    *thread_task_index,
                    thread_task_name,
                )
                .await;

                if received_exit(&thread_receiver) {
                    break 'launch_tasks;
                }

                // If the task_wait is defined, wait for a random time between tasks.
                if let Some((min, max)) = thread_task_set.task_wait {
                    let wait_time = rand::thread_rng().gen_range(min..max).as_millis();
                    // Counter to track how long we've slept, waking regularly to check for messages.
                    let mut slept: u128 = 0;
                    // Wake every second to check if the parent thread has told us to exit.
                    let mut in_sleep_loop = true;
                    // Track the time slept for Coordinated Omission Mitigation.
                    let sleep_timer = time::Instant::now();

                    while in_sleep_loop {
                        if received_exit(&thread_receiver) {
                            break 'launch_tasks;
                        }

                        let sleep_duration = if wait_time - slept >= 1000 {
                            slept += 1000;
                            if slept >= wait_time {
                                // Break out of sleep loop after next sleep.
                                in_sleep_loop = false;
                            }
                            one_second
                        } else {
                            slept += wait_time;
                            // Break out of sleep loop after next sleep.
                            in_sleep_loop = false;
                            Duration::from_millis((wait_time - slept) as u64)
                        };

                        debug!(
                            "user {} from {} sleeping {:?} ...",
                            thread_number, thread_task_set.name, sleep_duration
                        );

                        tokio::time::sleep(sleep_duration).await;
                    }
                    // Track how much time the GooseUser sleeps during this loop through all GooseTasks,
                    // used by Coordinated Omission Mitigation.
                    thread_user.slept += (time::Instant::now() - sleep_timer).as_millis() as u64;
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
