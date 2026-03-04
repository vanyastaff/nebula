use std::sync::Arc;
use std::time::Duration;

use nebula_action::Overflow;
use nebula_runtime::{BoundedStreamBuffer, PushOutcome};
use tokio::sync::Mutex;

#[tokio::test]
async fn bounded_buffer_block_policy_handles_slow_consumer() {
    let buffer = BoundedStreamBuffer::new(2, Overflow::Block);
    let consumed = Arc::new(Mutex::new(Vec::new()));

    let producer = {
        let buffer = buffer.clone();
        tokio::spawn(async move {
            for i in 0..5 {
                let outcome = buffer.push(i).await.expect("push should succeed");
                assert_eq!(outcome, PushOutcome::Accepted);
            }
        })
    };

    let consumer = {
        let buffer = buffer.clone();
        let consumed = Arc::clone(&consumed);
        tokio::spawn(async move {
            for _ in 0..5 {
                tokio::time::sleep(Duration::from_millis(20)).await;
                let v = buffer.pop().await;
                consumed.lock().await.push(v);
            }
        })
    };

    producer.await.expect("producer task must complete");
    consumer.await.expect("consumer task must complete");

    let got = consumed.lock().await.clone();
    assert_eq!(got, vec![0, 1, 2, 3, 4]);
}

#[tokio::test]
async fn bounded_buffer_drop_newest_policy_drops_when_full() {
    let buffer = BoundedStreamBuffer::new(2, Overflow::DropNewest);

    assert_eq!(buffer.push(1).await.unwrap(), PushOutcome::Accepted);
    assert_eq!(buffer.push(2).await.unwrap(), PushOutcome::Accepted);
    assert_eq!(buffer.push(3).await.unwrap(), PushOutcome::DroppedNewest);

    let first = buffer.pop().await;
    let second = buffer.pop().await;
    assert_eq!((first, second), (1, 2));
}
