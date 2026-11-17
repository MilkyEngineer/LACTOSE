//! The main thread: the event pump that owns it, and the state a main-lane call is lent.

use core::marker::PhantomData;
use std::sync::Arc;

use lactose_core::Result;
use lactose_task::{Lane, LanePump};

use crate::window::{PixelSize, Window, WindowDesc, WindowReports};

/// Makes the process's event pump and the lane that reaches the main thread.
///
/// Call it on the process's main thread, once. Nothing is shown yet: the window `desc`
/// describes is opened when [`EventPump::run`] starts. The [`Lane`] is the main-thread
/// lane of architecture section 5.1, and its clones go to whatever asks something of the
/// main thread; a call queued before `run` waits for it.
///
/// # Errors
///
/// `ErrorKind::Unsupported` for a second pump in one process, whether or not the first
/// has finished; `ErrorKind::Platform` where the operating system gives the process no
/// window system to reach.
///
/// # Panics
///
/// On any thread but the process's main thread, on every operating system.
pub fn event_pump(desc: WindowDesc) -> Result<(Lane<MainThread>, EventPump)> {
    todo!()
}

/// The main thread's loop: it pumps the operating system's events, keeps the window's
/// reports current and drains the main-thread lane. It is neither `Send` nor `Sync`: the
/// thread that made it runs it.
pub struct EventPump {
    /// The receiving side of the main-thread lane.
    pump: LanePump<MainThread>,
    /// What each drained call is lent.
    state: MainThread,
    /// What the window is opened with when `run` starts.
    desc: WindowDesc,
    /// The window's reports, which the pump writes, and which it turns closed when `run`
    /// returns, whether or not the window was destroyed.
    reports: WindowReports,
}

impl EventPump {
    /// Opens the window, then pumps events and drains the lane until the process is done
    /// with the window: it returns once [`MainThread::exit`] has been called and the
    /// window is let go, which is when every `Arc<Window>` that [`MainThread::window`]
    /// returned, and every clone of one, has been dropped. It takes the window back with
    /// `Arc::try_unwrap` on its own `Arc`, never by reading a count and never by
    /// `Arc::into_inner`, which gives up that `Arc` when it fails, and destroys it on this
    /// thread before it returns. It waits in the operating system between events and
    /// never for another thread; from `exit` until the window is let go it also wakes
    /// every 50 ms to look (`README.md`, "The main thread and its pump" and "Shutdown").
    ///
    /// When it returns, with a value or an error, [`Window::close_requested`] is `true`,
    /// and every call still queued, and every call queued later, resolves to
    /// `ErrorKind::Cancelled`. It turns `close_requested` true before it closes the lane,
    /// so a task whose call came back `Cancelled` reads the close. The lane's wake ignores
    /// an event loop that has ended, even after the lane is closed, since `exit` is queued
    /// from destructors and `Lane::run` wakes after it releases the queue. A panic that
    /// unwinds out of it leaves the window to the end of the process, as an error does.
    ///
    /// # Errors
    ///
    /// `ErrorKind::Platform` if the operating system refuses the window, before any call
    /// is drained, or if its event loop fails. Either way the lane's calls resolve to
    /// `Cancelled`, and a window still held is never destroyed.
    pub fn run(self) -> Result<()> {
        todo!()
    }
}

/// What a main-lane call is lent: the main thread's state, for the length of the call.
///
/// Only the pump makes one, and only a call drained by [`EventPump::run`] sees it, so
/// holding a `&mut MainThread` is being on the main thread while the window exists.
pub struct MainThread {
    /// The window, from the start of `run` to its return: what `window` hands out. This is
    /// the pump's own `Arc`, which `run` takes back with `Arc::try_unwrap`.
    window: Option<Arc<Window>>,
    /// Whether `exit` has been called.
    exit_requested: bool,
    /// Main-thread state never leaves the main thread.
    not_send: PhantomData<*const ()>,
}

impl MainThread {
    /// The window, to hand to a graphics backend as the window its device presents to.
    /// Every call returns the one window, and the pump runs until every `Arc` it returned,
    /// and every clone of one, has been dropped. Whatever only reads the window's reports
    /// takes [`Window::reports`] here and drops the `Arc`.
    #[must_use]
    pub fn window(&self) -> Arc<Window> {
        todo!()
    }

    /// Asks the operating system to give the window's drawable area `size`. The answer
    /// is a resize report or none: the operating system may give another size, or leave
    /// the window as it is.
    pub fn request_size(&mut self, size: PixelSize) {
        todo!()
    }

    /// Asks the operating system to minimise the window, or to restore it. The answer is
    /// what [`is_minimised`](Self::is_minimised) reports, or none.
    pub fn set_minimised(&mut self, minimised: bool) {
        todo!()
    }

    /// Whether the operating system reports the window minimised now, or `None` where it
    /// does not say. A request of [`set_minimised`](Self::set_minimised) that the
    /// operating system refused, or has not yet carried out, reads as the window was.
    #[must_use]
    pub fn is_minimised(&self) -> Option<bool> {
        todo!()
    }

    /// Tells the pump that the process is done with the window. It destroys nothing
    /// while an `Arc<Window>` is held: [`EventPump::run`] returns once the window is let
    /// go, and until then it pumps events and drains calls as before, so a call queued
    /// after this one still runs. A second call does nothing.
    pub fn exit(&mut self) {
        todo!()
    }
}
