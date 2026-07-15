use crate::config::{ConsumerConfig, ProducerConfig};

use rdkafka::consumer::{StreamConsumer, Consumer};
use std::{ sync::Arc, time::Duration};
use rdkafka::{Message, Offset, TopicPartitionList, config::ClientConfig, message::OwnedMessage, producer::{FutureProducer, FutureRecord}};
use rdkafka::util::Timeout;
use task_types::consumer::TaskPayload;
use tracing::{error, info};
use task_types::producer::ResponsePayload;

pub fn setup_consumer(config: ConsumerConfig) -> StreamConsumer {
    ClientConfig::new()
        .set("bootstrap.servers", config.kafka_url.as_str())
        .set("group.id", config.group_id.as_str())
        .set(
            "enable.auto.commit",
            config.auto_commit.to_string().as_str(),
        )
        .set("auto.offset.reset", config.auto_offset_reset.as_str())
        .create()
        .expect("Consumer creation failed")
}

pub fn process_kafka_payload(msg: &OwnedMessage) -> Result<TaskPayload, String> {
    let payload = msg.payload_view::<str>().unwrap_or(Ok("")).unwrap_or("");
    if payload.is_empty() {
        return Err(String::from("Payload is None"));
    }
    serde_json::from_str(payload).map_err(|e| e.to_string())
}

pub fn setup_producer(config: ProducerConfig) -> FutureProducer {
    ClientConfig::new()
        .set("bootstrap.servers", config.kafka_url)
        .set("acks", "all")
        .set("enable.idempotence", "true")
        .set("queue.buffering.max.ms", config.queue_wait_time.as_str())
        .set("message.timeout.ms", config.timeout_limit.as_str())
        .create()
        .expect("Producer creation failed")
}

pub fn ack_task_to_topic(consumer: Arc<StreamConsumer>, owned_kafka_msg: &OwnedMessage) {
    let mut tpl = TopicPartitionList::new();
    // info!("kafka acking payload: {:?}", owned_kafka_msg);
    if let Err(e) = tpl.add_partition_offset(
        owned_kafka_msg.topic(),
        owned_kafka_msg.partition(),
        Offset::Offset(owned_kafka_msg.offset() + 1),
    ) {
        error!("Failed to build TopicPartitionList: {:?}", e);
    } else {
        match consumer.commit(&tpl, rdkafka::consumer::CommitMode::Async) {
            Ok(_) => info!("Successfully committed offset."),
            Err(e) => error!("Failed to commit offset: {:?}", e),
        }
    }
}

pub async fn send_kafka_response(producer: &FutureProducer, topic: &str, key: &str, payload: &ResponsePayload) {
    let serialized = match serde_json::to_string(payload) {
        Ok(res) => res,
        Err(_) => String::from("{}"),
    };
    let msg = FutureRecord::to(topic).key(key).payload(&serialized);
    match producer.send(msg, Timeout::After(Duration::from_secs(10))).await {
       Ok(delivery) => {
           info!("Successfully sent Kafka response: submission id {:?}: produced to partition {} at offset {}", key, delivery.partition, delivery.offset);
       }
       Err(e) => {
           error!("Failed to send Kafka response: {:?}", e);
       }
    }
}
