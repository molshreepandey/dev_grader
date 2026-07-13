use std::env;

#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub brokers: String,
    pub group_id: String,
    pub submission_topic: String,
    pub result_topic: String,
    pub concurrency: usize,
}

impl WorkerConfig {
    pub fn from_env() -> Self {
        let get = |key: &str, default: &str| env::var(key).unwrap_or_else(|_| default.to_string());
        WorkerConfig {
            brokers: get("KAFKA_BROKERS", "62.171.180.143:29092"),
            group_id: get("KAFKA_GROUP_ID", "grader-workers"),
            submission_topic: get("SUBMISSION_TOPIC", "grader-submission"),
            result_topic: get("RESULT_TOPIC", "grader-result"),
            concurrency: get("GRADER_CONCURRENCY", "1").parse().unwrap_or(1),
        }
    }
}
