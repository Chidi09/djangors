use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn test_signal_basic() {
    let signal = Signal::<String>::new();
    let counter = Arc::new(AtomicUsize::new(0));

    let counter_clone1 = counter.clone();
    signal.connect(move |val| {
        let counter = counter_clone1.clone();
        async move {
            assert_eq!(val, "hello");
            counter.fetch_add(1, Ordering::SeqCst);
        }
    });

    let counter_clone2 = counter.clone();
    signal.connect(move |val| {
        let counter = counter_clone2.clone();
        async move {
            assert_eq!(val, "hello");
            counter.fetch_add(2, Ordering::SeqCst);
        }
    });

    signal.send("hello".to_string()).await;
    assert_eq!(counter.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn test_signal_panic_isolation() {
    let signal = Signal::<()>::new();
    let run_indicator = Arc::new(AtomicUsize::new(0));

    // First subscriber panics
    signal.connect(|_| async move {
        panic!("intended panic in subscriber");
    });

    // Second subscriber should still run successfully
    let indicator = run_indicator.clone();
    signal.connect(move |_| {
        let indicator = indicator.clone();
        async move {
            indicator.fetch_add(1, Ordering::SeqCst);
        }
    });

    signal.send(()).await;
    // Verify the second one ran
    assert_eq!(run_indicator.load(Ordering::SeqCst), 1);
}
