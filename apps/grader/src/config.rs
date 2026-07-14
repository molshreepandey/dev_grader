//! Worker configuration, read from the environment with local-dev defaults.

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub brokers: String,
    pub group_id: String,
    pub submission_topic: String,
    pub result_topic: String,
    /// Root of on-disk assignments (`<root>/<id>/{grader.json, template/}`).
    pub assignments_root: PathBuf,
    /// Base dir holding per-stack baked rootfs (`<base>/<stack>`).
    pub rootfs_base: PathBuf,
    /// Scratch dir for per-submission student/work/root dirs.
    pub work_root: PathBuf,
    /// Max submissions graded concurrently.
    pub concurrency: usize,
}

impl WorkerConfig {
    pub fn from_env() -> Self {
        let get = |key: &str, default: &str| std::env::var(key).unwrap_or_else(|_| default.to_string());
        WorkerConfig {
            brokers: get("KAFKA_BROKERS", "kafka:29092"),
            group_id: get("KAFKA_GROUP_ID", "grader-workers"),
            submission_topic: get("SUBMISSION_TOPIC", "assignment-submission"),
            result_topic: get("RESULT_TOPIC", "assignment-result"),
            assignments_root: get("ASSIGNMENTS_ROOT", "/opt/assignments").into(),
            rootfs_base: get("ROOTFS_BASE", "/opt/sandbox_rootfs").into(),
            work_root: get("WORK_ROOT", "/tmp/grader").into(),
            concurrency: get("GRADER_CONCURRENCY", "4").parse().unwrap_or(4),
        }
    }
}
