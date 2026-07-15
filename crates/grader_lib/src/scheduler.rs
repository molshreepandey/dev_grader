use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use kafka_types::connection::{ack_task_to_topic, send_kafka_response};
use rdkafka::{consumer::StreamConsumer, message::OwnedMessage, producer::FutureProducer};
use task_types::consumer::TaskPayload;
use task_types::producer::{GradeStage, GradeStatus, ResponsePayload, TestCaseResult};
use tokio::sync::Semaphore;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::config::GraderConfig;
use crate::error::{GraderError, StageResult};
use crate::junit::{collect_reports, xml_files_in};
use crate::languages::{ReportLocation, RunCommand, spec_for};
use crate::sandbox::{CACHE_DIR, SandboxOutcome, SandboxSpec, run_sandboxed};
use crate::workspace::{Workspace, prepare_project};

/// Cap on how many individual test results ride along on the result topic. The
/// counts stay exact regardless; only the detail list is trimmed.
const MAX_TESTCASE_DETAIL: usize = 500;

/// Take a concurrency permit, then run the task on its own tokio task so the
/// consumer loop keeps polling. Awaiting the permit here is the backpressure:
/// the loop stalls once `GRADER_CONCURRENCY` tasks are already grading.
pub async fn evaluate_grader_task(
    payload: TaskPayload,
    concurrency_limiter: Arc<Semaphore>,
    consumer: Arc<StreamConsumer>,
    owned_task_msg: OwnedMessage,
    kafka_producer_topic: String,
    kafka_producer: Arc<FutureProducer>,
    config: Arc<GraderConfig>,
) {
    let permit = match concurrency_limiter.acquire_owned().await {
        Ok(permit) => permit,
        Err(e) => {
            error!("concurrency semaphore closed, dropping task: {}", e);
            return;
        }
    };

    tokio::spawn(async move {
        let _permit = permit;
        let started = Instant::now();
        let task_id = payload.task_id.clone();
        let student_id = payload.student_id.clone();

        info!(
            task_id = %task_id,
            student_id = %student_id,
            language = payload.task_language.as_str(),
            "grading started"
        );

        let response = match grade_task(&payload, &config).await {
            Ok(mut cases) => {
                let mut result = ResponsePayload::graded(
                    task_id.clone(),
                    student_id.clone(),
                    std::mem::take(&mut cases),
                    started.elapsed().as_millis() as u64,
                );
                result.testcases.truncate(MAX_TESTCASE_DETAIL);
                result
            }
            Err(e) => {
                warn!(task_id = %task_id, "grading failed: {}", e);
                ResponsePayload::failure(
                    task_id.clone(),
                    student_id.clone(),
                    e.status,
                    e.stage,
                    e.message,
                    e.logs,
                    started.elapsed().as_millis() as u64,
                )
            }
        };

        info!(
            task_id = %task_id,
            status = ?response.status,
            passed = response.passed_testcases,
            failed = response.failed_testcases,
            total = response.total_testcases,
            duration_ms = response.duration_ms,
            "grading finished"
        );

        // Produce first, commit second: a crash in between replays the task,
        // which is safe (grading is idempotent). Committing first would lose it.
        send_kafka_response(
            &kafka_producer,
            &kafka_producer_topic,
            &task_id,
            &response,
        )
        .await;
        ack_task_to_topic(consumer, &owned_task_msg);
    });
}

/// clone → merge → install → test → parse. Every failure carries the stage it
/// happened in, so the result topic always says *where* it broke.
pub async fn grade_task(
    payload: &TaskPayload,
    config: &GraderConfig,
) -> StageResult<Vec<TestCaseResult>> {
    let run_id = build_run_id(&payload.task_id);
    let cache_dir = config.cache_root.clone();
    let spec = spec_for(
        payload.task_language,
        cache_dir.as_ref().map(|_| CACHE_DIR),
    );

    // Dropping the workspace wipes the whole tree, on every exit path below.
    let workspace = Workspace::create(config, &run_id).await?;

    prepare_project(
        &workspace,
        &payload.student_link,
        &payload.test_link,
        spec.purge_paths,
        config,
    )
    .await?;

    let install = run_stage(
        &workspace,
        &run_id,
        GradeStage::Install,
        &spec.install,
        &spec.env,
        config.install_timeout,
        cache_dir.clone(),
        config,
    )
    .await?;

    if !install.succeeded() {
        return Err(stage_error(
            GradeStage::Install,
            &install,
            "dependency install or compilation failed",
            config,
        ));
    }

    let test = run_stage(
        &workspace,
        &run_id,
        GradeStage::Test,
        &spec.test,
        &spec.env,
        config.test_timeout,
        cache_dir,
        config,
    )
    .await?;

    // A failing test suite exits non-zero by design, so the exit code is not
    // the verdict — the JUnit report is. Only a timeout, an OOM kill or a
    // missing report mean the run itself did not happen.
    if test.timed_out || test.oom_killed {
        return Err(stage_error(
            GradeStage::Test,
            &test,
            "the test suite did not finish",
            config,
        ));
    }

    let report_paths = resolve_reports(&workspace, &spec.report).await;
    if report_paths.is_empty() {
        return Err(GraderError::new(
            GradeStage::Report,
            GradeStatus::RunFailed,
            format!(
                "the test runner produced no JUnit report (exit code {})",
                test.exit_code
            ),
        )
        .with_logs(Some(test.logs.clone())));
    }

    let cases = collect_reports(&report_paths).await?;
    if cases.is_empty() {
        return Err(GraderError::new(
            GradeStage::Report,
            GradeStatus::RunFailed,
            "the test runner discovered no test cases".to_string(),
        )
        .with_logs(Some(test.logs)));
    }

    Ok(cases)
}

#[allow(clippy::too_many_arguments)]
async fn run_stage(
    workspace: &Workspace,
    run_id: &str,
    stage: GradeStage,
    command: &RunCommand,
    env: &[(String, String)],
    timeout: std::time::Duration,
    cache_dir: Option<PathBuf>,
    config: &GraderConfig,
) -> StageResult<SandboxOutcome> {
    let spec = SandboxSpec {
        run_id: format!("{}_{}", run_id, format!("{:?}", stage).to_lowercase()),
        stage,
        root_dir: workspace.root.clone(),
        cache_dir,
        program: command.program.clone(),
        args: command.args.clone(),
        env: env.to_vec(),
        timeout,
    };
    run_sandboxed(spec, config).await
}

/// Turn a non-zero sandbox outcome into the right status: a timeout and an OOM
/// kill are not the student's logic being wrong, and upstream reports them
/// differently.
fn stage_error(
    stage: GradeStage,
    outcome: &SandboxOutcome,
    context: &str,
    config: &GraderConfig,
) -> GraderError {
    let (status, message) = if outcome.timed_out {
        (
            GradeStatus::Timeout,
            format!(
                "{context}: exceeded the {}s limit",
                match stage {
                    GradeStage::Install => config.install_timeout.as_secs(),
                    _ => config.test_timeout.as_secs(),
                }
            ),
        )
    } else if outcome.oom_killed {
        (
            GradeStatus::MemoryLimitExceeded,
            format!(
                "{context}: exceeded the {} MB memory limit",
                config.memory_limit_mb
            ),
        )
    } else if let Some(signal) = outcome.signal {
        (
            GradeStatus::RunFailed,
            format!("{context}: killed by signal {signal}"),
        )
    } else {
        (
            match stage {
                GradeStage::Install => GradeStatus::InstallFailed,
                _ => GradeStatus::RunFailed,
            },
            format!("{context}: exit code {}", outcome.exit_code),
        )
    };

    GraderError::new(stage, status, message).with_logs(Some(outcome.logs.clone()))
}

/// Map the sandbox-visible report path back onto the host workspace.
async fn resolve_reports(workspace: &Workspace, report: &ReportLocation) -> Vec<PathBuf> {
    let to_host = |sandbox_path: &str| workspace.root.join(sandbox_path.trim_start_matches('/'));

    match report {
        ReportLocation::File(path) => {
            let host = to_host(path);
            if host.is_file() { vec![host] } else { Vec::new() }
        }
        ReportLocation::Dir(path) => xml_files_in(&to_host(path)).await,
    }
}

/// A filesystem- and cgroup-safe name. The task id comes off Kafka, so it is
/// not trusted to be a sane path component; the uuid keeps two replays of the
/// same task from colliding on disk.
fn build_run_id(task_id: &str) -> String {
    let cleaned: String = task_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(40)
        .collect();
    let short_uuid = Uuid::new_v4().simple().to_string();
    if cleaned.is_empty() {
        format!("task_{}", &short_uuid[..12])
    } else {
        format!("{}_{}", cleaned, &short_uuid[..12])
    }
}
