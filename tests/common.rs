use httpmock::MockServer;

use goose::GooseConfiguration;

pub fn build_configuration(server: &MockServer) -> GooseConfiguration {
    // Manually specify configuration for test, normally this is provided as
    // CLI options.
    GooseConfiguration {
        host: server.url("/"),
        users: Some(1),
        hatch_rate: 1,
        run_time: "1".to_string(),
        no_metrics: true,
        no_task_metrics: true,
        status_codes: false,
        only_summary: false,
        no_reset_metrics: false,
        list: false,
        verbose: 0,
        log_level: 0,
        log_file: "goose.log".to_string(),
        metrics_log_file: "".to_string(),
        metrics_log_format: "json".to_string(),
        debug_log_file: "".to_string(),
        debug_log_format: "json".to_string(),
        throttle_requests: None,
        sticky_follow: false,
        manager: false,
        no_hash_check: false,
        expect_workers: 0,
        manager_bind_host: "0.0.0.0".to_string(),
        manager_bind_port: 5115,
        worker: false,
        manager_host: "127.0.0.1".to_string(),
        manager_port: 5115,
    }
}
