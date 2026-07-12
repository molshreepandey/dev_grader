//! Kafka worker: consume `Submission` events, grade each in a sandbox, produce `GradeResult`.
//!
//! Grading is synchronous and heavy (fetch + build + sandboxed test run), so each submission is
//! graded on a blocking thread with a concurrency cap, keeping the async consumer responsive.

use std::sync::Arc;
use std::time::Duration;

use grader_engine::{Engine, FsAssignmentStore, HttpRepoFetcher, SandboxProjectRunner};
use grader_types::{GradeResult, GradeStatus, Submission};
use rdkafka::consumer::{CommitMode, Consumer, StreamConsumer};
use rdkafka::message::{Message, OwnedMessage};
use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::util::Timeout;
use rdkafka::{ClientConfig, Offset, TopicPartitionList};
use tokio::sync::Semaphore;
use tracing::{error, info, warn};

use crate::config::WorkerConfig;

type GraderEngine = Engine<FsAssignmentStore, HttpRepoFetcher, SandboxProjectRunner>;

/// Run the worker until the process is stopped.
pub async fn run(cfg: WorkerConfig) -> anyhow::Result<()> {
    let consumer: StreamConsumer = ClientConfig::new()
        .set("bootstrap.servers", &cfg.brokers)
        .set("group.id", &cfg.group_id)
        .set("enable.auto.commit", "false")
        .set("auto.offset.reset", "earliest")
        .create()?;
    consumer.subscribe(&[cfg.submission_topic.as_str()])?;
    let consumer = Arc::new(consumer);

    let producer: FutureProducer = ClientConfig::new()
        .set("bootstrap.servers", &cfg.brokers)
        .create()?;

    let engine = Arc::new(build_engine(&cfg));
    let limiter = Arc::new(Semaphore::new(cfg.concurrency.max(1)));
    let result_topic = Arc::new(cfg.result_topic.clone());

    info!(
        brokers = %cfg.brokers,
        submissions = %cfg.submission_topic,
        results = %cfg.result_topic,
        concurrency = cfg.concurrency,
        "grader worker started"
    );

    loop {
        let msg = match consumer.recv().await {
            Ok(m) => m.detach(),
            Err(e) => {
                error!("kafka recv error: {e}");
                continue;
            }
        };

        let permit = limiter
            .clone()
            .acquire_owned()
            .await
            .expect("semaphore closed");
        let engine = engine.clone();
        let producer = producer.clone();
        let consumer = consumer.clone();
        let result_topic = result_topic.clone();

        tokio::spawn(async move {
            let _permit = permit; // released when the task ends
            handle_message(engine, producer, consumer, &result_topic, msg).await;
        });
    }
}

async fn handle_message(
    engine: Arc<GraderEngine>,
    producer: FutureProducer,
    consumer: Arc<StreamConsumer>,
    result_topic: &str,
    msg: OwnedMessage,
) {
    let Some(payload) = msg.payload() else {
        warn!("empty message payload; skipping");
        commit(&consumer, &msg);
        return;
    };

    let submission: Submission = match serde_json::from_slice(payload) {
        Ok(s) => s,
        Err(e) => {
            error!("invalid submission payload: {e}");
            commit(&consumer, &msg);
            return;
        }
    };
    let id = submission.submission_id.clone();

    let result = match tokio::task::spawn_blocking(move || engine.grade(&submission)).await {
        Ok(r) => r,
        Err(e) => {
            error!("grade task panicked for {id}: {e}");
            GradeResult::failed(id, GradeStatus::InternalError, "grader panicked")
        }
    };

    produce_result(&producer, result_topic, &result).await;
    commit(&consumer, &msg);
}

async fn produce_result(producer: &FutureProducer, topic: &str, result: &GradeResult) {
    let payload = serde_json::to_string(result).unwrap_or_else(|_| "{}".to_string());
    let record = FutureRecord::to(topic)
        .key(&result.submission_id)
        .payload(&payload);
    match producer
        .send(record, Timeout::After(Duration::from_secs(5)))
        .await
    {
        Ok(delivery) => info!(
            "result {} ({:?}) -> partition {} offset {}",
            result.submission_id, result.status, delivery.partition, delivery.offset
        ),
        Err((e, _)) => error!("failed to produce result for {}: {e}", result.submission_id),
    }
}

fn build_engine(cfg: &WorkerConfig) -> GraderEngine {
    Engine::new(
        FsAssignmentStore::new(&cfg.assignments_root),
        HttpRepoFetcher::new(),
        SandboxProjectRunner::new(&cfg.rootfs_base),
        cfg.work_root.clone(),
    )
}

/// Commit the message's offset (we use manual commits so a crash re-delivers un-acked work).
fn commit(consumer: &StreamConsumer, msg: &OwnedMessage) {
    let mut tpl = TopicPartitionList::new();
    if let Err(e) =
        tpl.add_partition_offset(msg.topic(), msg.partition(), Offset::Offset(msg.offset() + 1))
    {
        error!("failed to build offset list: {e}");
        return;
    }
    if let Err(e) = consumer.commit(&tpl, CommitMode::Async) {
        error!("failed to commit offset: {e}");
    }
}
