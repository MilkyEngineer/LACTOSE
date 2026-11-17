//! The window: what it is opened with, what it reports, and the handles a surface is made
//! from.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64};
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
}

impl Window {
    /// The size of the window's drawable area in physical pixels, as of the newest resize
    /// the operating system has delivered to the main thread. It is zero in a dimension
    /// while the operating system gives the window no area, which Windows does for a
    /// minimised window.
    ///
    /// A read never waits and returns one whole size. This is what a resize reports:
    /// there is no event to take and no callback (`README.md`, "What a resize reports").
    #[must_use]
    pub fn size(&self) -> PixelSize {
        todo!()
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
        todo!()
    }

    /// The window's two reports, [`size`](Self::size) and
    /// [`close_requested`](Self::close_requested), in a handle that keeps nothing open:
    /// it holds no `Arc<Window>`, so no number of them keeps
    /// [`EventPump::run`](crate::EventPump::run) from returning.
    #[must_use]
    pub fn reports(&self) -> WindowReports {
        todo!()
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
        todo!()
    }

    /// What [`Window::close_requested`] returns: `true` from the first request, or from
    /// the return of [`EventPump::run`](crate::EventPump::run).
    #[must_use]
    pub fn close_requested(&self) -> bool {
        todo!()
    }
}

/// The two reports, written by the pump on the main thread and read on any thread.
///
/// `Relaxed` is enough for every access: each atomic is the whole of what it reports and
/// publishes nothing, since no reader reads other memory on the strength of a value it saw.
struct Reports {
    /// The newest size the operating system delivered, as one value so that a read is
    /// never the width of one resize and the height of another.
    size: AtomicU64,
    /// Whether the operating system has asked for the window to be closed, or the pump
    /// has returned.
    close_requested: AtomicBool,
}

impl HasWindowHandle for Window {
    /// The window's handle, on the main thread; [`HandleError::Unavailable`] on any other.
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        todo!()
    }
}

impl HasDisplayHandle for Window {
    /// The handle of the display the window is on, on the main thread;
    /// [`HandleError::Unavailable`] on any other.
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        todo!()
    }
}
