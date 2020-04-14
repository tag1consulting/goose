use std::collections::HashMap;
use std::sync::mpsc;

use rand::thread_rng;
use rand::seq::SliceRandom;

use crate::goose::{GooseTaskSet, GooseClient, GooseClientMode, GooseClientCommand};

pub fn client_main(
    thread_number: usize,
    thread_task_set: GooseTaskSet,
    mut thread_client: GooseClient,
    thread_receiver: mpsc::Receiver<GooseClientCommand>,
    thread_sender: mpsc::Sender<GooseClient>,
) {
    info!("launching client {} from {}...", thread_number, thread_task_set.name);
    // Notify parent that our run mode has changed to Running.
    thread_client.set_mode(GooseClientMode::RUNNING);
    thread_sender.send(thread_client.clone()).unwrap();

    // Repeatedly loop through all available tasks in a random order.
    let mut thread_continue = true;
    while thread_continue {
        // We've run through all tasks, re-shuffle and start over.
        if thread_task_set.tasks.len() <= thread_client.weighted_position {
            thread_client.weighted_tasks.shuffle(&mut thread_rng());
            debug!("re-shuffled {} tasks: {:?}", &thread_task_set.name, thread_client.weighted_tasks);
            thread_client.weighted_position = 0;
        }

        // Determine which task we're going to run next.
        let thread_weighted_task = thread_client.weighted_tasks[thread_client.weighted_position];
        let thread_task_name = &thread_task_set.tasks[thread_weighted_task].name;
        let function = thread_task_set.tasks[thread_weighted_task].function.expect(&format!("{} {} missing load testing function", thread_task_set.name, thread_task_name));
        debug!("launching {} task from {}", thread_task_name, thread_task_set.name);
        // Invoke the task function.
        function(&mut thread_client);

        // Move to the next task in thread_client.weighted_tasks.
        thread_client.weighted_position += 1;

        // Check if the parent thread has sent us any messages.
        let message = thread_receiver.try_recv();
        if message.is_ok() {
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
                    thread_sender.send(thread_client.clone()).unwrap();
                    // No need to reset per-thread counters, we're exiting and memory will be freed
                    thread_continue = false
                }
            }
        }

        // @TODO: configurable/optional delay (wait_time attribute)
    }
}