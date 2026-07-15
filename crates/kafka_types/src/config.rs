#[derive(Debug, Clone)]
pub struct ConsumerConfig {
    pub kafka_url: String,
    pub group_id: String,
    pub auto_commit: bool,
    pub auto_offset_reset: String,
}
impl ConsumerConfig {
    pub fn configure(
        kafka_url: String,
        group_id: String,
        auto_commit: bool,
        auto_offset_reset: String,
    ) -> Self {
        Self {
            kafka_url,
            group_id,
            auto_commit,
            auto_offset_reset,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProducerConfig {
    pub kafka_url: String,
    pub queue_wait_time: String,
    pub timeout_limit: String,
}

impl ProducerConfig {
    pub fn configure(kafka_url: String, queue_wait_time: String, timeout_limit: String) -> Self {
        Self {
            kafka_url,
            queue_wait_time,
            timeout_limit,
        }
    }
}
