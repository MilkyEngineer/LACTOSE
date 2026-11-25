//! One interpolation slot: a sequence lock over a ring of states, written once against
//! [`Memory`] so the loom models (`models`) check these functions, not a copy of them. The
//! buffer ships them over the standard atomics ([`Native`]) at its own shape; the models
//! run them over loom's at a smaller one.
//!
//! The writer stores the odd sequence (`Relaxed`), issues `fence(Release)`, stores the words
//! (`Relaxed`), and stores the even sequence with `Release`; a reader loads the sequence with
//! `Acquire`, loads the words with `Relaxed`, issues `fence(Acquire)`, and reloads the
//! sequence (`Relaxed`), retrying when the first load was odd or the two differ. The even
//! store's `Release` pairs with a reader's first `Acquire` load that reads it: every word,
//! `generation` and `count` of that write and the writes before it happens before the
//! reader's loads, so none of them reads a value older than that sequence. The writer's
//! `fence(Release)` pairs with the reader's `fence(Acquire)` through any word, `generation`
//! or `count` the reader loads from a later write: that write's odd store then happens before
//! the reader's reload, which reads it or a later value and retries.
//!
//! Those four orderings are the lock's only ones stronger than `Relaxed`, and each operation
//! and ordering pair names exactly one of them, which is how the models' mutants weaken one
//! at a time; `each_ordering_stronger_than_relaxed_has_one_site` holds both. A first load
//! left `Relaxed` admits torn reads on aarch64 (the review measured them); `Acquire` admits
//! none. `generation` and `count` are words read inside that window.
//!
//! One writer: every store to a slot is the buffer's one writer's, so its own loads of the
//! slot (the sequence in `write`, `generation`) need no ordering and its `read` in `push`
//! never retries. The sequence is 32 bits and wraps: a reader that loads the same even value
//! twice has seen no write in between unless a multiple of 2^31 writes of the slot fell
//! between its two loads.

use core::sync::atomic::Ordering::{self, Acquire, Relaxed, Release};
use core::sync::atomic::{AtomicU32, AtomicU64};

/// The atomic operations the lock is built from.
pub(crate) trait Memory {
    type U32;
    type U64;

    fn new_u32(value: u32) -> Self::U32;
    fn new_u64(value: u64) -> Self::U64;
    fn load_u32(atomic: &Self::U32, order: Ordering) -> u32;
    fn store_u32(atomic: &Self::U32, value: u32, order: Ordering);
    fn load_u64(atomic: &Self::U64, order: Ordering) -> u64;
    fn store_u64(atomic: &Self::U64, value: u64, order: Ordering);
    fn fence(order: Ordering);
    /// Waits a moment before a reader retries.
    fn pause();
}

/// The standard atomics: what the interpolation buffer runs on.
pub(crate) struct Native;

impl Memory for Native {
    type U32 = AtomicU32;
    type U64 = AtomicU64;

    fn new_u32(value: u32) -> AtomicU32 {
        AtomicU32::new(value)
    }

    fn new_u64(value: u64) -> AtomicU64 {
        AtomicU64::new(value)
    }

    fn load_u32(atomic: &AtomicU32, order: Ordering) -> u32 {
        atomic.load(order)
    }

    fn store_u32(atomic: &AtomicU32, value: u32, order: Ordering) {
        atomic.store(value, order);
    }

    fn load_u64(atomic: &AtomicU64, order: Ordering) -> u64 {
        atomic.load(order)
    }

    fn store_u64(atomic: &AtomicU64, value: u64, order: Ordering) {
        atomic.store(value, order);
    }

    fn fence(order: Ordering) {
        core::sync::atomic::fence(order);
    }

    fn pause() {
        core::hint::spin_loop();
    }
}

/// A slot of `STATES` states of `WORDS` words each. A state's word 0 holds its tick in the
/// upper 32 bits, by which the ring is ordered.
pub(crate) struct Slot<M: Memory, const STATES: usize, const WORDS: usize> {
    sequence: M::U32,
    /// The slot's current allocation, zero when free.
    generation: M::U32,
    /// How many states the ring holds, up to `STATES`: `states[..count]`, ascending by
    /// tick, so the newest is the last.
    count: M::U32,
    states: [[M::U64; WORDS]; STATES],
}

/// A slot's contents as one consistent read: the generation, then the states held,
/// ascending by tick, in `states[..count]`; the words after them are zero.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Snapshot<const STATES: usize, const WORDS: usize> {
    pub(crate) generation: u32,
    pub(crate) count: usize,
    pub(crate) states: [[u64; WORDS]; STATES],
}

/// The tick a state's words hold.
pub(crate) const fn tick<const WORDS: usize>(state: &[u64; WORDS]) -> u32 {
    (state[0] >> 32) as u32
}

/// Whether `tick` is after `than` under the wrapping comparison the frames use.
pub(crate) const fn tick_after(tick: u32, than: u32) -> bool {
    let distance = tick.wrapping_sub(than);
    distance != 0 && distance < 1 << 31
}

impl<M: Memory, const STATES: usize, const WORDS: usize> Slot<M, STATES, WORDS> {
    /// A free, empty slot at sequence zero.
    pub(crate) fn new() -> Self {
        const { assert!(WORDS > 0, "a state's word 0 holds its tick") };
        Self {
            sequence: M::new_u32(0),
            generation: M::new_u32(0),
            count: M::new_u32(0),
            states: core::array::from_fn(|_| core::array::from_fn(|_| M::new_u64(0))),
        }
    }

    /// The slot's generation as the writer last stored it. Only the writer may call it.
    pub(crate) fn generation(&self) -> u32 {
        M::load_u32(&self.generation, Relaxed)
    }

    /// Empties the slot and sets its generation, zero to free it, in one write.
    pub(crate) fn reset(&self, generation: u32) {
        self.write(|slot| {
            M::store_u32(&slot.generation, generation, Relaxed);
            M::store_u32(&slot.count, 0, Relaxed);
        });
    }

    /// Puts `state` into the ring, which keeps the `STATES` newest states by tick: a state
    /// older than the oldest of a full ring is dropped without a write, one with the tick of
    /// a kept state replaces it, and otherwise the oldest falls out of a full ring.
    pub(crate) fn push(&self, state: [u64; WORDS]) {
        let snapshot = self.read();
        let kept = &snapshot.states[..snapshot.count];
        let at = kept.iter().take_while(|kept| tick_after(tick(&state), tick(kept))).count();
        let replaces = kept.get(at).is_some_and(|kept| tick(kept) == tick(&state));
        if !replaces && at == 0 && kept.len() == STATES {
            return;
        }
        // Ascending by tick: the states before `state`, `state` (replacing an equal tick),
        // the states after it; then the oldest dropped if there are too many.
        let after = if replaces { at + 1 } else { at };
        let first = (at + 1 + kept.len() - after).saturating_sub(STATES);
        let ring = kept[..at].iter().chain([&state]).chain(&kept[after..]).skip(first);
        self.write(|slot| slot.store_states(ring));
    }

    /// The writer's side of the lock: `write` stores words with `Relaxed` between the odd
    /// and the even sequence.
    fn write(&self, write: impl FnOnce(&Self)) {
        let sequence = M::load_u32(&self.sequence, Relaxed);
        M::store_u32(&self.sequence, sequence.wrapping_add(1), Relaxed);
        M::fence(Release);
        write(self);
        M::store_u32(&self.sequence, sequence.wrapping_add(2), Release);
    }

    /// Stores `states`, at most `STATES` of them, as the ring, inside a write.
    fn store_states<'a>(&self, states: impl Iterator<Item = &'a [u64; WORDS]>) {
        let mut count = 0;
        for (held, state) in self.states.iter().zip(states) {
            for (word, &value) in held.iter().zip(state) {
                M::store_u64(word, value, Relaxed);
            }
            count += 1;
        }
        M::store_u32(&self.count, count, Relaxed);
    }

    /// One consistent read, retried until no write overlapped it.
    pub(crate) fn read(&self) -> Snapshot<STATES, WORDS> {
        self.read_counted().0
    }

    /// [`read`](Self::read), and how many times it was retried.
    pub(crate) fn read_counted(&self) -> (Snapshot<STATES, WORDS>, u32) {
        let mut retries = 0_u32;
        loop {
            let before = M::load_u32(&self.sequence, Acquire);
            if before & 1 == 1 {
                retries = retries.saturating_add(1);
                M::pause();
                continue;
            }
            let generation = M::load_u32(&self.generation, Relaxed);
            // A torn count is caught by the sequence check below; the bound keeps the
            // indexing in range meanwhile.
            let count = (M::load_u32(&self.count, Relaxed) as usize).min(STATES);
            let mut states = [[0; WORDS]; STATES];
            for (held, words) in self.states[..count].iter().zip(&mut states) {
                for (word, value) in held.iter().zip(words) {
                    *value = M::load_u64(word, Relaxed);
                }
            }
            M::fence(Acquire);
            if M::load_u32(&self.sequence, Relaxed) == before {
                return (Snapshot { generation, count, states }, retries);
            }
            retries = retries.saturating_add(1);
            M::pause();
        }
    }
}

#[cfg(test)]
mod tests {
    use core::cell::RefCell;

    use super::*;

    std::thread_local! {
        static CALLS: RefCell<Vec<(&'static str, Ordering)>> = const { RefCell::new(Vec::new()) };
    }

    /// The standard atomics, recording each call's operation and ordering on this thread.
    struct Counting;

    fn record(operation: &'static str, order: Ordering) {
        CALLS.with_borrow_mut(|calls| calls.push((operation, order)));
    }

    impl Memory for Counting {
        type U32 = AtomicU32;
        type U64 = AtomicU64;

        fn new_u32(value: u32) -> AtomicU32 {
            Native::new_u32(value)
        }

        fn new_u64(value: u64) -> AtomicU64 {
            Native::new_u64(value)
        }

        fn load_u32(atomic: &AtomicU32, order: Ordering) -> u32 {
            record("load_u32", order);
            Native::load_u32(atomic, order)
        }

        fn store_u32(atomic: &AtomicU32, value: u32, order: Ordering) {
            record("store_u32", order);
            Native::store_u32(atomic, value, order);
        }

        fn load_u64(atomic: &AtomicU64, order: Ordering) -> u64 {
            record("load_u64", order);
            Native::load_u64(atomic, order)
        }

        fn store_u64(atomic: &AtomicU64, value: u64, order: Ordering) {
            record("store_u64", order);
            Native::store_u64(atomic, value, order);
        }

        fn fence(order: Ordering) {
            record("fence", order);
            Native::fence(order);
        }

        fn pause() {
            Native::pause();
        }
    }

    /// The calls stronger than `Relaxed` that `run` makes, in order.
    fn ordered(run: impl FnOnce()) -> Vec<(&'static str, Ordering)> {
        CALLS.with_borrow_mut(Vec::clear);
        run();
        CALLS.with_borrow(|calls| {
            calls.iter().copied().filter(|&(_, order)| order != Relaxed).collect()
        })
    }

    /// The models' mutants find each ordering by its operation and ordering alone, so each of
    /// the four occurs at one site, and no other ordering stronger than `Relaxed` occurs: the
    /// calls a single thread makes are counted, and the code's sites, retry paths included.
    #[test]
    fn each_ordering_stronger_than_relaxed_has_one_site() {
        const WRITE: [(&str, Ordering); 2] = [("fence", Release), ("store_u32", Release)];
        const READ: [(&str, Ordering); 2] = [("load_u32", Acquire), ("fence", Acquire)];
        let slot = Slot::<Counting, 2, 2>::new();
        assert_eq!(ordered(|| slot.reset(1)), WRITE);
        // Pushes into the ring at its end, at its front, past a full ring, and replacing.
        for tick in [2_u32, 1, 3, 4, 4] {
            let state = [u64::from(tick) << 32, 7];
            assert_eq!(ordered(|| slot.push(state)), [READ, WRITE].concat());
        }
        // A push older than a full ring's oldest is dropped without a write.
        assert_eq!(ordered(|| slot.push([1 << 32, 7])), READ);
        assert_eq!(
            ordered(|| {
                slot.read();
            }),
            READ
        );
        assert_eq!(
            ordered(|| {
                slot.generation();
            }),
            []
        );
        assert_eq!(slot.read().count, 2);

        // A site on a path the calls above never take (a reader's retry) is counted in the
        // code before this module, comments and whitespace aside.
        let code: String = include_str!("seqlock.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap_or_default()
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .flat_map(str::split_whitespace)
            .collect();
        for site in ["M::fence(Release)", "M::fence(Acquire)", ",Release)", ",Acquire)"] {
            assert_eq!(code.matches(site).count(), 1, "`{site}` in seqlock.rs");
        }
    }

    std::thread_local! {
        /// How many of this thread's next `Acquire` loads read an odd sequence.
        static ODD: core::cell::Cell<u32> = const { core::cell::Cell::new(0) };
        /// How many of this thread's next reads see a whole write begin and end between
        /// their first load of the sequence and its reload.
        static MOVED: core::cell::Cell<u32> = const { core::cell::Cell::new(0) };
        /// The `Relaxed` 32-bit loads since this thread's last `Acquire` one.
        static SINCE: core::cell::Cell<u32> = const { core::cell::Cell::new(0) };
    }

    /// The standard atomics, with a reader's first load seeing a write in progress as many
    /// times as [`ODD`] says, and its reload seeing a write that began and ended since the
    /// first load as many times as [`MOVED`] says. The reload is the third `Relaxed` 32-bit
    /// load after the first, which is `Acquire`: the generation, the count, the sequence.
    struct Overlapped;

    impl Memory for Overlapped {
        type U32 = AtomicU32;
        type U64 = AtomicU64;

        fn new_u32(value: u32) -> AtomicU32 {
            Native::new_u32(value)
        }

        fn new_u64(value: u64) -> AtomicU64 {
            Native::new_u64(value)
        }

        fn load_u32(atomic: &AtomicU32, order: Ordering) -> u32 {
            let value = Native::load_u32(atomic, order);
            if order == Acquire {
                SINCE.set(0);
                let odd = ODD.get();
                if odd > 0 {
                    ODD.set(odd - 1);
                    return value | 1;
                }
                return value;
            }
            let since = SINCE.get() + 1;
            SINCE.set(since);
            let moved = MOVED.get();
            if since == 3 && moved > 0 {
                MOVED.set(moved - 1);
                return value.wrapping_add(2);
            }
            value
        }

        fn store_u32(atomic: &AtomicU32, value: u32, order: Ordering) {
            Native::store_u32(atomic, value, order);
        }

        fn load_u64(atomic: &AtomicU64, order: Ordering) -> u64 {
            Native::load_u64(atomic, order)
        }

        fn store_u64(atomic: &AtomicU64, value: u64, order: Ordering) {
            Native::store_u64(atomic, value, order);
        }

        fn fence(order: Ordering) {
            Native::fence(order);
        }

        fn pause() {
            Native::pause();
        }
    }

    #[test]
    fn a_read_counts_its_retries() {
        let slot = Slot::<Overlapped, 2, 2>::new();
        slot.reset(1);
        slot.push([2 << 32, 7]);
        let (clean, retries) = slot.read_counted();
        assert_eq!((clean.count, retries), (1, 0));
        ODD.set(3);
        let (read, retries) = slot.read_counted();
        assert_eq!((read, retries), (clean, 3));
    }

    /// The other retry: a read whose reload of the sequence finds that a write began and
    /// ended since its first load (review r1 of `measure-m2`, C5).
    #[test]
    fn a_read_counts_its_retries_by_the_reload() {
        let slot = Slot::<Overlapped, 2, 2>::new();
        slot.reset(1);
        slot.push([2 << 32, 7]);
        let (clean, retries) = slot.read_counted();
        assert_eq!((clean.count, retries), (1, 0));
        MOVED.set(2);
        let (read, retries) = slot.read_counted();
        assert_eq!((read, retries), (clean, 2));
        assert_eq!(MOVED.get(), 0, "both reloads were taken");
    }
}
