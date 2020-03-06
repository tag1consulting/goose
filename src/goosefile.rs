/// @TODO:
///  - compile the goosefile as a dynamic binary included at run-time
///  - provide tools for goose to compile goosefiles
///  - ultimately load-tests are shipped with two compiled binaries:
///      o the main goose binary (pre-compiled)
///      o the goosefile dynamic binary (compiled with a goose helper)

// @TODO: this needs to be entirely provided by goose or goose_codegen
trait TaskSet {
    // @TODO: this needs to be useful
}

#[derive(TaskSet)]
struct WebsiteTasks {}

impl WebsiteTasks {
    #[task]
    fn on_start(&self) -> Result<(), Box<dyn std::error::Error>> {
        let params = [("username", "test_user"), ("password", "secure_example")];
        let client = reqwest::Client::new();
        let res = client.post("/login")
            .form(&params)
            .send()?;
        Ok(())
    }

    #[task]
    fn index(&self) -> Result<(), Box<dyn std::error::Error>> {
        let resp = reqwest::blocking::get("/");
        println!("{:#?}", resp);
        Ok(())
    }

    #[task]
    fn about(&self) {
        let resp = reqwest::blocking::get("/about/");
        println!("{:#?}", resp);
        Ok(())
    }
}

/*
class WebsiteUser(HttpLocust):
    task_set = WebsiteTasks
    wait_time = between(5, 15)
*/