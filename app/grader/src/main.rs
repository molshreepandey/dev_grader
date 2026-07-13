mod config;

use anyhow::{Context, Result};
use config::WorkerConfig;
use grader_lib::config::GraderConfig;
use grader_lib::scheduler::evaluate_grader_task;
use kafka_types::config::{ConsumerConfig, ProducerConfig};
use kafka_types::connection::{process_kafka_payload, setup_consumer, setup_producer};
use rdkafka::consumer::Consumer;
use std::sync::Arc;
use tokio::signal;
use tokio::sync::Semaphore;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let config = WorkerConfig::from_env();
    let grader_config = Arc::new(GraderConfig::from_env());
    info!("{:?}", config);
    info!("{:?}", grader_config);

    if !grader_config.isolation {
        warn!("GRADER_ISOLATION=false: student code will run without namespaces or cgroups");
    }

    // A leftover workspace from a hard kill would otherwise sit on the volume forever.
    reset_work_dir(&grader_config).await?;

    let kafka_consumer_config = ConsumerConfig::configure(
        config.brokers.clone(),
        config.group_id.clone(),
        false,
        "earliest".to_string(),
    );

    let kafka_producer_config = ProducerConfig::configure(
        config.brokers.clone(),
        "1000".to_string(),
        "5000".to_string(),
    );

    let grader_consumer = setup_consumer(kafka_consumer_config);
    let kafka_producer = setup_producer(kafka_producer_config);

    grader_consumer
        .subscribe(&[&config.submission_topic])
        .context("Failed to subscribe the submission topic")?;

    let arc_grader_consumer = Arc::new(grader_consumer);
    let arc_kafka_producer = Arc::new(kafka_producer);

    let task_semaphore = Arc::new(Semaphore::new(config.concurrency));

    info!(
        topic = %config.submission_topic,
        concurrency = config.concurrency,
        "grader worker ready"
    );

    loop {
        tokio::select! {
            msg = arc_grader_consumer.recv() => {
                match msg {
                    Ok(m) => {
                        let owned_kafka_message = m.detach();
                        match process_kafka_payload(&owned_kafka_message) {
                            Ok(payload) => {
                                evaluate_grader_task(
                                    payload,
                                    task_semaphore.clone(),
                                    arc_grader_consumer.clone(),
                                    owned_kafka_message,
                                    config.result_topic.clone(),
                                    arc_kafka_producer.clone(),
                                    grader_config.clone(),
                                ).await;
                            }
                            Err(e) => {
                                // Nothing to reply to — a payload we cannot parse has no
                                // task id. Commit it so it does not block the partition.
                                warn!("skipping unparseable payload: {}", e);
                                kafka_types::connection::ack_task_to_topic(
                                    arc_grader_consumer.clone(),
                                    &owned_kafka_message,
                                );
                            }
                        }
                    },
                    Err(e) => {
                        warn!("kafka receive failed: {}", e);
                    }
                }
            }
            _ = signal::ctrl_c() => {
                info!("Received Ctrl+C! Initiating graceful shutdown...");
                break;
            }
        }
    }

    // Wait for in-flight grades to produce their result and commit their offset.
    let _ = task_semaphore
        .acquire_many(config.concurrency as u32)
        .await;
    info!("all in-flight tasks finished, exiting");

    Ok(())
}

/// Recreate the work root empty. Anything still in there is from a previous
/// process that died mid-task; its Kafka offset was never committed, so the
/// task will simply be redelivered.
async fn reset_work_dir(config: &GraderConfig) -> Result<()> {
    if config.work_root.exists() {
        tokio::fs::remove_dir_all(&config.work_root)
            .await
            .with_context(|| format!("failed to clear {}", config.work_root.display()))?;
    }
    tokio::fs::create_dir_all(&config.work_root)
        .await
        .with_context(|| format!("failed to create {}", config.work_root.display()))?;
    if let Some(cache) = &config.cache_root {
        tokio::fs::create_dir_all(cache)
            .await
            .with_context(|| format!("failed to create {}", cache.display()))?;
    }
    Ok(())
}
