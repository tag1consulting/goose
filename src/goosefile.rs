// @TODO:
//  - make goose a cargo library
//  - create goosefile by creating a cargo-app with a goose dependency
//  - implementing a load test should then be more or less writing this goosefile
//  - loadtests are shipped as a single compiled binary

use crate::goose::{GooseTaskSets, GooseTaskSet, GooseClient, GooseTask};

impl GooseTaskSets {
    // @TODO: auto-write this function with metaprogramming helpers
    pub fn initialize_goosefile(&mut self) {
        trace!("initialize_goosefile");

        // Create and configure an anonymous user task set.
        let mut anonymous_tasks = GooseTaskSet::new("AnonymousTasks")
            // Optional, task sets with a higher weight value will be assigned to more client threads.
            .set_weight(10)
            // Optional, configure a default host for all tasks, can be overridden with CLI --host option.
            .set_host("http://apache.fosciana")
            // Optional, a random sleep value selected from low to high, randomly invoked after each task is run
            .set_wait_time(0, 5);
        anonymous_tasks.register_task(GooseTask::new().set_weight(6).set_function(GooseClient::website_task_index));
        anonymous_tasks.register_task(GooseTask::new().set_weight(9).set_function(GooseClient::website_task_story));
        anonymous_tasks.register_task(GooseTask::new().set_weight(3).set_function(GooseClient::website_task_about));
        self.register_taskset(anonymous_tasks);

        // Create and configure a logged-in user task set.
        let mut user_tasks = GooseTaskSet::new("UserTasks")
            .set_weight(4)
            .set_host("http://apache.fosciana")
            .set_wait_time(1, 3);
        // Create named task, set a sequence value so it runs before other tasks, and tell it to run only on start.
        user_tasks.register_task(GooseTask::named("user login").set_sequence(1).set_on_start().set_function(GooseClient::user_task_login));
        // Create named tasks so they are split out in the statistics, even though they use the same functions as anonymous users.
        user_tasks.register_task(GooseTask::named("user /").set_weight(10).set_function(GooseClient::website_task_index));
        user_tasks.register_task(GooseTask::named("user /story.html").set_weight(4).set_function(GooseClient::website_task_story));
        self.register_taskset(user_tasks);
    }
}

impl GooseClient {
    fn website_task_index(&mut self) {
        let _response = self.get("/");
    }

    fn website_task_story(&mut self) {
        let _response = self.get("/story.html");
    }

    fn website_task_about(&mut self) {
        let _response = self.get("/about.html");
    }

    fn user_task_login(&mut self) {
        let _response = self.post("/user/login");
    }
}
