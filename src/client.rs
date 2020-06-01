use rand::seq::SliceRandom;
use rand::thread_rng;
use rand::Rng;
use std::time;
use std::sync::atomic::Ordering;
use tokio::sync::mpsc;

use crate::{get_worker_id, CLIENT};
use crate::goose::{GooseClient, GooseClientCommand, GooseTaskSet};

pub async fn client_main(
    thread_number: usize,
    thread_task_set: GooseTaskSet,
    mut thread_client: GooseClient,
    mut thread_receiver: mpsc::UnboundedReceiver<GooseClientCommand>,
    worker: bool,
) {
    if worker {
        info!(
            "[{}] launching client {} from {}...",
            get_worker_id(),
            thread_number,
            thread_task_set.name
        );
    } else {
        info!(
            "launching client {} from {}...",
            thread_number, thread_task_set.name
        );
    }

    // Client is starting, first invoke the weighted on_start tasks.
    if !thread_client.weighted_on_start_tasks.is_empty() {
        for mut sequence in thread_client.weighted_on_start_tasks.clone() {
            if sequence.len() > 1 {
                sequence.shuffle(&mut thread_rng());
            }
            for task_index in &sequence {
                // Determine which task we're going to run next.
                let thread_task_name = &thread_task_set.tasks[*task_index].name;
                let function = &thread_task_set.tasks[*task_index].function;
                debug!(
                    "launching on_start {} task from {}",
                    thread_task_name, thread_task_set.name
                );
                if thread_task_name != "" {
                    thread_client.task_request_name = Some(thread_task_name.to_string());
                }
                // Invoke the task function.
                function(&mut thread_client).await;
            }
        }
    }

    // Repeatedly loop through all available tasks in a random order.
    let mut thread_continue: bool = true;
    let mut weighted_bucket = CLIENT.read().await[thread_client.weighted_clients_index].weighted_bucket.load(Ordering::SeqCst);
    let mut weighted_bucket_position = CLIENT.read().await[thread_client.weighted_clients_index].weighted_bucket_position.fetch_add(1, Ordering::SeqCst);
    while thread_continue {
        // Weighted_tasks is divided into buckets of tasks sorted by sequence, and then all non-sequenced tasks.
        if thread_client.weighted_tasks[weighted_bucket].len()
            <= weighted_bucket_position
        {
            // This bucket is exhausted, move on to position 0 of the next bucket.
            weighted_bucket_position = 0;
            CLIENT.read().await[thread_client.weighted_clients_index].weighted_bucket_position.store(weighted_bucket_position, Ordering::SeqCst);

            weighted_bucket += 1;
            if thread_client.weighted_tasks.len() <= weighted_bucket {
                weighted_bucket = 0;
            }
            CLIENT.read().await[thread_client.weighted_clients_index].weighted_bucket_position.store(weighted_bucket, Ordering::SeqCst);
            // Shuffle new bucket before we walk through the tasks.
            thread_client.weighted_tasks[weighted_bucket].shuffle(&mut thread_rng());
            debug!(
                "re-shuffled {} tasks: {:?}",
                &thread_task_set.name, thread_client.weighted_tasks[weighted_bucket]
            );
        }

        // Determine which task we're going to run next.
        let thread_weighted_task = thread_client.weighted_tasks[weighted_bucket]
            [weighted_bucket_position];
        let thread_task_name = &thread_task_set.tasks[thread_weighted_task].name;
        let function = &thread_task_set.tasks[thread_weighted_task].function;
        debug!(
            "launching {} task from {}",
            thread_task_name, thread_task_set.name
        );
        // If task name is set, it will be used for storing request statistics instead of the raw url.
        if thread_task_name != "" {
            thread_client.task_request_name = Some(thread_task_name.to_string());
        }
        // Invoke the task function.
        function(&mut thread_client).await;

        // Prepare to sleep for a random value from min_wait to max_wait.
        let wait_time = if thread_client.max_wait > 0 {
            rand::thread_rng().gen_range(thread_client.min_wait, thread_client.max_wait)
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
                    // Time to exit.
                    GooseClientCommand::EXIT => {
                        // No need to reset per-thread counters, we're exiting and memory will be freed
                        thread_continue = false;
                    }
                    command => {
                        debug!("ignoring unexpected GooseClientCommand: {:?}", command);
                    }
                }
                message = thread_receiver.try_recv();
            }
            if thread_continue && thread_client.max_wait > 0 {
                let sleep_duration = time::Duration::from_secs(1);
                debug!(
                    "client {} from {} sleeping {:?} second...",
                    thread_number, thread_task_set.name, sleep_duration
                );
                tokio::time::delay_for(sleep_duration).await;
                slept += 1;
                if slept > wait_time {
                    in_sleep_loop = false;
                }
            } else {
                in_sleep_loop = false;
            }
        }

        // Move to the next task in thread_client.weighted_tasks.
        weighted_bucket_position += 1;
        CLIENT.read().await[thread_client.weighted_clients_index].weighted_bucket_position.store(weighted_bucket_position, Ordering::SeqCst);
    }

    // Client is exiting, first invoke the weighted on_stop tasks.
    if !thread_client.weighted_on_stop_tasks.is_empty() {
        for mut sequence in thread_client.weighted_on_stop_tasks.clone() {
            if sequence.len() > 1 {
                sequence.shuffle(&mut thread_rng());
            }
            for task_index in &sequence {
                // Determine which task we're going to run next.
                let thread_task_name = &thread_task_set.tasks[*task_index].name;
                let function = &thread_task_set.tasks[*task_index].function;
                debug!(
                    "launching on_stop {} task from {}",
                    thread_task_name, thread_task_set.name
                );
                if thread_task_name != "" {
                    thread_client.task_request_name = Some(thread_task_name.to_string());
                }
                // Invoke the task function.
                function(&mut thread_client).await;
            }
        }
    }

    // Optional debug output when exiting.
    if worker {
        info!(
            "[{}] exiting client {} from {}...",
            get_worker_id(),
            thread_number,
            thread_task_set.name
        );
    } else {
        info!(
            "exiting client {} from {}...",
            thread_number, thread_task_set.name
        );
    }
}
