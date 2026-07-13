//! Run a single grading task straight from a JSON payload, with no Kafka in the
//! loop. This is the debugging counterpart of the worker: same pipeline, same
//! sandbox, result printed to stdout.
//!
//!   cargo run -p grader --bin grade_once -- '{"task_id":"t1", …}'
//!   cargo run -p grader --bin grade_once -- @payload.json

use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use grader_lib::config::GraderConfig;
use grader_lib::scheduler::grade_task;
use task_types::consumer::TaskPayload;
use task_types::producer::ResponsePayload;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let Some(arg) = std::env::args().nth(1) else {
        bail!("usage: grade_once '<payload json>' | grade_once @payload.json");
    };
    let raw = match arg.strip_prefix('@') {
        Some(path) => std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?,
        None => arg,
    };
    let payload: TaskPayload = serde_json::from_str(&raw).context("parsing the task payload")?;

    let config = Arc::new(GraderConfig::from_env());
    let started = Instant::now();

    let response = match grade_task(&payload, &config).await {
        Ok(cases) => ResponsePayload::graded(
            payload.task_id.clone(),
            payload.student_id.clone(),
            cases,
            started.elapsed().as_millis() as u64,
        ),
        Err(e) => ResponsePayload::failure(
            payload.task_id.clone(),
            payload.student_id.clone(),
            e.status,
            e.stage,
            e.message,
            e.logs,
            started.elapsed().as_millis() as u64,
        ),
    };

    println!("{}", serde_json::to_string_pretty(&response)?);
    // Non-zero exit when the task never reached a verdict, so a shell caller can
    // tell "infrastructure broke" from "the student failed".
    if !response.status.is_graded() {
        std::process::exit(1);
    }
    Ok(())
}
