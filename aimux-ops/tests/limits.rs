use std::time::Duration;

use aimux_core::AbortSignal;
use aimux_ops::envelope::{OpsError, ProtocolError};
use aimux_ops::limits::{
    AUDIO_INPUT_ITEMS, ITEM_BYTES, OUTPUT_ITEMS, QUEUE_BYTES, QueueEvent, QueueLimits, QueueStatus,
    bounded_queue,
};
use futures::poll;
use tokio::time::timeout;

fn small(items: usize, bytes: usize) -> QueueLimits {
    QueueLimits::output().lowered(items, bytes, bytes).unwrap()
}

fn assert_resource_limit<T>(result: Result<T, OpsError>) {
    assert!(matches!(
        result,
        Err(OpsError::Protocol(ProtocolError::ResourceLimit(_)))
    ));
}

fn assert_closed<T>(result: Result<T, OpsError>) {
    assert!(matches!(
        result,
        Err(OpsError::Protocol(ProtocolError::InvalidState(_)))
    ));
}

#[test]
fn defaults_match_contract_and_only_lowering_is_allowed() {
    let input = QueueLimits::audio_input();
    let output = QueueLimits::output();
    assert_eq!(input.max_items(), AUDIO_INPUT_ITEMS);
    assert_eq!(input.max_items(), 64);
    assert_eq!(output.max_items(), OUTPUT_ITEMS);
    assert_eq!(output.max_items(), 256);
    assert_eq!(input.max_bytes(), QUEUE_BYTES);
    assert_eq!(output.max_bytes(), 8 * 1024 * 1024);
    assert_eq!(input.max_item_bytes(), ITEM_BYTES);
    assert_eq!(output.max_item_bytes(), 1024 * 1024);
    let lower = output.lowered(2, 8, 4).unwrap();
    assert_eq!(lower.max_items(), 2);
    assert_eq!(lower.max_bytes(), 8);
    assert_eq!(lower.max_item_bytes(), 4);
    for (items, bytes, item) in [
        (0, 8, 4),
        (2, 0, 4),
        (2, 8, 0),
        (3, 8, 4),
        (2, 9, 4),
        (2, 8, 5),
        (usize::MAX, usize::MAX, usize::MAX),
    ] {
        assert_resource_limit(lower.lowered(items, bytes, item));
    }
    assert_resource_limit(input.lowered(65, QUEUE_BYTES, ITEM_BYTES));
}

#[tokio::test]
async fn oversize_is_rejected_before_payload_allocation_even_when_full() {
    let (tx, _rx) = bounded_queue::<Vec<u8>, ()>(small(1, 4));
    tx.send(vec![0; 4], None).await.unwrap();
    assert_resource_limit(tx.reserve(5, None).await);
    assert_resource_limit(tx.reserve(usize::MAX, None).await);
    assert_resource_limit(QueueLimits::output().check_item(ITEM_BYTES + 1));
    QueueLimits::output().check_item(ITEM_BYTES).unwrap();
    // A lowered total-byte budget can be stricter than the single-item budget.
    let lower = QueueLimits::output().lowered(1, 2, 4).unwrap();
    assert_resource_limit(lower.check_item(3));
}

#[tokio::test]
async fn item_budget_backpressures_and_preserves_fifo() {
    let (tx, mut rx) = bounded_queue::<Vec<u8>, ()>(small(2, 8));
    tx.send(vec![1], None).await.unwrap();
    tx.send(vec![2], None).await.unwrap();
    let waiting = tx.send(vec![3], None);
    tokio::pin!(waiting);
    assert!(poll!(waiting.as_mut()).is_pending());
    assert_eq!(rx.recv().await, Some(QueueEvent::Data(vec![1])));
    timeout(Duration::from_secs(1), waiting)
        .await
        .unwrap()
        .unwrap();
    tx.finish(());
    assert_eq!(rx.recv().await, Some(QueueEvent::Data(vec![2])));
    assert_eq!(rx.recv().await, Some(QueueEvent::Data(vec![3])));
    assert_eq!(rx.recv().await, Some(QueueEvent::Terminal(())));
    assert_eq!(rx.recv().await, None);
}

#[tokio::test]
async fn byte_budget_backpressures_before_item_budget() {
    let (tx, mut rx) = bounded_queue::<Vec<u8>, ()>(small(4, 4));
    tx.send(vec![1; 3], None).await.unwrap();
    let waiting = tx.send(vec![2; 2], None);
    tokio::pin!(waiting);
    assert!(poll!(waiting.as_mut()).is_pending());
    assert_eq!(rx.recv().await, Some(QueueEvent::Data(vec![1; 3])));
    timeout(Duration::from_secs(1), waiting)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rx.recv().await, Some(QueueEvent::Data(vec![2; 2])));
}

#[tokio::test]
async fn defaults_enforce_the_actual_item_counts() {
    for limits in [QueueLimits::audio_input(), QueueLimits::output()] {
        let (tx, mut rx) = bounded_queue::<Vec<u8>, ()>(limits);
        for _ in 0..limits.max_items() {
            tx.send(Vec::new(), None).await.unwrap();
        }
        let waiting = tx.reserve(0, None);
        tokio::pin!(waiting);
        assert!(poll!(waiting.as_mut()).is_pending());
        assert_eq!(rx.recv().await, Some(QueueEvent::Data(Vec::new())));
        drop(
            timeout(Duration::from_secs(1), waiting)
                .await
                .unwrap()
                .unwrap(),
        );
    }
}

#[tokio::test]
async fn default_eight_mib_budget_and_terminal_survive_a_full_queue() {
    let (tx, mut rx) = bounded_queue::<Vec<u8>, &str>(QueueLimits::output());
    for _ in 0..8 {
        tx.send(vec![0; ITEM_BYTES], None).await.unwrap();
    }
    let waiting = tx.reserve(1, None);
    tokio::pin!(waiting);
    assert!(poll!(waiting.as_mut()).is_pending());
    assert!(tx.finish("provider error"));
    assert_closed(timeout(Duration::from_secs(1), waiting).await.unwrap());
    for _ in 0..8 {
        assert!(matches!(rx.recv().await, Some(QueueEvent::Data(_))));
    }
    assert_eq!(
        rx.recv().await,
        Some(QueueEvent::Terminal("provider error"))
    );
    assert_eq!(rx.recv().await, None);
}

#[tokio::test]
async fn abort_interrupts_both_kinds_of_capacity_wait() {
    for (items, bytes) in [(1, 4), (4, 1)] {
        let (tx, mut rx) = bounded_queue::<Vec<u8>, &str>(small(items, bytes));
        let abort = AbortSignal::new();
        tx.send(vec![1], None).await.unwrap();
        let waiting = tx.send(vec![2], Some(&abort));
        tokio::pin!(waiting);
        assert!(poll!(waiting.as_mut()).is_pending());
        abort.abort();
        assert!(matches!(
            timeout(Duration::from_secs(1), waiting).await.unwrap(),
            Err(OpsError::AiMux(aimux_core::error::AiMuxError::Aborted(_)))
        ));
        assert!(tx.finish("cancelled"));
        assert_eq!(rx.recv().await, Some(QueueEvent::Data(vec![1])));
        assert_eq!(rx.recv().await, Some(QueueEvent::Terminal("cancelled")));
    }
}

#[tokio::test]
async fn already_aborted_send_does_not_take_free_capacity() {
    let (tx, mut rx) = bounded_queue::<Vec<u8>, ()>(small(1, 1));
    let abort = AbortSignal::new();
    abort.abort();
    assert!(matches!(
        tx.send(vec![1], Some(&abort)).await,
        Err(OpsError::AiMux(aimux_core::error::AiMuxError::Aborted(_)))
    ));
    tx.send(vec![2], None).await.unwrap();
    assert_eq!(rx.recv().await, Some(QueueEvent::Data(vec![2])));
}

#[tokio::test]
async fn terminal_is_first_writer_wins_and_not_capacity_bound() {
    let (tx, mut rx) = bounded_queue::<Vec<u8>, &str>(small(1, 1));
    tx.send(vec![1], None).await.unwrap();
    assert_eq!(tx.status(), QueueStatus::Open);
    assert!(tx.finish("success"));
    assert!(!tx.finish("cancelled"));
    assert_eq!(tx.status(), QueueStatus::Finished);
    assert_closed(tx.send(vec![2], None).await);
    assert_eq!(rx.recv().await, Some(QueueEvent::Data(vec![1])));
    assert_eq!(rx.recv().await, Some(QueueEvent::Terminal("success")));
    assert_eq!(rx.recv().await, None);
    assert!(!tx.finish("late error"));
    drop(rx);
    assert_eq!(tx.status(), QueueStatus::Finished);
}

#[tokio::test]
async fn terminal_does_not_wait_for_outstanding_reservations() {
    let (tx, mut rx) = bounded_queue::<Vec<u8>, &str>(small(1, 1));
    let permit = tx.reserve(1, None).await.unwrap();
    tx.finish("error");
    assert_eq!(rx.recv().await, Some(QueueEvent::Terminal("error")));
    assert_closed(permit.send(vec![1]));
    assert_eq!(rx.recv().await, None);
}

#[tokio::test]
async fn dropped_reservations_release_item_and_byte_capacity() {
    let (tx, mut rx) = bounded_queue::<Vec<u8>, ()>(small(1, 4));
    let permit = tx.reserve(4, None).await.unwrap();
    let waiting = tx.send(vec![2; 4], None);
    tokio::pin!(waiting);
    assert!(poll!(waiting.as_mut()).is_pending());
    drop(permit);
    timeout(Duration::from_secs(1), waiting)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rx.recv().await, Some(QueueEvent::Data(vec![2; 4])));
}

#[tokio::test]
async fn smaller_payload_returns_unused_reserved_bytes() {
    let (tx, mut rx) = bounded_queue::<Vec<u8>, ()>(small(2, 4));
    let permit = tx.reserve(4, None).await.unwrap();
    permit.send(vec![1]).unwrap();
    tx.send(vec![2; 3], None).await.unwrap();
    assert_eq!(rx.recv().await, Some(QueueEvent::Data(vec![1])));
    assert_eq!(rx.recv().await, Some(QueueEvent::Data(vec![2; 3])));
}

#[tokio::test]
async fn oversized_commit_releases_reservation_without_enqueuing() {
    let (tx, mut rx) = bounded_queue::<Vec<u8>, ()>(small(1, 4));
    let permit = tx.reserve(1, None).await.unwrap();
    assert_resource_limit(permit.send(vec![0; 2]));
    tx.send(vec![1; 4], None).await.unwrap();
    assert_eq!(rx.recv().await, Some(QueueEvent::Data(vec![1; 4])));
}

#[tokio::test]
async fn explicit_close_discards_data_and_wakes_all_waiters() {
    let (tx, mut rx) = bounded_queue::<Vec<u8>, ()>(small(1, 1));
    tx.send(vec![1], None).await.unwrap();
    let waiting1 = tx.send(vec![2], None);
    let waiting2 = tx.send(vec![3], None);
    tokio::pin!(waiting1, waiting2);
    assert!(poll!(waiting1.as_mut()).is_pending());
    assert!(poll!(waiting2.as_mut()).is_pending());
    rx.close();
    rx.close();
    assert_closed(timeout(Duration::from_secs(1), waiting1).await.unwrap());
    assert_closed(timeout(Duration::from_secs(1), waiting2).await.unwrap());
    assert_eq!(tx.status(), QueueStatus::ConsumerClosed);
    assert_eq!(rx.recv().await, None);
    assert!(!tx.finish(()));
}

#[tokio::test]
async fn dropping_consumer_wakes_sender_and_invalidates_reservations() {
    let (tx, rx) = bounded_queue::<Vec<u8>, ()>(small(1, 1));
    let permit = tx.reserve(1, None).await.unwrap();
    let waiting = tx.send(vec![2], None);
    tokio::pin!(waiting);
    assert!(poll!(waiting.as_mut()).is_pending());
    drop(rx);
    assert_closed(timeout(Duration::from_secs(1), waiting).await.unwrap());
    assert_closed(permit.send(vec![1]));
    assert_eq!(tx.status(), QueueStatus::ConsumerClosed);
}

#[tokio::test]
async fn close_releases_only_queued_budget_not_outstanding_reservations() {
    let (tx, mut rx) = bounded_queue::<Vec<u8>, ()>(small(3, 4));
    let permit = tx.reserve(2, None).await.unwrap();
    tx.send(vec![0; 2], None).await.unwrap();
    rx.close();
    drop(permit); // Must not underflow the budget after close.
    assert_closed(tx.reserve(1, None).await);
}

#[tokio::test]
async fn waiting_receive_wakes_for_data_and_terminal() {
    let (tx, mut rx) = bounded_queue::<String, &str>(small(1, 4));
    {
        let waiting = rx.recv();
        tokio::pin!(waiting);
        assert!(poll!(waiting.as_mut()).is_pending());
        tx.send("hi".to_string(), None).await.unwrap();
        assert_eq!(
            timeout(Duration::from_secs(1), waiting).await.unwrap(),
            Some(QueueEvent::Data("hi".to_string()))
        );
    }
    let waiting = rx.recv();
    tokio::pin!(waiting);
    assert!(poll!(waiting.as_mut()).is_pending());
    tx.finish("done");
    assert_eq!(
        timeout(Duration::from_secs(1), waiting).await.unwrap(),
        Some(QueueEvent::Terminal("done"))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_senders_make_progress_without_loss() {
    let (tx, mut rx) = bounded_queue::<Vec<u8>, ()>(small(3, 5));
    let mut tasks = Vec::new();
    for producer in 0..4u8 {
        let tx = tx.clone();
        tasks.push(tokio::spawn(async move {
            for sequence in 0..100u8 {
                tx.reserve(2, None)
                    .await
                    .unwrap()
                    .send(vec![producer, sequence])
                    .unwrap();
            }
        }));
    }
    let terminal = tx.clone();
    let finish = tokio::spawn(async move {
        for task in tasks {
            task.await.unwrap();
        }
        terminal.finish(());
    });
    timeout(Duration::from_secs(5), async {
        let mut next = [0u8; 4];
        while let Some(event) = rx.recv().await {
            match event {
                QueueEvent::Data(data) => {
                    assert_eq!(data[1], next[data[0] as usize]);
                    next[data[0] as usize] += 1;
                }
                QueueEvent::Terminal(()) => assert_eq!(next, [100; 4]),
            }
        }
        finish.await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn retained_allocation_capacity_and_utf8_bytes_are_budgeted() {
    let (tx, mut rx) = bounded_queue::<Vec<u8>, ()>(small(2, 4));
    let mut spare = Vec::with_capacity(4);
    spare.push(1);
    tx.send(spare, None).await.unwrap();
    let waiting = tx.reserve(1, None);
    tokio::pin!(waiting);
    assert!(poll!(waiting.as_mut()).is_pending());
    assert!(matches!(rx.recv().await, Some(QueueEvent::Data(_))));
    drop(
        timeout(Duration::from_secs(1), waiting)
            .await
            .unwrap()
            .unwrap(),
    );

    let (tx, _rx) = bounded_queue::<String, ()>(small(2, 4));
    assert_resource_limit(tx.send("你好".into(), None).await);
    tx.send("你".into(), None).await.unwrap();
}

#[tokio::test]
async fn cancelling_waiting_send_does_not_consume_future_capacity() {
    let (tx, mut rx) = bounded_queue::<Box<[u8]>, ()>(small(1, 1));
    tx.send(vec![1].into_boxed_slice(), None).await.unwrap();
    {
        let waiting = tx.send(vec![2].into_boxed_slice(), None);
        tokio::pin!(waiting);
        assert!(poll!(waiting.as_mut()).is_pending());
        // Dropping this future must not consume capacity or enqueue the item.
    }
    assert_eq!(
        rx.recv().await,
        Some(QueueEvent::Data(vec![1].into_boxed_slice()))
    );
    tx.send(vec![3].into_boxed_slice(), None).await.unwrap();
    tx.finish(());
    assert_eq!(
        rx.recv().await,
        Some(QueueEvent::Data(vec![3].into_boxed_slice()))
    );
    assert_eq!(rx.recv().await, Some(QueueEvent::Terminal(())));
}
