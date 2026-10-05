//! Thread lanes: ordering, wake-ups, cancellation and non-`Send` state.

#![cfg(not(feature = "loom"))]

mod common;

use core::sync::atomic::{AtomicUsize, Ordering};
use std::rc::Rc;
use std::sync::Arc;

use common::{executor, sized};
use lactose_core::ErrorKind;
use lactose_task::{Lane, WorkerClass, lane};

/// State that could not cross threads even if a lane tried to move it.
struct Window {
    title: Rc<String>,
    log: Vec<u32>,
}

#[test]
fn calls_run_on_the_owning_thread_in_submission_order() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Lane<Window>>();

    let executor = executor(2, 1);
    let wakes = Arc::new(AtomicUsize::new(0));
    let owner = std::thread::current();
    let (lane, mut pump) = {
        let wakes = wakes.clone();
        lane::<Window>("test-lane", move || {
            wakes.fetch_add(1, Ordering::Relaxed);
            owner.unpark();
        })
    };
    let mut window = Window { title: Rc::new("lane".to_owned()), log: Vec::new() };
    assert_eq!(pump.drain(&mut window), 0);

    // Several senders on worker threads, each submitting an ordered sequence and awaiting
    // every reply. The owning thread never blocks them and they never block it.
    let senders = sized(4, 2);
    let per_sender = sized(50, 5);
    let tasks: Vec<_> = (0..senders)
        .map(|sender| {
            let lane = lane.clone();
            executor.spawn(WorkerClass::FrameCritical, async move {
                let calls: Vec<_> = (0..per_sender)
                    .map(|sequence| {
                        lane.run(move |window: &mut Window| {
                            window.log.push(sender * 1000 + sequence);
                            window.title.len() as u32 + sequence
                        })
                    })
                    .collect();
                let mut replies = Vec::new();
                for call in calls {
                    replies.push(call.await.unwrap());
                }
                replies
            })
        })
        .collect();

    let total = (senders * per_sender) as usize;
    let mut ran = 0;
    while ran < total {
        ran += pump.drain(&mut window);
        if ran < total {
            std::thread::park();
        }
    }
    assert_eq!(pump.drain(&mut window), 0);
    for task in tasks {
        let replies = executor.block_on(task).unwrap();
        assert_eq!(replies, (0..per_sender).map(|sequence| 4 + sequence).collect::<Vec<_>>());
    }
    // `wake` ran once per queued call, and each sender's calls ran in its own order.
    assert_eq!(wakes.load(Ordering::Relaxed), total);
    for sender in 0..senders {
        let mine: Vec<u32> =
            window.log.iter().copied().filter(|entry| entry / 1000 == sender).collect();
        assert_eq!(
            mine,
            (0..per_sender).map(|sequence| sender * 1000 + sequence).collect::<Vec<_>>()
        );
    }
    executor.shutdown();
}

#[test]
fn dropping_the_pump_cancels_pending_and_later_calls() {
    let executor = executor(1, 1);
    let wakes = Arc::new(AtomicUsize::new(0));
    let (lane, mut pump) = {
        let wakes = wakes.clone();
        lane::<u32>("doomed-lane", move || {
            wakes.fetch_add(1, Ordering::Relaxed);
        })
    };
    let mut state = 0;
    let served = lane.run(|state| {
        *state += 1;
        *state
    });
    assert_eq!(pump.drain(&mut state), 1);

    // Dropping the future of a queued call does not cancel the call.
    drop(lane.run(|state| *state += 10));
    assert_eq!(pump.drain(&mut state), 1);
    assert_eq!(state, 11);

    let token = Arc::new(());
    let pending = {
        let token = token.clone();
        lane.run(move |_| drop(token))
    };
    drop(pump);
    let later = lane.run(|state| *state);
    assert_eq!(wakes.load(Ordering::Relaxed), 3, "a call that was never queued wakes nobody");

    assert_eq!(executor.block_on(served).unwrap(), 1);
    assert_eq!(executor.block_on(pending).unwrap_err().kind(), ErrorKind::Cancelled);
    assert_eq!(executor.block_on(later).unwrap_err().kind(), ErrorKind::Cancelled);
    assert_eq!(Arc::strong_count(&token), 1, "a cancelled call is dropped, not kept");
    executor.shutdown();
}
