//! The window: what it is opened with, what it reports, and the handles a surface is made
//! from.

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
/// read from any thread and never wait.
///
/// [`HasWindowHandle::window_handle`] and [`HasDisplayHandle::display_handle`] lend the
/// handles on the main thread alone, which is inside a main-lane call: on any other thread
/// each returns [`HandleError::Unavailable`], on every operating system. The handles are
/// valid for as long as the borrow of the window they were lent from.
pub struct Window {
    /// The newest size the operating system delivered, as one value so that a read is
    /// never the width of one resize and the height of another.
    size: AtomicU64,
    /// Whether the operating system has asked for the window to be closed.
    close_requested: AtomicBool,
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
    /// first request and `true` from then on.
    ///
    /// The window is not closed by the request, and nothing in this crate ends the
    /// process on it: the client reads this, winds down and calls
    /// [`MainThread::exit`](crate::MainThread::exit) (`README.md`, "What a close reports").
    #[must_use]
    pub fn close_requested(&self) -> bool {
        todo!()
    }
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
