use criterion::{criterion_group, criterion_main, Criterion};
use goose::prelude::*;
use goose::GooseConfiguration;

async fn task_a(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get("/a").await?;

    Ok(())
}

async fn task_b(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get("/b").await?;

    Ok(())
}

fn create_clients_benchmark(c: &mut Criterion) {
    let config = GooseConfiguration {
        host: "http://localhost/".to_string(),
        users: Some(10),
        hatch_rate: 1,
        run_time: "".to_string(),
        no_stats: true,
        status_codes: false,
        only_summary: false,
        no_reset_stats: true,
        list: false,
        verbose: 0,
        log_level: 0,
        log_file: "goose.log".to_string(),
        stats_log_file: "".to_string(),
        stats_log_format: "json".to_string(),
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
    };
    let mut goose_attack = GooseAttack::initialize_with_config(config)
        .setup()
        .unwrap()
        .register_taskset(
            taskset!("foo")
                .register_task(task!(task_a))
                .register_task(task!(task_b)),
        );
    c.bench_function("create 10 clients", |b| {
        b.iter(|| goose_attack.weight_task_set_users())
    });
}

criterion_group!(benches, create_clients_benchmark);
criterion_main!(benches);
