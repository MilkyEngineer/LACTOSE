//! The window: what it is opened with, what it reports, and the handles a surface is made
//! from.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::ThreadId;

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};

/// A width and a height in physical pixels: the pixels a swapchain image has, never the
/// points or logical pixels a display scale divides them into. Either may be zero.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PixelSize {
    /// The width.
    pub width: u32,
    /// The height.
    pub height: u32,
}

impl PixelSize {
    /// Whether either dimension is zero. Nothing is presented at such a size: no swapchain
    /// is made or resized to it (`README.md`, "What a resize reports").
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// What the window is opened with.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WindowDesc {
    /// The title the operating system shows for the window.
    pub title: String,
    /// The size asked for the window's drawable area. The operating system may give
    /// another: [`Window::size`] is what it gave.
    pub size: PixelSize,
}

/// The process's window: what it reports to any thread, and the handles a graphics backend
/// makes its surface from.
///
/// It is reached through [`MainThread::window`](crate::MainThread::window) as an `Arc`, and
/// the operating system's window lives for as long as any such `Arc` does (`README.md`,
/// "The window and its handles"). [`Window::size`] and [`Window::close_requested`] are
/// read from any thread and never wait; [`Window::reports`] gives the same two reads in a
/// handle that keeps nothing open.
///
/// [`HasWindowHandle::window_handle`] and [`HasDisplayHandle::display_handle`] lend the
/// handles on the main thread alone, which is inside a main-lane call: on any other thread
/// each returns [`HandleError::Unavailable`], on every operating system. The handles are
/// valid for as long as the borrow of the window they were lent from.
pub struct Window {
    /// What the window reports, shared with every [`WindowReports`] and with the pump,
    /// which writes it.
    reports: Arc<Reports>,
    /// The thread that runs the pump, which is the one thread the handles are lent on.
    main_thread: ThreadId,
    /// The operating system's window. Dropping it destroys the window, which only the pump
    /// does, on the main thread, once it has taken the last `Arc<Window>` back.
    window: winit::window::Window,
}

impl Window {
    /// Wraps the window the pump opened on `main_thread`.
    pub(crate) fn new(
        window: winit::window::Window,
        reports: Arc<Reports>,
        main_thread: ThreadId,
    ) -> Self {
        Self { reports, main_thread, window }
    }

    /// The operating system's window, for the main-thread requests.
    pub(crate) fn os(&self) -> &winit::window::Window {
        &self.window
    }

    /// Reports a size the operating system gave the window.
    pub(crate) fn delivered(&self, size: PixelSize) {
        self.reports.set_size(size);
    }

    /// The size of the window's drawable area in physical pixels, as of the newest resize
    /// the operating system has delivered to the main thread. It is zero in a dimension
    /// while the operating system gives the window no area, which Windows does for a
    /// minimised window.
    ///
    /// A read never waits and returns one whole size. This is what a resize reports:
    /// there is no event to take and no callback (`README.md`, "What a resize reports").
    #[must_use]
    pub fn size(&self) -> PixelSize {
        self.reports.size()
    }

    /// Whether the operating system has asked for the window to be closed: its close
    /// button, or the operating system's own way of closing a window. `false` until the
    /// first request, or until [`EventPump::run`](crate::EventPump::run) returns with a
    /// value or an error, and `true` from then on.
    ///
    /// The window is not closed by the request, and nothing in this crate ends the
    /// process on it: the client reads this and winds down, calling
    /// [`MainThread::exit`](crate::MainThread::exit) first (`README.md`, "What a close
    /// reports" and "Shutdown").
    #[must_use]
    pub fn close_requested(&self) -> bool {
        self.reports.close_requested()
    }

    /// The window's two reports, [`size`](Self::size) and
    /// [`close_requested`](Self::close_requested), in a handle that keeps nothing open:
    /// it holds no `Arc<Window>`, so no number of them keeps
    /// [`EventPump::run`](crate::EventPump::run) from returning.
    #[must_use]
    pub fn reports(&self) -> WindowReports {
        WindowReports { reports: self.reports.clone() }
    }

    /// Whether the calling thread is the one the handles are lent on.
    fn on_main_thread(&self) -> bool {
        std::thread::current().id() == self.main_thread
    }
}

/// The window's two reports apart from the window, for whatever reads a resize and a close
/// and must not keep the window open (`README.md`, "What a resize reports" and "What a
/// close reports").
///
/// Made by [`Window::reports`]. It shares what the pump writes and holds nothing of the
/// operating system's window, so a reader may keep one in a world or a resource that
/// outlives `exit`. After the window is destroyed it reads the last size delivered, and a
/// close requested.
#[derive(Clone)]
pub struct WindowReports {
    /// The values the pump writes.
    reports: Arc<Reports>,
}

impl WindowReports {
    /// What [`Window::size`] returns; after the window is destroyed, the last size it
    /// returned.
    #[must_use]
    pub fn size(&self) -> PixelSize {
        self.reports.size()
    }

    /// What [`Window::close_requested`] returns: `true` from the first request, or from
    /// the return of [`EventPump::run`](crate::EventPump::run).
    #[must_use]
    pub fn close_requested(&self) -> bool {
        self.reports.close_requested()
    }
}

/// The two reports, written by the pump on the main thread and read on any thread.
///
/// `Relaxed` is enough for every access: each atomic is the whole of what it reports and
/// publishes nothing, since no reader reads other memory on the strength of a value it saw.
/// The one order a reader relies on, the close before a main-lane call's `Cancelled`, the
/// lane gives: the pump sets the close before it drops its pump, each cancelled reply's drop
/// then releases its slot's lock, and the call's poll acquires it (`README.md`, "The main
/// thread and its pump").
/// A main-lane call that panics breaks that order for the rest of its batch, whose calls
/// its unwinding drops before the close is set; a binary aborts first, so only a test can
/// see it.
pub(crate) struct Reports {
    /// The newest size the operating system delivered, as one value so that a read is
    /// never the width of one resize and the height of another: the width in the high 32
    /// bits, the height in the low.
    size: AtomicU64,
    /// Whether the operating system has asked for the window to be closed, or the pump
    /// has returned.
    close_requested: AtomicBool,
}

impl Reports {
    /// Reports of a window not yet opened: no area, and no close requested.
    pub(crate) const fn new() -> Self {
        Self { size: AtomicU64::new(0), close_requested: AtomicBool::new(false) }
    }

    /// Stores a size the operating system delivered.
    pub(crate) fn set_size(&self, size: PixelSize) {
        let packed = (u64::from(size.width) << 32) | u64::from(size.height);
        self.size.store(packed, Ordering::Relaxed);
    }

    pub(crate) fn size(&self) -> PixelSize {
        let packed = self.size.load(Ordering::Relaxed);
        // The truncations take the two halves the store packed.
        PixelSize { width: (packed >> 32) as u32, height: packed as u32 }
    }

    /// Turns the close report on; nothing turns it off.
    pub(crate) fn request_close(&self) {
        self.close_requested.store(true, Ordering::Relaxed);
    }

    pub(crate) fn close_requested(&self) -> bool {
        self.close_requested.load(Ordering::Relaxed)
    }
}

impl HasWindowHandle for Window {
    /// The window's handle, on the main thread; [`HandleError::Unavailable`] on any other.
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        if !self.on_main_thread() {
            return Err(HandleError::Unavailable);
        }
        // winit lends its own borrowed handle, valid for the borrow of its window, which
        // lives as long as `self`: no handle is made here from a raw one.
        self.window.window_handle()
    }
}

impl HasDisplayHandle for Window {
    /// The handle of the display the window is on, on the main thread;
    /// [`HandleError::Unavailable`] on any other.
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        if !self.on_main_thread() {
            return Err(HandleError::Unavailable);
        }
        self.window.display_handle()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use super::*;

    /// A read on another thread never sees a torn size: under a writer that alternates two
    /// sizes, which differ in both halves, every read is one of them whole (`README.md`,
    /// "What a resize reports"). Miri runs it too, where its weak-memory emulation may give
    /// a read any value the writer stored.
    #[test]
    fn a_read_never_sees_a_torn_size() {
        const WIDE: PixelSize = PixelSize { width: 1920, height: 1 };
        const TALL: PixelSize = PixelSize { width: 1, height: 1080 };
        const READS: usize = if cfg!(miri) { 200 } else { 200_000 };

        let reports = Arc::new(Reports::new());
        reports.set_size(WIDE);
        let readers: Vec<WindowReports> =
            (0..3).map(|_| WindowReports { reports: reports.clone() }).collect();
        let stop = Arc::new(AtomicBool::new(false));
        let writer = {
            let (reports, stop) = (reports.clone(), stop.clone());
            std::thread::spawn(move || {
                let mut writes = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    reports.set_size(if writes.is_multiple_of(2) { TALL } else { WIDE });
                    writes += 1;
                }
                writes
            })
        };
        // Every reader is started before any is joined, so the three read at once.
        let reading: Vec<_> = readers
            .into_iter()
            .map(|reader| {
                std::thread::spawn(move || {
                    // Natively each reader goes on until it has seen both sizes, within a
                    // bound, so the writer is shown to have alternated under it; Miri
                    // reads a fixed count, slowly enough that its scheduler interleaves.
                    let bound = std::time::Instant::now() + std::time::Duration::from_secs(10);
                    let (mut wide, mut tall) = (0usize, 0usize);
                    while wide + tall < READS
                        || (!cfg!(miri)
                            && (wide == 0 || tall == 0)
                            && std::time::Instant::now() < bound)
                    {
                        let size = reader.size();
                        assert!(size == WIDE || size == TALL, "a torn size: {size:?}");
                        if size == WIDE {
                            wide += 1;
                        } else {
                            tall += 1;
                        }
                    }
                    (wide, tall)
                })
            })
            .collect();
        let seen: Vec<_> = reading
            .into_iter()
            .map(|reader| reader.join().expect("no reader saw a torn size"))
            .collect();
        stop.store(true, Ordering::Relaxed);
        let writes = writer.join().expect("the writer does not panic");
        assert!(writes > 0, "the writer wrote");
        assert!(seen.iter().all(|(wide, tall)| wide + tall >= READS));
        if !cfg!(miri) {
            assert!(
                seen.iter().all(|&(wide, tall)| wide > 0 && tall > 0),
                "each reader saw both sizes: {seen:?}"
            );
        }
    }

    #[test]
    fn a_close_once_requested_stays_requested() {
        let reports = Reports::new();
        assert!(!reports.close_requested());
        assert_eq!(reports.size(), PixelSize { width: 0, height: 0 });
        reports.request_close();
        reports.request_close();
        assert!(reports.close_requested());
        let extreme = PixelSize { width: u32::MAX, height: 0 };
        reports.set_size(extreme);
        assert_eq!(reports.size(), extreme);
        reports.set_size(PixelSize { width: 0, height: u32::MAX });
        assert_eq!(reports.size(), PixelSize { width: 0, height: u32::MAX });
    }
}
