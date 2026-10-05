//! Chaos scheduling for the determinism harness (architecture section 8.3).
//!
//! Chaos is a runtime option. A worker of an executor built without it carries `None` and
//! pays one branch on a local per job.
//!
//! A recorded log lists every graph job and scope child in the order they started, with the
//! worker that ran each. Recording must not disturb the interleavings chaos exists to
//! explore, so a job start costs one increment of a shared sequence counter and a push onto
//! the recording worker's own list; the lists are merged by sequence number when the log is
//! read. The log grows by 28 bytes per job for as long as the executor lives.
//!
//! Replay does not schedule: it holds released work in one pool and hands each worker
//! exactly the job the log names next, in log order. That is slow and serialised on a
//! mutex, which is fine for reproducing one failure.

use core::ops::Range;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use lactose_core::{Error, ErrorKind, Result};

use crate::job::{ChildHeader, ScopeCore};
use crate::lock;
use crate::pool::Entry;

/// Perturbs scheduling to expose ordering bugs that a four-core CI runner never hits.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ChaosConfig {
    /// Seeds every perturbation. The seed alone does not make a run repeat, because real
    /// threads still race; exact repetition needs a recorded [`ExecutionLog`].
    pub seed: u64,
    /// Choose steal victims and steal timing from the seed.
    pub shuffle_steals: bool,
    /// Yield the worker's time slice at seeded points between jobs.
    pub inject_yields: bool,
    /// Split every range job down to single units, as if every half had been stolen.
    pub force_split: bool,
    /// Record which worker ran which job in which order, for
    /// [`crate::ExecutorBuilder::replay`].
    pub record: bool,
}

/// A recorded job execution order. Opaque; it round-trips through bytes so a failing CI run
/// can attach it as an artifact.
///
/// It covers graph jobs (each call of `GraphNode::run` and `finalize`) and scope children.
/// Stackless tasks are not recorded: what wakes them (timers, IO, other threads) is outside
/// anything a log could replay.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ExecutionLog {
    bytes: Vec<u8>,
}

impl ExecutionLog {
    /// Rebuilds a log from [`ExecutionLog::as_bytes`] output. The bytes are validated when
    /// an executor is built with the log.
    #[must_use]
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    /// The serialised log.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

const MAGIC: [u8; 8] = *b"WHISKLOG";
const VERSION: u32 = 1;
const HEADER_LEN: usize = 16;
const RECORD_LEN: usize = 28;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum JobKind {
    /// One call of `GraphNode::run`.
    Units = 0,
    /// One call of `GraphNode::finalize`.
    Finalize = 1,
    /// One call of a scope child.
    Child = 2,
}

/// One job start.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct LogRecord {
    pub(crate) worker: u32,
    pub(crate) kind: JobKind,
    /// The ordinal of the graph run for graph jobs; for a child, the log position of the
    /// job that opened its scope.
    pub(crate) key: u64,
    /// The node for graph jobs; for a child, its fork ordinal within the opening job.
    pub(crate) index: u32,
    pub(crate) units: Range<u32>,
}

fn encode(workers: u32, records: &[LogRecord]) -> ExecutionLog {
    let mut bytes = Vec::with_capacity(HEADER_LEN + records.len() * RECORD_LEN);
    bytes.extend_from_slice(&MAGIC);
    bytes.extend_from_slice(&VERSION.to_le_bytes());
    bytes.extend_from_slice(&workers.to_le_bytes());
    for record in records {
        bytes.extend_from_slice(&record.worker.to_le_bytes());
        bytes.extend_from_slice(&(record.kind as u32).to_le_bytes());
        bytes.extend_from_slice(&record.key.to_le_bytes());
        bytes.extend_from_slice(&record.index.to_le_bytes());
        bytes.extend_from_slice(&record.units.start.to_le_bytes());
        bytes.extend_from_slice(&record.units.end.to_le_bytes());
    }
    ExecutionLog { bytes }
}

fn decode(log: &ExecutionLog, workers: u32) -> Result<Vec<LogRecord>> {
    let invalid = |message: &'static str| Error::new(ErrorKind::InvalidData, message);
    let u32_at = |at: usize| u32::from_le_bytes(log.bytes[at..at + 4].try_into().expect("4 bytes"));
    let u64_at = |at: usize| u64::from_le_bytes(log.bytes[at..at + 8].try_into().expect("8 bytes"));

    if log.bytes.len() < HEADER_LEN || log.bytes[..8] != MAGIC {
        return Err(invalid("not an execution log"));
    }
    if u32_at(8) != VERSION {
        return Err(Error::new(ErrorKind::VersionMismatch, "execution log version differs"));
    }
    if u32_at(12) != workers {
        return Err(invalid(
            "execution log was recorded with a different frame-critical worker count",
        ));
    }
    let body = log.bytes.len() - HEADER_LEN;
    if !body.is_multiple_of(RECORD_LEN) {
        return Err(invalid("execution log is truncated"));
    }
    (0..body / RECORD_LEN)
        .map(|record| {
            let at = HEADER_LEN + record * RECORD_LEN;
            let kind = match u32_at(at + 4) {
                0 => JobKind::Units,
                1 => JobKind::Finalize,
                2 => JobKind::Child,
                _ => return Err(invalid("execution log names an unknown job kind")),
            };
            let record = LogRecord {
                worker: u32_at(at),
                kind,
                key: u64_at(at + 8),
                index: u32_at(at + 16),
                units: u32_at(at + 20)..u32_at(at + 24),
            };
            if record.worker >= workers || record.units.start > record.units.end {
                return Err(invalid("execution log record is out of range"));
            }
            Ok(record)
        })
        .collect()
}

/// A small counter-free generator for scheduling perturbation (xorshift64*). Quality does
/// not matter here; being seedable and cheap does.
pub(crate) struct ChaosRng(u64);

impl ChaosRng {
    pub(crate) fn new(seed: u64, stream: u64) -> Self {
        // splitmix64 of the seed and stream, so neighbouring workers do not correlate.
        let mut z = seed ^ stream.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        Self((z ^ (z >> 31)) | 1)
    }

    pub(crate) fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A value below `bound`, which must be non-zero.
    pub(crate) fn below(&mut self, bound: usize) -> usize {
        ((self.next() >> 32) as usize) % bound
    }
}

/// Which pooled work a caller of [`Chaos::take`] may be handed.
#[derive(Clone, Copy)]
pub(crate) enum Filter {
    /// A worker between jobs.
    Any,
    /// An opener waiting on this scope.
    Scope(NonNull<ScopeCore>),
    /// A worker draining during shutdown: children still have an opener waiting for them.
    Children,
}

/// Who should look at the pool after it changed.
pub(crate) enum Wake {
    Nobody,
    Worker(usize),
    Anyone,
}

/// The job starts one worker has noted, with their positions.
type Journal = Mutex<Vec<(u64, LogRecord)>>;

/// The executor-wide chaos state.
pub(crate) struct Chaos {
    pub(crate) config: ChaosConfig,
    workers: u32,
    /// The number of jobs started so far, which is the position the next one gets.
    started: AtomicU64,
    /// The job starts each frame-critical worker has seen, with their positions, if
    /// recording. A worker's list is locked by that worker and by a reader of the log.
    recorded: Option<Box<[Journal]>>,
    state: Mutex<State>,
    /// The log being replayed; empty when not replaying.
    script: Vec<LogRecord>,
    pub(crate) replaying: bool,
}

/// The replay pool.
struct State {
    /// Released work nobody has taken yet.
    pool: Vec<Entry>,
    /// Jobs handed out and not yet reported finished.
    in_flight: u32,
    /// The next script record to hand out. At `script.len()`, replay is in free mode: the
    /// log is exhausted (or abandoned at shutdown) and any worker takes any pooled work.
    cursor: usize,
}

impl Chaos {
    pub(crate) fn new(
        config: ChaosConfig,
        replay: Option<&ExecutionLog>,
        workers: u32,
    ) -> Result<Self> {
        let script = match replay {
            Some(log) => decode(log, workers)?,
            None => Vec::new(),
        };
        Ok(Self {
            config,
            workers,
            started: AtomicU64::new(0),
            recorded: config.record.then(|| (0..workers).map(|_| Mutex::default()).collect()),
            state: Mutex::new(State { pool: Vec::new(), in_flight: 0, cursor: 0 }),
            script,
            replaying: replay.is_some(),
        })
    }

    pub(crate) fn recording(&self) -> bool {
        self.config.record
    }

    /// Gives a starting job its position and notes it if recording. In replay the caller
    /// holds the pool's lock, which is what makes positions follow the script.
    pub(crate) fn record(&self, record: LogRecord) -> u64 {
        let position = self.started.fetch_add(1, Ordering::Relaxed);
        if let Some(recorded) = &self.recorded {
            lock(&recorded[record.worker as usize]).push((position, record));
        }
        position
    }

    pub(crate) fn log(&self) -> Option<ExecutionLog> {
        let recorded = self.recorded.as_ref()?;
        let mut records: Vec<(u64, LogRecord)> =
            recorded.iter().flat_map(|worker| lock(worker).clone()).collect();
        records.sort_unstable_by_key(|&(position, _)| position);
        // A job that has its position but has not been noted yet leaves a gap; the log
        // ends before the first one, so it is always a prefix of the full history.
        let complete = records
            .iter()
            .zip(0u64..)
            .take_while(|((position, _), expected)| position == expected)
            .map(|((_, record), _)| record.clone())
            .collect::<Vec<_>>();
        Some(encode(self.workers, &complete))
    }

    /// Adds released work to the replay pool.
    pub(crate) fn push(&self, entry: Entry) -> Wake {
        let mut state = lock(&self.state);
        state.pool.push(entry);
        self.next_wake(&state)
    }

    fn next_wake(&self, state: &State) -> Wake {
        match self.script.get(state.cursor) {
            Some(record) => Wake::Worker(record.worker as usize),
            None if state.pool.is_empty() => Wake::Nobody,
            None => Wake::Anyone,
        }
    }

    /// Stops following the script. Used at shutdown so waiting openers still get their
    /// children run.
    pub(crate) fn abandon(&self) {
        lock(&self.state).cursor = self.script.len();
    }

    /// Whether [`Chaos::take`] would currently hand `worker` something.
    pub(crate) fn ready(&self, worker: usize) -> bool {
        let state = lock(&self.state);
        match self.script.get(state.cursor) {
            Some(record) => record.worker as usize == worker && find(&state.pool, record).is_some(),
            None => !state.pool.is_empty(),
        }
    }

    /// Hands `worker` its next job: the one the script names, or in free mode any pooled
    /// work that passes `filter`. Returns the job, its position, and who to wake next.
    pub(crate) fn take(&self, worker: usize, filter: Filter) -> (Option<(Entry, u64)>, Wake) {
        let mut state = lock(&self.state);
        let (at, units, record) = match self.script.get(state.cursor) {
            Some(record) => {
                if record.worker as usize != worker {
                    return (None, Wake::Nobody);
                }
                let Some(at) = find(&state.pool, record) else { return (None, Wake::Nobody) };
                if !passes(&state.pool[at], filter) {
                    // The log has this worker start an unrelated job next, which it did
                    // after its scope closed. Children that have started elsewhere are
                    // still finishing; the opener waits on its latch and comes back for
                    // this job from the worker loop.
                    return (None, Wake::Nobody);
                }
                state.cursor += 1;
                (at, record.units.clone(), record.clone())
            }
            None => {
                let Some(at) = state.pool.iter().position(|entry| passes(entry, filter)) else {
                    return (None, Wake::Nobody);
                };
                let record = describe(&state.pool[at], worker as u32);
                (at, record.units.clone(), record)
            }
        };
        let entry = carve(&mut state.pool, at, units);
        state.in_flight += 1;
        let position = self.record(record);
        let wake = self.next_wake(&state);
        (Some((entry, position)), wake)
    }

    /// Reports that a job handed out by [`Chaos::take`] has returned.
    pub(crate) fn finished(&self) {
        lock(&self.state).in_flight -= 1;
    }

    /// Whether the replay can no longer follow its script: work is pooled, nothing is
    /// running that could release more, and the job the script names next is not there. A
    /// faithful replay is never in this state for longer than it takes a worker to wake,
    /// because in the recording an idle worker took pooled work at once. Returns the
    /// script position it is stuck at.
    ///
    /// `runs_started` is how many graph runs the program has started. A record of a run it
    /// has not started yet is not stuck, however long that takes: the replay is waiting
    /// for the program, which may be slow for reasons of its own. Only graph records can
    /// be in that position. A child record's key is the job that forked it, which is
    /// earlier in the script and so has started; tasks and timers are not recorded at all.
    pub(crate) fn stalled(&self, runs_started: u64) -> Option<usize> {
        let state = lock(&self.state);
        let next = self.script.get(state.cursor)?;
        let awaits_program = next.kind != JobKind::Child && next.key >= runs_started;
        let stuck = !awaits_program
            && state.in_flight == 0
            && !state.pool.is_empty()
            && find(&state.pool, next).is_none();
        stuck.then_some(state.cursor)
    }

    /// What to say when [`Chaos::stalled`] has held for too long.
    pub(crate) fn stall_report(&self) -> String {
        let state = lock(&self.state);
        let pooled: Vec<LogRecord> =
            state.pool.iter().take(8).map(|entry| describe_whole(entry, 0)).collect();
        format!(
            "the execution log cannot be replayed by this program: record {} of {} is {:?}, \
             but no such job is available, nothing is running that could release it, and {} \
             other entries are waiting (first: {:?}). The log was recorded from a different \
             program, or the program started its graph runs in a different order.",
            state.cursor,
            self.script.len(),
            self.script.get(state.cursor),
            state.pool.len(),
            pooled,
        )
    }

    /// Drops pooled graph work, which will never run. Children stay: each has an opener
    /// waiting for it.
    pub(crate) fn discard_graph_work(&self) {
        let dropped: Vec<Entry> = {
            let mut state = lock(&self.state);
            let (children, graph_work) =
                state.pool.drain(..).partition(|entry| matches!(entry, Entry::Child { .. }));
            state.pool = children;
            graph_work
        };
        // Outside the lock: dropping an entry may drop a graph and its nodes.
        drop(dropped);
    }
}

fn passes(entry: &Entry, filter: Filter) -> bool {
    match (entry, filter) {
        (_, Filter::Any) | (Entry::Child { .. }, Filter::Children) => true,
        (Entry::Child { scope, .. }, Filter::Scope(own)) => *scope == own,
        _ => false,
    }
}

/// The scope position and fork ordinal that identify a child entry.
fn child_identity(scope: NonNull<ScopeCore>, job: NonNull<ChildHeader>) -> (u64, u32) {
    // SAFETY: a child entry exists only while its scope's latch counts its units, so the
    // opener is still inside `JobContext::scope` and both the scope and the job storage are
    // alive. The fields read are written before the entry is created and never changed.
    unsafe { ((*scope.as_ptr()).position, (*job.as_ptr()).ordinal) }
}

/// The index of the pooled entry `record` can be carved from.
fn find(pool: &[Entry], record: &LogRecord) -> Option<usize> {
    let wanted = &record.units;
    pool.iter().position(|entry| match (entry, record.kind) {
        (Entry::Range { run, node, units, .. }, JobKind::Units) => {
            run.ordinal() == record.key
                && *node == record.index
                && !wanted.is_empty()
                && units.start <= wanted.start
                && wanted.end <= units.end
        }
        (Entry::Range { run, node, units, .. }, JobKind::Finalize) => {
            run.ordinal() == record.key && *node == record.index && units.is_empty()
        }
        (Entry::Child { scope, job, units }, JobKind::Child) => {
            child_identity(*scope, *job) == (record.key, record.index)
                && units.start <= wanted.start
                && wanted.end <= units.end
        }
        _ => false,
    })
}

/// The record for taking the first unit of a pooled entry in free mode.
fn describe(entry: &Entry, worker: u32) -> LogRecord {
    let mut record = describe_whole(entry, worker);
    record.units.end = record.units.end.min(record.units.start.saturating_add(1));
    record
}

/// The record that would take a pooled entry whole.
fn describe_whole(entry: &Entry, worker: u32) -> LogRecord {
    let first = |units: &Range<u32>| units.clone();
    match entry {
        Entry::Range { run, node, units, .. } => LogRecord {
            worker,
            kind: if units.is_empty() { JobKind::Finalize } else { JobKind::Units },
            key: run.ordinal(),
            index: *node,
            units: first(units),
        },
        Entry::Child { scope, job, units } => {
            let (key, index) = child_identity(*scope, *job);
            LogRecord { worker, kind: JobKind::Child, key, index, units: first(units) }
        }
        Entry::Task(_) => unreachable!("tasks never enter the replay pool"),
    }
}

/// Removes `units` from the pooled entry at `at`, returns them as an entry of their own and
/// leaves whatever surrounds them in the pool.
fn carve(pool: &mut Vec<Entry>, at: usize, units: Range<u32>) -> Entry {
    let entry = pool.swap_remove(at);
    let whole = match &entry {
        Entry::Range { units, .. } | Entry::Child { units, .. } => units.clone(),
        Entry::Task(_) => unreachable!("tasks never enter the replay pool"),
    };
    for rest in [whole.start..units.start, units.end..whole.end] {
        if !rest.is_empty() {
            pool.push(entry.with_units(rest));
        }
    }
    entry.into_units(units)
}

#[cfg(all(test, not(feature = "loom")))]
mod tests {
    use super::*;

    fn record(worker: u32, kind: JobKind, units: Range<u32>) -> LogRecord {
        LogRecord { worker, kind, key: 7, index: 3, units }
    }

    #[test]
    fn logs_round_trip_through_bytes() {
        let records = [
            record(0, JobKind::Units, 0..4),
            record(3, JobKind::Finalize, 0..0),
            record(1, JobKind::Child, 2..3),
        ];
        let log = encode(4, &records);
        let copy = ExecutionLog::from_bytes(log.as_bytes().to_vec());
        assert_eq!(copy, log);
        assert_eq!(decode(&copy, 4).unwrap(), records);
    }

    #[test]
    fn malformed_logs_are_errors() {
        let log = encode(4, &[record(3, JobKind::Units, 0..1)]);
        let kind = |log: &ExecutionLog, workers| decode(log, workers).unwrap_err().kind();
        assert_eq!(kind(&log, 2), ErrorKind::InvalidData);
        assert_eq!(kind(&ExecutionLog::from_bytes(vec![1, 2, 3]), 4), ErrorKind::InvalidData);
        let mut truncated = log.as_bytes().to_vec();
        truncated.pop();
        assert_eq!(kind(&ExecutionLog::from_bytes(truncated), 4), ErrorKind::InvalidData);
        let mut versioned = log.as_bytes().to_vec();
        versioned[8] = 9;
        assert_eq!(kind(&ExecutionLog::from_bytes(versioned), 4), ErrorKind::VersionMismatch);
        let mut unknown = log.as_bytes().to_vec();
        unknown[HEADER_LEN + 4] = 9;
        assert_eq!(kind(&ExecutionLog::from_bytes(unknown), 4), ErrorKind::InvalidData);
    }

    #[test]
    fn rng_streams_differ_and_repeat() {
        let draw = |seed, stream| {
            let mut rng = ChaosRng::new(seed, stream);
            [rng.next(), rng.next(), rng.next()]
        };
        assert_eq!(draw(1, 0), draw(1, 0));
        assert_ne!(draw(1, 0), draw(1, 1));
        assert_ne!(draw(1, 0), draw(2, 0));
        let mut rng = ChaosRng::new(0, 0);
        assert!((0..1000).all(|_| rng.below(7) < 7));
    }
}
