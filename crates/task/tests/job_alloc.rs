//! What a job of `Executor::run_job` allocates: its one cell, under `AllocTag::Task`, and
//! for each child its scope forks only a share of the pool queue's block. Counted by the
//! tagged global allocator this test binary installs, across every tag and thread, against
//! the same number of `block_on` calls of a future that is ready at once, so what
//! `block_on` itself costs cancels out.

#![forbid(unsafe_code)]
// A binary that installs `TaggedAlloc` runs under Miri only with Tree Borrows
// (`crates/core/SAFETY.md`), and the crate's Miri run uses Stacked Borrows.
#![cfg(not(miri))]
#![cfg(not(feature = "loom"))]

use core::pin::pin;
use core::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use lactose_core::{AllocTag, TaggedAlloc};
use lactose_task::Executor;

#[global_allocator]
static ALLOCATOR: TaggedAlloc = TaggedAlloc::system();

/// Entries in one block of a `crossbeam-deque` injector, which allocates a block when its
/// last one fills.
const BLOCK: u64 = 63;

fn allocations() -> u64 {
    AllocTag::ALL.iter().map(|&tag| ALLOCATOR.stats(tag).total_allocations).sum()
}

fn task_allocations() -> u64 {
    ALLOCATOR.stats(AllocTag::Task).total_allocations
}

/// The allocations, every tag's and `AllocTag::Task`'s, of `calls` calls of `call`.
fn count(calls: u64, mut call: impl FnMut()) -> (u64, u64) {
    let (all, task) = (allocations(), task_allocations());
    for _ in 0..calls {
        call();
    }
    (allocations() - all, task_allocations() - task)
}

#[test]
fn a_job_allocates_its_cell_and_a_child_a_share_of_a_queue_block() {
    const CHILDREN: u32 = 16;
    let executor = Executor::builder().frame_workers(4).background_workers(1).build().unwrap();
    // Bit `s` is set once the worker of slot `s` has opened a scope that forks.
    let forked = Arc::new(AtomicU32::new(0));
    let job = |forks: u32| {
        let forked = forked.clone();
        executor
            .block_on(executor.run_job(move |ctx| {
                if forks > 0 {
                    forked.fetch_or(1 << ctx.slot(), Ordering::Relaxed);
                }
                let ran = AtomicU32::new(0);
                ctx.scope(|scope| {
                    for _ in 0..forks {
                        scope.fork(|_| {
                            ran.fetch_add(1, Ordering::Relaxed);
                        });
                    }
                });
                ran.into_inner()
            }))
            .unwrap()
    };

    // Warm up the queues, whose first blocks then exist, and every frame-critical worker's
    // scope arena. A worker allocates the arena's chunk, and its list of chunks, the first
    // time it opens a scope that forks, and which worker takes a job is the scheduler's
    // choice: on a loaded machine one may take none of the first few hundred.
    let every_slot = (1 << executor.slot_count()) - 1;
    let mut jobs = 0;
    while jobs < 4 * BLOCK || forked.load(Ordering::Relaxed) != every_slot {
        assert!(
            jobs < 400 * BLOCK,
            "slots {:#b} of {every_slot:#b} opened a scope in {jobs} jobs",
            forked.load(Ordering::Relaxed)
        );
        assert_eq!(job(CHILDREN), CHILDREN);
        jobs += 1;
    }
    // A frame-critical worker locks the timers' state whenever it goes idle. Where std's
    // mutex is a pthread mutex, as on Apple targets, the first lock allocates it, once on
    // each worker that races to it, and on a loaded machine no worker may have gone idle
    // yet. A sleep that waits locks it first, on this thread, to register its timer.
    loop {
        let mut sleep = pin!(executor.sleep_until(Instant::now() + Duration::from_millis(1)));
        let mut waited = false;
        executor
            .block_on(core::future::poll_fn(|cx| {
                let poll = sleep.as_mut().poll(cx);
                waited |= poll.is_pending();
                poll
            }))
            .unwrap();
        if waited {
            break;
        }
    }

    let calls = 20 * BLOCK;
    let (ready, ready_task) = count(calls, || executor.block_on(async {}));
    assert_eq!(ready_task, 0);
    // What a mutex costs when it is first locked: nothing on most targets, one allocation
    // where std's mutex is a pthread mutex, as on Apple targets.
    let (mutex, _) = count(calls, || {
        let mutex = std::hint::black_box(std::sync::Mutex::new(0u32));
        *mutex.lock().unwrap() += 1;
    });
    let (alone, alone_task) = count(calls, || assert_eq!(job(0), 0));
    let (forking, forking_task) = count(calls, || assert_eq!(job(CHILDREN), CHILDREN));

    // The cell, and its mutex where that allocates, under the task tag, whatever the job
    // forks.
    assert_eq!(alone_task, calls + mutex);
    assert_eq!(forking_task, calls + mutex);
    // Besides them, the jobs' queue's block: one per `BLOCK` calls.
    let per_job = alone - ready;
    assert!(
        (alone_task..=alone_task + calls / BLOCK + 1).contains(&per_job),
        "{per_job} allocations for {calls} jobs"
    );
    // A child costs a share of the children's queue's block, nothing else.
    let children = u64::from(CHILDREN) * calls;
    let per_children = forking - alone;
    assert!(
        per_children <= children / BLOCK + 1,
        "{per_children} allocations for {children} children"
    );
    eprintln!(
        "{calls} jobs: {per_job} allocations beside block_on's, {alone_task} of them the task \
         tag's; {children} children: {per_children} more"
    );
    executor.shutdown();
}
