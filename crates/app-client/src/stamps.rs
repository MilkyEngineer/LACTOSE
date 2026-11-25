//! `frame-pace`'s stamps (`README.md`, "Stamps"; `crates/app/README.md`, "Pacing"): the
//! instants the paced mode's root, the render side and the present side take of each frame
//! and each present call, kept in memory while a run takes them and written to a file once
//! its frames end.
//!
//! Every site loads [`on`] first, one `Relaxed` load, and takes no instant and no lock while
//! it is `false`, which it is unless a run asked for stamps. The flag orders nothing: the
//! stamps themselves go through a lock, and a stamp that races [`finish`] is kept or lost
//! with nothing read from it. While it is `true` each stamp takes that lock, a cost every
//! figure of a stamped run includes.

use std::cell::{Cell, RefCell};
use std::fmt::Write as _;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use lactose_app::{Rendered, ShownStats};
use lactose_core::{Error, ErrorKind, Result};
use tracing::Subscriber;
use tracing::span::Id;
use tracing_subscriber::Layer;
use tracing_subscriber::filter::{LevelFilter, filter_fn};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;

/// Whether a run takes stamps.
static ON: AtomicBool = AtomicBool::new(false);
/// Whether [`install_log`] installed the recorders' layer.
static RECORDERS: AtomicBool = AtomicBool::new(false);
/// The stamps taken since [`start`].
static STAMPS: Mutex<Vec<Stamp>> = Mutex::new(Vec::new());
/// The next thread's number.
static THREADS: AtomicU32 = AtomicU32::new(0);

thread_local! {
    /// This thread's number, given at its first stamp.
    static THREAD: Cell<Option<u32>> = const { Cell::new(None) };
    /// The instants this thread entered the recorders' spans it is inside, innermost last.
    static ENTERED: RefCell<Vec<Instant>> = const { RefCell::new(Vec::new()) };
}

/// Room for some four and a half minutes' stamps at 60 frames a second and the eight or so
/// stamps a frame a run takes, so a run of that length allocates while it takes none.
const ROOM: usize = 1 << 17;

/// One stamp.
#[derive(Clone, Debug)]
pub(crate) enum Stamp {
    /// A figure the run's others are read beside, by name.
    Info(&'static str, String),
    /// A paced frame, taken by the root.
    Frame(FrameStamp),
    /// The start of a frame's job: `RenderSide::present`'s entry.
    Job {
        /// The thread that ran it.
        thread: u32,
        /// When.
        start: Instant,
    },
    /// `RenderSide::draw`'s return.
    Drew(Instant),
    /// A closure of the record on a frame-critical worker.
    Closure {
        /// `recorder`, a pass of the render graph from its span's entry to its exit, or
        /// `writer`, the remote batch's writer closure.
        kind: &'static str,
        /// The thread that ran it.
        thread: u32,
        /// Its entry.
        entered: Instant,
        /// Its return.
        left: Instant,
    },
    /// A frame's `RenderSide::submit` that succeeded.
    Submit {
        /// Its entry.
        entered: Instant,
        /// The latch: just before the camera latch reads the input.
        latched: Instant,
        /// Its return.
        returned: Instant,
        /// The frame's point's value.
        point: u64,
    },
    /// A present call.
    Present(PresentStamp),
    /// A display time the swapchain reported.
    Displayed {
        /// The present's id.
        id: u64,
        /// When it reached the display.
        at: Instant,
    },
    /// The swapchain's image count, at the first call and after each change.
    Images(u32),
    /// `Shown::stats` as the frames ended.
    Stats(ShownStats),
}

/// A paced frame, as the root took it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FrameStamp {
    /// The deadline the root slept to before it.
    pub deadline: Instant,
    /// The first tick's start, taken in the frame callback; `None` where no tick ran.
    pub first_tick: Option<Instant>,
    /// `Shown::frame`'s call.
    pub called: Instant,
    /// Its return.
    pub returned: Instant,
    /// `ShownOutput::joined`.
    pub joined: Option<Rendered>,
    /// `ShownOutput::deferred`.
    pub deferred: Option<Rendered>,
    /// `FrameOutput::behind`.
    pub behind: bool,
}

/// A present call, as the present side took it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PresentStamp {
    /// Its start.
    pub began: Instant,
    /// The time of its own waits: for its last read, the resize, and, where the side holds
    /// its bound of encoders, the oldest one's read. None waits for this present's submit.
    pub waits: Duration,
    /// The acquire's time.
    pub acquire: Duration,
    /// The time of the encoder's reset or creation and the blit's recording.
    pub record: Duration,
    /// The time of `Swapchain::present`: the wait for the image's last-present fence, then
    /// the driver's submit and present under the queue's lock.
    pub driver: Duration,
    /// When `Swapchain::present` was called, `None` where the call made none.
    pub driver_at: Option<Instant>,
    /// Its return.
    pub returned: Instant,
    /// The `waiting` it reported, `None` where it reported none or failed.
    pub waiting: Option<u32>,
    /// Whether the swapchain has a source of display times.
    pub source: bool,
    /// The value of the point of the frame whose target it read.
    pub point: u64,
    /// The id of the present it made, `None` where it made none.
    pub id: Option<u64>,
    /// Whether it reported `shown`.
    pub shown: bool,
}

/// Whether a run takes stamps: one `Relaxed` load.
#[inline]
pub(crate) fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// The instant now where a run takes stamps, `None` with no instant taken where it does not.
#[inline]
pub(crate) fn now() -> Option<Instant> {
    on().then(Instant::now)
}

/// Keeps `stamp` where a run takes stamps.
pub(crate) fn stamp(stamp: Stamp) {
    if on() {
        STAMPS.lock().unwrap_or_else(PoisonError::into_inner).push(stamp);
    }
}

/// This thread's number, given at its first call.
pub(crate) fn thread() -> u32 {
    THREAD.with(|number| {
        number.get().unwrap_or_else(|| {
            let given = THREADS.fetch_add(1, Ordering::Relaxed);
            number.set(Some(given));
            given
        })
    })
}

/// Runs `f`, a closure of the record, stamped as `kind` where a run takes stamps.
pub(crate) fn closure<R>(kind: &'static str, f: impl FnOnce() -> R) -> R {
    let Some(entered) = now() else { return f() };
    let returned = f();
    stamp(Stamp::Closure { kind, thread: thread(), entered, left: Instant::now() });
    returned
}

/// Starts taking stamps, every earlier one dropped, and returns the instant the file's times
/// count from.
pub(crate) fn start() -> Instant {
    let mut stamps = STAMPS.lock().unwrap_or_else(PoisonError::into_inner);
    stamps.clear();
    stamps.reserve(ROOM);
    ON.store(true, Ordering::Relaxed);
    Instant::now()
}

/// Stops taking stamps and writes those taken to `path`, each instant as nanoseconds since
/// `epoch`.
///
/// # Errors
///
/// `Io` where the file cannot be written.
pub(crate) fn finish(path: &Path, epoch: Instant) -> Result<()> {
    std::fs::write(path, taken(epoch)).map_err(|error| {
        Error::new(ErrorKind::Io, format!("the stamps file {}: {error}", path.display()))
    })
}

/// Stops taking stamps and returns the file's text of those taken.
fn taken(epoch: Instant) -> String {
    ON.store(false, Ordering::Relaxed);
    let stamps = core::mem::take(&mut *STAMPS.lock().unwrap_or_else(PoisonError::into_inner));
    text(&stamps, epoch)
}

/// Nanoseconds from `epoch` to `at`, zero for an instant before it.
fn nanos(at: Instant, epoch: Instant) -> u128 {
    at.saturating_duration_since(epoch).as_nanos()
}

/// A frame's outcome as the file names it.
fn word(rendered: Option<Rendered>) -> &'static str {
    match rendered {
        None => "-",
        Some(Rendered::Drew { queued: true, .. }) => "queued",
        Some(Rendered::Drew { held: true, .. }) => "held",
        Some(Rendered::Drew { shed: true, .. }) => "shed",
        Some(Rendered::Drew { .. }) => "dropped",
        Some(Rendered::Hidden) => "hidden",
        Some(Rendered::Waited(_)) => "waited",
        Some(Rendered::Torn) => "torn",
        Some(Rendered::Busy) => "busy",
    }
}

/// The file's text: one line a stamp, in the order taken, each a kind and its fields
/// separated by spaces, an absent value written `-` (`README.md`, "Stamps").
fn text(stamps: &[Stamp], epoch: Instant) -> String {
    let at = |instant: Instant| nanos(instant, epoch);
    let or = |value: Option<String>| value.unwrap_or_else(|| "-".to_owned());
    let mut out = String::new();
    for stamp in stamps {
        // Writing to a `String` does not fail.
        let _ = match stamp {
            Stamp::Info(name, value) => writeln!(out, "info {name} {value}"),
            Stamp::Frame(frame) => writeln!(
                out,
                "frame {} {} {} {} {} {} {}",
                at(frame.deadline),
                or(frame.first_tick.map(|first| at(first).to_string())),
                at(frame.called),
                at(frame.returned),
                word(frame.joined),
                word(frame.deferred),
                u8::from(frame.behind),
            ),
            Stamp::Job { thread, start } => writeln!(out, "job {thread} {}", at(*start)),
            Stamp::Drew(drew) => writeln!(out, "drew {}", at(*drew)),
            Stamp::Closure { kind, thread, entered, left } => {
                writeln!(out, "closure {kind} {thread} {} {}", at(*entered), at(*left))
            }
            Stamp::Submit { entered, latched, returned, point } => {
                writeln!(out, "submit {} {} {} {point}", at(*entered), at(*latched), at(*returned))
            }
            Stamp::Present(call) => writeln!(
                out,
                "present {} {} {} {} {} {} {} {} {} {} {} {}",
                at(call.began),
                call.waits.as_nanos(),
                call.acquire.as_nanos(),
                call.record.as_nanos(),
                call.driver.as_nanos(),
                or(call.driver_at.map(|driver| at(driver).to_string())),
                at(call.returned),
                or(call.waiting.map(|waiting| waiting.to_string())),
                u8::from(call.source),
                call.point,
                or(call.id.map(|id| id.to_string())),
                u8::from(call.shown),
            ),
            Stamp::Displayed { id, at: shown } => writeln!(out, "displayed {id} {}", at(*shown)),
            Stamp::Images(count) => writeln!(out, "images {count}"),
            Stamp::Stats(stats) => writeln!(
                out,
                "stats drawn={} hidden={} waited={} busy={} torn={} queued={} held={} shed={} \
                 shown={} resets={}",
                stats.drawn,
                stats.hidden,
                stats.waited,
                stats.busy,
                stats.torn,
                stats.queued,
                stats.held,
                stats.shed,
                stats.shown,
                stats.resets,
            ),
        };
    }
    out
}

/// The layer that stamps each recorder of `lactose-render-graph`: the graph enters a span
/// named `pass` at its target around each kept pass's transitions, beginning, recorder and
/// end, on the worker that records it.
struct Recorders;

impl<S: Subscriber> Layer<S> for Recorders {
    fn on_enter(&self, _: &Id, _: Context<'_, S>) {
        if on() {
            ENTERED.with(|entered| entered.borrow_mut().push(Instant::now()));
        }
    }

    fn on_exit(&self, _: &Id, _: Context<'_, S>) {
        // Popped whether or not stamps are taken now, so an entry pushed before a run
        // stopped taking them leaves no stamp and pairs with no later exit.
        let entered = ENTERED.with(|entered| entered.borrow_mut().pop());
        if let Some(entered) = entered {
            stamp(Stamp::Closure {
                kind: "recorder",
                thread: thread(),
                entered,
                left: Instant::now(),
            });
        }
    }
}

/// Whether the recorders' layer of [`install_log`] is installed, which the stamps file
/// records.
pub(crate) fn recorders() -> bool {
    RECORDERS.load(Ordering::Relaxed)
}

/// Installs the binary's log as the process's `tracing` subscriber: every event at `INFO` or
/// above, formatted to stderr. With `stamps`, also the layer that stamps each recorder of
/// `lactose-render-graph` from the entry to the exit of its pass's span, which the paced
/// mode's stamps read; without it the subscriber has no such layer and the graph's spans cost
/// what they cost with no subscriber that wants them. Returns whether it installed the
/// subscriber: it must be the process's first, since a subscriber already set is left in
/// place, and a paced run that takes stamps then writes `info recorders absent` and carries
/// no recorder. Called once, first in `main` after the crash hook.
#[must_use]
pub fn install_log(stamps: bool) -> bool {
    let log = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(LevelFilter::INFO);
    let recorders = stamps.then(|| {
        Recorders.with_filter(filter_fn(|metadata| {
            metadata.is_span()
                && metadata.target() == lactose_render_graph::LOG_TARGET
                && metadata.name() == "pass"
        }))
    });
    let installed = tracing_subscriber::registry().with(log).with(recorders).try_init().is_ok();
    if installed {
        RECORDERS.store(stamps, Ordering::Relaxed);
    }
    installed
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use lactose_app::{Rendered, ShownStats};

    use tracing_subscriber::util::SubscriberInitExt;

    use super::{
        FrameStamp, PresentStamp, Stamp, closure, install_log, now, on, recorders, stamp, start,
        taken, text, word,
    };

    /// A subscriber set before `install_log` is left in place: the call says so, and the
    /// stamps then record no recorders' layer.
    #[test]
    fn a_subscriber_set_first_leaves_the_recorders_absent() {
        assert!(!recorders());
        tracing_subscriber::registry().try_init().expect("the first subscriber");
        assert!(!install_log(true));
        assert!(!recorders());
    }

    /// The one test that sets the flag, so no other test of the process meets it set: unset,
    /// a site keeps nothing; set, a stamp of another thread is kept after one taken before it
    /// was spawned; once taken, the flag is unset and nothing more is kept.
    #[test]
    fn stamps_are_kept_only_while_a_run_takes_them() {
        assert!(!on() && now().is_none());
        stamp(Stamp::Drew(Instant::now()));
        assert_eq!(closure("writer", || 7), 7);
        let epoch = start();
        assert!(on());
        stamp(Stamp::Images(3));
        std::thread::spawn(|| closure("writer", || ())).join().expect("the thread");
        let written = taken(epoch);
        let lines: Vec<&str> = written.lines().collect();
        assert_eq!(lines.len(), 2, "{written}");
        assert_eq!(lines[0], "images 3");
        assert!(lines[1].starts_with("closure writer "), "{written}");
        assert!(!on());
        stamp(Stamp::Images(4));
        assert_eq!(taken(epoch), "");
    }

    #[test]
    fn each_stamp_is_one_line_of_its_kind_and_fields() {
        let epoch = Instant::now();
        let ms = |n: u64| epoch + Duration::from_millis(n);
        let stamps = [
            Stamp::Info("frame_workers", "3".to_owned()),
            Stamp::Frame(FrameStamp {
                deadline: ms(1),
                first_tick: Some(ms(2)),
                called: ms(2),
                returned: ms(3),
                joined: None,
                deferred: Some(Rendered::Busy),
                behind: true,
            }),
            Stamp::Job { thread: 4, start: ms(5) },
            Stamp::Drew(ms(6)),
            Stamp::Closure { kind: "recorder", thread: 1, entered: ms(5), left: ms(6) },
            Stamp::Submit { entered: ms(7), latched: ms(8), returned: ms(9), point: 12 },
            Stamp::Present(PresentStamp {
                began: ms(10),
                waits: Duration::from_nanos(1),
                acquire: Duration::from_nanos(2),
                record: Duration::from_nanos(4),
                driver: Duration::from_nanos(3),
                driver_at: Some(ms(10)),
                returned: ms(11),
                waiting: None,
                source: true,
                point: 12,
                id: Some(40),
                shown: true,
            }),
            Stamp::Displayed { id: 40, at: ms(30) },
            Stamp::Images(3),
            Stamp::Stats(ShownStats { drawn: 1, shown: 1, ..ShownStats::default() }),
        ];
        let written = text(&stamps, epoch);
        let lines: Vec<&str> = written.lines().collect();
        assert_eq!(
            lines,
            [
                "info frame_workers 3",
                "frame 1000000 2000000 2000000 3000000 - busy 1",
                "job 4 5000000",
                "drew 6000000",
                "closure recorder 1 5000000 6000000",
                "submit 7000000 8000000 9000000 12",
                "present 10000000 1 2 4 3 10000000 11000000 - 1 12 40 1",
                "displayed 40 30000000",
                "images 3",
                "stats drawn=1 hidden=0 waited=0 busy=0 torn=0 queued=0 held=0 shed=0 shown=1 \
                 resets=0",
            ]
        );
    }

    #[test]
    fn a_frame_that_drew_is_named_by_what_its_present_did() {
        let drew = |queued, held, shed| Rendered::Drew {
            target: lactose_app::TargetIndex::new(0).expect("a target"),
            point: lactose_rhi_api::TimelinePoint {
                timeline: lactose_core::Handle::from_parts(0, core::num::NonZeroU32::MIN),
                value: 1,
            },
            counts: lactose_app::FrameCounts::default(),
            queued,
            held,
            shed,
        };
        assert_eq!(word(Some(drew(true, false, false))), "queued");
        assert_eq!(word(Some(drew(false, true, false))), "held");
        assert_eq!(word(Some(drew(false, false, true))), "shed");
        assert_eq!(word(Some(drew(false, false, false))), "dropped");
        assert_eq!(word(Some(Rendered::Hidden)), "hidden");
        assert_eq!(word(Some(Rendered::Torn)), "torn");
        assert_eq!(word(None), "-");
    }
}
