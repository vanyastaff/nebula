use super::*;
use std::sync::atomic::AtomicBool;
use std::task::{Context, Poll, Wake, Waker};

struct ReceiptObserver {
    admission: Arc<Admission>,
    losses: Arc<AtomicUsize>,
    expected_losses: usize,
    live: Arc<AtomicUsize>,
    capacity: Arc<Semaphore>,
    observed: AtomicBool,
    settled: AtomicBool,
}

impl Wake for ReceiptObserver {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        let settled = self.live.load(Ordering::SeqCst) == 0
            && self.losses.load(Ordering::SeqCst) == self.expected_losses
            && self.admission.state.lock().unwrap().outstanding == 0
            && self.capacity.available_permits() == 1;
        self.settled.fetch_and(settled, Ordering::SeqCst);
        self.observed.store(true, Ordering::SeqCst);
    }
}

struct SettlementProbe(Arc<AtomicUsize>);

impl Drop for SettlementProbe {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Clone, Copy, Debug)]
enum ReceiptPath {
    Abandon,
    UnpolledDrop,
    Complete,
    FactoryPanic,
    FuturePanic,
    Timeout,
    Cancel,
}

#[tokio::test(start_paused = true)]
async fn receipt_notification_follows_all_owned_settlement() {
    for path in [
        ReceiptPath::Abandon,
        ReceiptPath::UnpolledDrop,
        ReceiptPath::Complete,
        ReceiptPath::FactoryPanic,
        ReceiptPath::FuturePanic,
        ReceiptPath::Timeout,
        ReceiptPath::Cancel,
    ] {
        let admission = Admission::new();
        let losses = Arc::new(AtomicUsize::new(0));
        let live = Arc::new(AtomicUsize::new(1));
        let capacity = Arc::new(Semaphore::new(1));
        let probe = SettlementProbe(Arc::clone(&live));
        let permit = Arc::clone(&capacity).try_acquire_owned().unwrap();
        let (receipt, mut receiver) = oneshot::channel();
        let observer = Arc::new(ReceiptObserver {
            admission: Arc::clone(&admission),
            losses: Arc::clone(&losses),
            expected_losses: usize::from(!matches!(path, ReceiptPath::Complete)),
            live,
            capacity,
            observed: AtomicBool::new(false),
            settled: AtomicBool::new(true),
        });
        let waker = Waker::from(Arc::clone(&observer));
        assert!(
            Pin::new(&mut receiver)
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        let queued = QueuedTask {
            factory: Box::new(move || {
                let owned = (probe, permit);
                if matches!(path, ReceiptPath::FactoryPanic) {
                    panic!("factory fault fixture");
                }
                Box::pin(async move {
                    let _owned = owned;
                    match path {
                        ReceiptPath::FuturePanic => panic!("future fault fixture"),
                        ReceiptPath::Timeout | ReceiptPath::Cancel => std::future::pending().await,
                        _ => Ok(()),
                    }
                })
            }),
            completion: TaskCompletion {
                loss: TaskLoss {
                    counter: losses,
                    reason: Some("abandoned"),
                    remaining: 1,
                },
                receipt: Some(receipt),
                _permit: admission.acquire(false),
                capacity: None,
            },
            class: JobClass::Entry,
        };
        match path {
            ReceiptPath::Abandon => queued.abandon("test_rejection"),
            ReceiptPath::UnpolledDrop => drop(queued),
            ReceiptPath::Cancel => {
                let mut execution = Box::pin(ReleaseQueue::execute_task(queued));
                assert!(futures::poll!(&mut execution).is_pending());
                drop(execution);
            },
            _ => ReleaseQueue::execute_task(queued).await,
        }
        assert!(
            observer.observed.load(Ordering::SeqCst),
            "{path:?} must notify"
        );
        assert!(
            observer.settled.load(Ordering::SeqCst),
            "{path:?} notified before owned settlement"
        );
        assert!(matches!(
            Pin::new(&mut receiver).poll(&mut Context::from_waker(&waker)),
            Poll::Ready(_)
        ));
    }
}

#[tokio::test]
async fn reentrant_receipt_notification_releases_dispatch_capacity() {
    let admission = Admission::new();
    let capacity = Arc::new(Semaphore::new(1));
    let losses = Arc::new(AtomicUsize::new(0));
    let observer = Arc::new(ReceiptObserver {
        admission: Arc::clone(&admission),
        losses: Arc::clone(&losses),
        expected_losses: 0,
        live: Arc::new(AtomicUsize::new(0)),
        capacity: Arc::clone(&capacity),
        observed: AtomicBool::new(false),
        settled: AtomicBool::new(true),
    });
    let (receipt, mut receiver) = oneshot::channel();
    let waker = Waker::from(Arc::clone(&observer));
    assert!(
        Pin::new(&mut receiver)
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    ReleaseQueue::execute_reentrant(
        QueuedTask {
            factory: Box::new(|| Box::pin(async { Ok(()) })),
            completion: TaskCompletion {
                loss: TaskLoss {
                    counter: losses,
                    reason: Some("abandoned"),
                    remaining: 1,
                },
                receipt: Some(receipt),
                _permit: admission.acquire(false),
                capacity: Some(capacity.try_acquire_owned().unwrap()),
            },
            class: JobClass::Entry,
        },
        Arc::new(()),
    )
    .await;
    assert!(observer.observed.load(Ordering::SeqCst));
    assert!(
        observer.settled.load(Ordering::SeqCst),
        "receipt must make reentrant capacity immediately reusable"
    );
}
