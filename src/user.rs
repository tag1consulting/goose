use rand::Rng;
use std::sync::atomic::Ordering;
use std::time;

use crate::get_worker_id;
use crate::goose::{GooseTaskFunction, GooseTaskSet, GooseUser, GooseUserCommand};
use crate::metrics::{GooseMetric, GooseRawTask};

pub async fn user_main(
    thread_number: usize,
    thread_task_set: GooseTaskSet,
    thread_user: GooseUser,
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
    if !thread_user.weighted_on_start_tasks.is_empty() {
        // Tasks are already weighted and scheduled, execute each in order.
        for (thread_task_index, thread_task_name) in &thread_user.weighted_on_start_tasks {
            // Determine which task we're going to run next.
            let function = &thread_task_set.tasks[*thread_task_index].function;
            debug!(
                "[user {}]: launching on_start {} task from {}",
                thread_number, thread_task_name, thread_task_set.name
            );
            // Invoke the task function.
            invoke_task_function(function, &thread_user, *thread_task_index, thread_task_name)
                .await;
        }
    }

    // If normal tasks are defined, loop launching tasks until parent tells us to stop.
    if !thread_user.weighted_tasks.is_empty() {
        let mut position;
        'launch_tasks: loop {
            // Start at the first task in thread_user.weighted_tasks.
            position = 0;
            thread_user.position.store(position, Ordering::SeqCst);
            for (thread_task_index, thread_task_name) in &thread_user.weighted_tasks {
                // Determine which task we're going to run next.
                let function = &thread_task_set.tasks[*thread_task_index].function;
                debug!(
                    "launching on_start {} task from {}",
                    thread_task_name, thread_task_set.name
                );
                // Invoke the task function.
                invoke_task_function(function, &thread_user, *thread_task_index, thread_task_name)
                    .await;

                // Prepare to sleep for a random value from min_wait to max_wait.
                let wait_time = if thread_user.max_wait > 0 {
                    rand::thread_rng().gen_range(thread_user.min_wait..thread_user.max_wait)
                } else {
                    0
                };
                // Counter to track how long we've slept, waking regularly to check for messages.
                let mut slept: usize = 0;

                // Check if the parent thread has sent us any messages.
                let mut in_sleep_loop = true;
                while in_sleep_loop {
                    let mut message = thread_receiver.try_recv();
                    while message.is_ok() {
                        match message.unwrap() {
                            // Time to exit, break out of launch_tasks loop.
                            GooseUserCommand::Exit => {
                                break 'launch_tasks;
                            }
                            command => {
                                debug!("ignoring unexpected GooseUserCommand: {:?}", command);
                            }
                        }
                        message = thread_receiver.try_recv();
                    }
                    if thread_user.max_wait > 0 {
                        let sleep_duration = time::Duration::from_secs(1);
                        debug!(
                            "user {} from {} sleeping {:?} second...",
                            thread_number, thread_task_set.name, sleep_duration
                        );
                        tokio::time::sleep(sleep_duration).await;
                        slept += 1;
                        if slept > wait_time {
                            in_sleep_loop = false;
                        }
                    } else {
                        in_sleep_loop = false;
                    }
                }

                // Move to the next task in thread_user.weighted_tasks.
                position += 1;
                thread_user.position.store(position, Ordering::SeqCst);
            }
        }
    }

    // User is exiting, first invoke the weighted on_stop tasks.
    if !thread_user.weighted_on_stop_tasks.is_empty() {
        // Tasks are already weighted and scheduled, execute each in order.
        for (thread_task_index, thread_task_name) in &thread_user.weighted_on_stop_tasks {
            // Determine which task we're going to run next.
            let function = &thread_task_set.tasks[*thread_task_index].function;
            debug!(
                "[user: {}]: launching on_stop {} task from {}",
                thread_number, thread_task_name, thread_task_set.name
            );
            // Invoke the task function.
            invoke_task_function(function, &thread_user, *thread_task_index, thread_task_name)
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
    thread_user: &GooseUser,
    thread_task_index: usize,
    thread_task_name: &str,
) {
    let started = time::Instant::now();
    let mut raw_task = GooseRawTask::new(
        thread_user.started.elapsed().as_millis(),
        thread_user.task_sets_index,
        thread_task_index,
        thread_task_name.to_string(),
        thread_user.weighted_users_index,
    );
    let success = function(&thread_user).await.is_ok();
    raw_task.set_time(started.elapsed().as_millis(), success);

    // Exit if all metrics or task metrics are disabled.
    if thread_user.config.no_metrics || thread_user.config.no_task_metrics {
        return;
    }

    // Otherwise send metrics to parent.
    if let Some(parent) = thread_user.channel_to_parent.clone() {
        // Best effort metrics.
        let _ = parent.send(GooseMetric::Task(raw_task));
    }
}
