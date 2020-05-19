use std::collections::HashMap;
use std::sync::mpsc;

use rand::thread_rng;
use rand::seq::SliceRandom;
use rand::Rng;
use std::{thread, time};

use crate::get_worker_id;
use crate::goose::{GooseTaskSet, GooseClient, GooseClientMode, GooseClientCommand};

pub fn client_main(
    thread_number: usize,
    thread_task_set: GooseTaskSet,
    mut thread_client: GooseClient,
    thread_receiver: mpsc::Receiver<GooseClientCommand>,
    thread_sender: mpsc::Sender<GooseClient>,
    worker: bool,
) {
    if worker {
        info!("[{}] launching client {} from {}...", get_worker_id(), thread_number, thread_task_set.name);
    }
    else {
        info!("launching client {} from {}...", thread_number, thread_task_set.name);
    }
    // Notify parent that our run mode has changed to Running.
    thread_client.set_mode(GooseClientMode::RUNNING);
    thread_sender.send(thread_client.clone()).unwrap();

    // Client is starting, first invoke the weighted on_start tasks.
    if thread_client.weighted_on_start_tasks.len() > 0 {
        for mut sequence in thread_client.weighted_on_start_tasks.clone() {
            if sequence.len() > 1 {
                sequence.shuffle(&mut thread_rng());
            }
            for task_index in &sequence {
                // Determine which task we're going to run next.
                let thread_task_name = &thread_task_set.tasks[*task_index].name;
                let function = thread_task_set.tasks[*task_index].function;
                debug!("launching on_start {} task from {}", thread_task_name, thread_task_set.name);
                if thread_task_name != "" {
                    thread_client.task_request_name = Some(thread_task_name.to_string());
                }
                // Invoke the task function.
                function(&mut thread_client);
            }
        }
    }

    // Repeatedly loop through all available tasks in a random order.
    let mut thread_continue: bool = true;
    while thread_continue {
        // Weighted_tasks is divided into buckets of tasks sorted by sequence, and then all non-sequenced tasks.
        if thread_client.weighted_tasks[thread_client.weighted_bucket].len() <= thread_client.weighted_bucket_position {
            // This bucket is exhausted, move on to position 0 of the next bucket.
            thread_client.weighted_bucket_position = 0;
            thread_client.weighted_bucket += 1;
            if thread_client.weighted_tasks.len() <= thread_client.weighted_bucket {
                thread_client.weighted_bucket = 0;
            }
            // Shuffle new bucket before we walk through the tasks.
            thread_client.weighted_tasks[thread_client.weighted_bucket].shuffle(&mut thread_rng());
            debug!("re-shuffled {} tasks: {:?}", &thread_task_set.name, thread_client.weighted_tasks[thread_client.weighted_bucket]);
        }

        // Determine which task we're going to run next.
        let thread_weighted_task = thread_client.weighted_tasks[thread_client.weighted_bucket][thread_client.weighted_bucket_position];
        let thread_task_name = &thread_task_set.tasks[thread_weighted_task].name;
        let function = thread_task_set.tasks[thread_weighted_task].function;
        debug!("launching {} task from {}", thread_task_name, thread_task_set.name);
        // If task name is set, it will be used for storing request statistics instead of the raw url.
        if thread_task_name != "" {
            thread_client.task_request_name = Some(thread_task_name.to_string());
        }
        // Invoke the task function.
        function(&mut thread_client);

        // Prepare to sleep for a random value from min_wait to max_wait.
        let wait_time: usize;
        if thread_client.max_wait > 0 {
            wait_time = rand::thread_rng().gen_range(
                thread_client.min_wait,
                thread_client.max_wait,
            );
        }
        else {
            wait_time = 0;
        }
        // Counter to track how long we've slept, waking regularly to check for messages.
        let mut slept: usize = 0;

        // Check if the parent thread has sent us any messages.
        let mut in_sleep_loop = true;
        while in_sleep_loop {
            let mut message = thread_receiver.try_recv();
            while message.is_ok() {
                match message.unwrap() {
                    // Sync our state to the parent.
                    GooseClientCommand::SYNC => {
                        thread_sender.send(thread_client.clone()).unwrap();
                        // Reset per-thread counters, as totals have been sent to the parent
                        thread_client.requests = HashMap::new();
                    },
                    // Sync our state to the parent and then exit.
                    GooseClientCommand::EXIT => {
                        thread_client.set_mode(GooseClientMode::EXITING);
                        // No need to reset per-thread counters, we're exiting and memory will be freed
                        thread_continue = false;
                    },
                    command => {
                        debug!("ignoring unexpected GooseClientCommand: {:?}", command);
                    }
                }
                message = thread_receiver.try_recv();
            }
            if thread_continue && thread_client.max_wait > 0 {
                let sleep_duration = time::Duration::from_secs(1);
                debug!("client {} from {} sleeping {:?} second...", thread_number, thread_task_set.name, sleep_duration);
                thread::sleep(sleep_duration);
                slept += 1;
                if slept > wait_time {
                    in_sleep_loop = false;
                }
            }
            else {
                in_sleep_loop = false;
            }
        }

        // Move to the next task in thread_client.weighted_tasks.
        thread_client.weighted_bucket_position += 1;
    }

    // Client is exiting, first invoke the weighted on_stop tasks.
    if thread_client.weighted_on_stop_tasks.len() > 0 {
        for mut sequence in thread_client.weighted_on_stop_tasks.clone() {
            if sequence.len() > 1 {
                sequence.shuffle(&mut thread_rng());
            }
            for task_index in &sequence {
                // Determine which task we're going to run next.
                let thread_task_name = &thread_task_set.tasks[*task_index].name;
                let function = thread_task_set.tasks[*task_index].function;
                debug!("launching on_stop {} task from {}", thread_task_name, thread_task_set.name);
                if thread_task_name != "" {
                    thread_client.task_request_name = Some(thread_task_name.to_string());
                }
                // Invoke the task function.
                function(&mut thread_client);
            }
        }
    }

    // Do our final sync before we exit.
    thread_sender.send(thread_client.clone()).unwrap();
    if worker {
        info!("[{}] exiting client {} from {}...", get_worker_id(), thread_number, thread_task_set.name);
    }
    else {
        info!("exiting client {} from {}...", thread_number, thread_task_set.name);
    }
}