/// @TODO:
///  - allow for writing complex goosefiles similar to locustfiles
///  - compile the goosefile as a dynamic binary included at run-time
///  - provide tools for goose to compile goosefiles
///  - ultimately load-tests are shipped with two compiled binaries:
///      o the main goose binary (pre-compiled)
///      o the goosefile dynamic binary (compiled with a goose helper)

trait TaskSet {
    // @TODO: on_start needs to be optional
    fn on_start(&self);
    // @TODO: macro(?) to add arbitrary tasks
}
struct WebsiteTasks {}

impl TaskSet for WebsiteTasks {
    fn on_start(&self) {
        //self.client.post("/login", {
        //    "username": "test_user",
        //    "password": ""
        //})
    }

    /*
    @task
    fn index(&self) {
        //self.client.get("/")
    }
    */

    /*
    @task
    fn about(&self) {
        //self.client.get("/about/")
    }
    */
}

/*
class WebsiteUser(HttpLocust):
    task_set = WebsiteTasks
    wait_time = between(5, 15)
*/