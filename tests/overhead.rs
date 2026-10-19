//! Per-call overhead of tracking. Ignored by default; run with
//! `cargo test --release --test overhead -- --ignored`, then read
//! `target/tmp/overhead.txt`.

mod common;

use std::time::Instant;

use common::*;
use serde_json::json;

async fn time_calls(client: &Client, n: usize) -> f64 {
    // Steady state: the agent echoes its session ID, as after its first call.
    let session = "ses_2cOHEO0LYGADMzRvWTXXVbbgxgm";
    let start = Instant::now();
    for i in 0..n {
        call(
            client,
            "echo",
            json!({"text": format!("item {i}"), "context": "Measuring overhead.", "session_id": session}),
        )
        .await
        .unwrap();
    }
    start.elapsed().as_secs_f64() * 1e6 / n as f64
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn tracking_overhead_per_call() {
    const N: usize = 2000;
    let plain = connect(Todo::new()).await;
    let _ = time_calls(&plain, 200).await;
    let plain_us = time_calls(&plain, N).await;

    let agentcat = builder("overhead").build();
    let tracked = connect(agentcat.track(Todo::new())).await;
    let _ = tracked.list_all_tools().await.unwrap();
    let _ = time_calls(&tracked, 200).await;
    let tracked_us = time_calls(&tracked, N).await;

    let report = format!(
        "untracked: {plain_us:.1} µs/call, tracked: {tracked_us:.1} µs/call, overhead: {:.1} µs/call\n",
        tracked_us - plain_us
    );
    std::fs::write(
        std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("overhead.txt"),
        report,
    )
    .unwrap();
    assert!(agentcat::flush(std::time::Duration::from_secs(30)).await);
}
