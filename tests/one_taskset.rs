use httpmock::Method::GET;
use httpmock::{Mock, MockServer};

mod common;

use goose::goose::GooseMethod;
use goose::prelude::*;
use std::sync::Arc;

const INDEX_PATH: &str = "/";
const ABOUT_PATH: &str = "/about.html";

pub async fn get_index(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

pub async fn get_about(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(ABOUT_PATH).await?;
    Ok(())
}

#[test]
// Load test with a single task set containing two weighted tasks. Validate
// weighting and statistics.
fn test_single_taskset() {
    let server = MockServer::start();

    let index = Mock::new()
        .expect_method(GET)
        .expect_path(INDEX_PATH)
        .return_status(200)
        .create_on(&server);
    let about = Mock::new()
        .expect_method(GET)
        .expect_path(ABOUT_PATH)
        .return_status(200)
        .create_on(&server);

    let mut config = common::build_configuration(&server);
    config.no_metrics = false;
    // Start users in .5 seconds.
    config.users = Some(2);
    config.hatch_rate = 4;
    config.status_codes = true;
    config.no_reset_metrics = true;
    let goose_metrics = crate::GooseAttack::initialize_with_config(config.clone())
        .unwrap()
        .setup()
        .unwrap()
        .register_taskset(
            taskset!("LoadTest")
                .register_task(task!(get_index).set_weight(9).unwrap())
                .register_task(task!(get_about).set_weight(3).unwrap()),
        )
        .execute()
        .unwrap();

    // Confirm that we loaded the mock endpoints.
    assert!(index.times_called() > 0);
    assert!(about.times_called() > 0);

    // Confirm that we loaded the index roughly three times as much as the about page.
    let one_third_index = index.times_called() / 3;
    let difference = about.times_called() as i32 - one_third_index as i32;
    assert!(difference >= -2 && difference <= 2);

    let index_metrics = goose_metrics
        .requests
        .get(&format!("GET {}", INDEX_PATH))
        .unwrap();
    let about_metrics = goose_metrics
        .requests
        .get(&format!("GET {}", ABOUT_PATH))
        .unwrap();

    // Confirm that the path and method are correct in the statistics.
    assert!(index_metrics.path == INDEX_PATH);
    assert!(index_metrics.method == GooseMethod::GET);
    assert!(about_metrics.path == ABOUT_PATH);
    assert!(about_metrics.method == GooseMethod::GET);

    // Confirm that Goose and the server saw the same number of page loads.
    let status_code: u16 = 200;
    assert!(index_metrics.response_time_counter == index.times_called());
    assert!(index_metrics.status_code_counts[&status_code] == index.times_called());
    assert!(index_metrics.success_count == index.times_called());
    assert!(index_metrics.fail_count == 0);
    assert!(about_metrics.response_time_counter == about.times_called());
    assert!(about_metrics.status_code_counts[&status_code] == about.times_called());
    assert!(about_metrics.success_count == about.times_called());
    assert!(about_metrics.fail_count == 0);

    // Verify that Goose started the correct number of users.
    assert!(goose_metrics.users == config.users.unwrap());
}

#[test]
// Load test with a single task set containing two weighted tasks. Validate
// that setting the host in the load test is properly recognized, and doesn't
// otherwise affect the test.
fn test_single_taskset_empty_config_host() {
    let server = MockServer::start();

    let index = Mock::new()
        .expect_method(GET)
        .expect_path(INDEX_PATH)
        .return_status(200)
        .create_on(&server);
    let about = Mock::new()
        .expect_method(GET)
        .expect_path(ABOUT_PATH)
        .return_status(200)
        .create_on(&server);

    let mut config = common::build_configuration(&server);
    // Leaves an empty string in config.host.
    let host = std::mem::take(&mut config.host);
    // Enable statistics to confirm Goose and web server agree.
    config.no_metrics = false;
    config.no_reset_metrics = true;
    let goose_metrics = crate::GooseAttack::initialize_with_config(config)
        .unwrap()
        .setup()
        .unwrap()
        .register_taskset(
            taskset!("LoadTest")
                .register_task(task!(get_index).set_weight(9).unwrap())
                .register_task(task!(get_about).set_weight(3).unwrap()),
        )
        .set_host(&host)
        .execute()
        .unwrap();

    // Confirm that we loaded the mock endpoints.
    assert!(index.times_called() > 0);
    assert!(about.times_called() > 0);

    // Confirm that we loaded the index roughly three times as much as the about page.
    let one_third_index = index.times_called() / 3;
    let difference = about.times_called() as i32 - one_third_index as i32;
    assert!(difference >= -2 && difference <= 2);

    // Confirm that Goose and the server saw the same number of page loads.
    assert!(
        goose_metrics
            .requests
            .get(&format!("GET {}", INDEX_PATH))
            .unwrap()
            .response_time_counter
            == index.times_called()
    );
    assert!(
        goose_metrics
            .requests
            .get(&format!("GET {}", ABOUT_PATH))
            .unwrap()
            .response_time_counter
            == about.times_called()
    );
}

#[test]
// Load test with a single task set containing two weighted tasks setup via closure.
// Validate weighting and statistics.
fn test_single_taskset_closure() {
    // @todo Move out to common.rs.
    #[derive(Debug)]
    struct LoadtestEndpoint<'a> {
        pub path: &'a str,
        pub status_code: u16,
        pub weight: usize,
    };

    // Configure endpoints to test.
    let test_endpoints = vec![
        LoadtestEndpoint {
            path: INDEX_PATH,
            status_code: 200,
            weight: 9,
        },
        LoadtestEndpoint {
            path: ABOUT_PATH,
            status_code: 200,
            weight: 3,
        },
    ];

    // Start mock server.
    let server = MockServer::start();

    // Build configuration.
    let mut config = common::build_configuration(&server);
    config.no_metrics = false;
    // Start users in .5 seconds.
    config.users = Some(test_endpoints.len());
    config.hatch_rate = 2 * test_endpoints.len();
    config.status_codes = true;
    config.no_reset_metrics = true;

    // Setup mock endpoints.
    let mut mock_endpoints = Vec::with_capacity(test_endpoints.len());
    for (idx, item) in test_endpoints.iter().enumerate() {
        let path = item.path;
        let mock_endpoint = Mock::new()
            .expect_method(GET)
            .expect_path(path)
            .return_status(item.status_code.into())
            .create_on(&server);

        // Ensure the index matches.
        assert!(idx == mock_endpoints.len());
        mock_endpoints.push(mock_endpoint);
    }

    // Build dynamic taskset.
    let mut taskset = GooseTaskSet::new("LoadTest");
    for item in &test_endpoints {
        let path = item.path;
        let weight = item.weight;

        let closure: GooseTaskFunction = Arc::new(move |user| {
            Box::pin(async move {
                let _goose = user.get(path).await?;

                Ok(())
            })
        });

        let task = GooseTask::new(closure).set_weight(weight).unwrap();
        // We need to do the variable dance as taskset.register_task returns self and hence moves
        // self out of `taskset`. By storing it in a new local variable and then moving it over
        // we can avoid that error.
        let new_taskset = taskset.register_task(task);
        taskset = new_taskset;
    }

    // Run the loadtest.
    let goose_metrics = crate::GooseAttack::initialize_with_config(config.clone())
        .unwrap()
        .setup()
        .unwrap()
        .register_taskset(taskset)
        .execute()
        .unwrap();

    // Ensure that the right paths have been called.
    for (idx, item) in test_endpoints.iter().enumerate() {
        let mock_endpoint = &mock_endpoints[idx];

        let format_item = |message, assert_item| {
            return format!("{} for item = {:#?}", message, assert_item);
        };

        // Confirm that we loaded the mock endpoint.
        assert!(
            mock_endpoint.times_called() > 0,
            format_item("Endpoint was not called > 0", &item)
        );
        let expect_error = format_item("Item does not exist in goose_metrics", &item);
        let endpoint_metrics = goose_metrics
            .requests
            .get(&format!("GET {}", item.path))
            .expect(&expect_error);

        assert!(
            endpoint_metrics.path == item.path,
            format_item(
                &format!("{} != {}", endpoint_metrics.path, item.path),
                &item
            )
        );
        assert!(endpoint_metrics.method == GooseMethod::GET);

        // Confirm that Goose and the server saw the same number of page loads.
        let status_code: u16 = item.status_code;

        assert!(
            endpoint_metrics.response_time_counter == mock_endpoint.times_called(),
            format_item("response_time_counter != times_called()", &item)
        );
        assert!(
            endpoint_metrics.status_code_counts[&status_code] == mock_endpoint.times_called(),
            format_item("status_code_counts != times_called()", &item)
        );
        assert!(
            endpoint_metrics.success_count == mock_endpoint.times_called(),
            format_item("success_count != times_called()", &item)
        );
        assert!(
            endpoint_metrics.fail_count == 0,
            format_item("fail_count != 0", &item)
        );
    }

    // Test specific things directly access the mock endpoints here.
    let index = &mock_endpoints[0];
    let about = &mock_endpoints[1];

    // Confirm that we loaded the index roughly three times as much as the about page.
    let one_third_index = index.times_called() / 3;
    let difference = about.times_called() as i32 - one_third_index as i32;
    assert!(difference >= -2 && difference <= 2);

    // Verify that Goose started the correct number of users.
    assert!(goose_metrics.users == config.users.unwrap());
}

#[test]
// Load test with a single task set containing two weighted tasks. Validate
// weighting and statistics after resetting metrics.
fn test_single_taskset_reset_metrics() {
    let server = MockServer::start();

    let index = Mock::new()
        .expect_method(GET)
        .expect_path(INDEX_PATH)
        .return_status(200)
        .create_on(&server);
    let about = Mock::new()
        .expect_method(GET)
        .expect_path(ABOUT_PATH)
        .return_status(200)
        .create_on(&server);

    let mut config = common::build_configuration(&server);
    config.no_metrics = false;
    // Start users in .5 seconds.
    config.users = Some(2);
    config.hatch_rate = 4;
    config.status_codes = true;
    let goose_metrics = crate::GooseAttack::initialize_with_config(config.clone())
        .unwrap()
        .setup()
        .unwrap()
        .register_taskset(
            taskset!("LoadTest")
                .register_task(task!(get_index).set_weight(9).unwrap())
                .register_task(task!(get_about).set_weight(3).unwrap()),
        )
        .execute()
        .unwrap();

    // Confirm that we loaded the mock endpoints.
    assert!(index.times_called() > 0);
    assert!(about.times_called() > 0);

    // Confirm that we loaded the index roughly three times as much as the about page.
    let one_third_index = index.times_called() / 3;
    let difference = about.times_called() as i32 - one_third_index as i32;
    assert!(difference >= -2 && difference <= 2);

    let index_metrics = goose_metrics
        .requests
        .get(&format!("GET {}", INDEX_PATH))
        .unwrap();
    let about_metrics = goose_metrics
        .requests
        .get(&format!("GET {}", ABOUT_PATH))
        .unwrap();

    // Confirm that the path and method are correct in the statistics.
    assert!(index_metrics.path == INDEX_PATH);
    assert!(index_metrics.method == GooseMethod::GET);
    assert!(about_metrics.path == ABOUT_PATH);
    assert!(about_metrics.method == GooseMethod::GET);

    // Confirm that Goose saw fewer page loads than the server, as the statistics
    // were reset after .5 seconds.
    let status_code: u16 = 200;
    assert!(index_metrics.response_time_counter < index.times_called());
    assert!(index_metrics.status_code_counts[&status_code] < index.times_called());
    assert!(index_metrics.success_count < index.times_called());
    assert!(index_metrics.fail_count == 0);
    assert!(about_metrics.response_time_counter < about.times_called());
    assert!(about_metrics.status_code_counts[&status_code] < about.times_called());
    assert!(about_metrics.success_count < about.times_called());
    assert!(about_metrics.fail_count == 0);

    // Verify that Goose started the correct number of users.
    assert!(goose_metrics.users == config.users.unwrap());
}
