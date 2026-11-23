//! The main thread: the event pump that owns it, and the state a main-lane call is lent.

use core::marker::PhantomData;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::sync::Arc;
use std::thread::ThreadId;
use std::time::Instant;

use lactose_core::{Error, ErrorKind, Result};
use lactose_task::{Lane, LanePump, lane};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::error::EventLoopError;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::WindowId;

use crate::LOG_TARGET;
use crate::window::{PixelSize, Reports, Window, WindowDesc};

/// How often the pump looks for the last `Arc<Window>` between `exit` and its return.
const LOOK: Duration = Duration::from_millis(50);
/// How often the pump warns while it waits for the window to be let go.
const WARN: Duration = Duration::from_secs(1);
/// How long the pump waits for the operating system to report the window destroyed once it
/// has dropped it, before it returns regardless. The report is what tells the pump that a
/// destruction the operating system defers, as Windows does, has been carried out.
const DESTROYED: Duration = Duration::from_secs(1);

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
/// has finished, and whether or not it panicked; `ErrorKind::Platform` where the operating
/// system gives the process no window system to reach.
///
/// # Panics
///
/// On any thread but the process's main thread, on every operating system. That call is
/// the process's one call, and every later call is `Unsupported`.
pub fn event_pump(desc: WindowDesc) -> Result<(Lane<MainThread>, EventPump)> {
    // Taken before winit is reached, so that a second call is refused here on every
    // operating system, whatever winit does with it.
    static MADE: AtomicBool = AtomicBool::new(false);
    if MADE.swap(true, Ordering::Relaxed) {
        return Err(Error::new(ErrorKind::Unsupported, "a process has one event pump"));
    }
    // winit panics on any thread but the process's main one, on macOS, Windows and Linux,
    // before it looks for a window system (docs/verification.md, winit's behaviours).
    let event_loop = EventLoop::<Wake>::with_user_event().build().map_err(|error| match error {
        EventLoopError::RecreationAttempt => {
            Error::new(ErrorKind::Unsupported, "the process has made a winit event loop already")
        }
        error => Error::new(ErrorKind::Platform, format!("no window system to reach: {error}")),
    })?;
    // The proxy is `Send + Sync`, so the wake takes no lock of this crate's. Once the event
    // loop has ended it refuses the wake, which is ignored: `exit` is queued from
    // destructors, unwinding included, and `Lane::run` wakes after it releases the queue.
    let proxy = event_loop.create_proxy();
    let (lane, pump) = lane("main", move || {
        let _ = proxy.send_event(Wake);
    });
    let state = MainThread { window: None, exit_requested: false, not_send: PhantomData };
    let reports = Arc::new(Reports::new());
    let main_thread = std::thread::current().id();
    Ok((lane, EventPump { pump, state, desc, reports, main_thread, event_loop }))
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
    reports: Arc<Reports>,
    /// The thread that made the pump, which is the process's main thread.
    main_thread: ThreadId,
    /// The operating system's event loop, which `run` gives the main thread to.
    event_loop: EventLoop<Wake>,
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
        let Self { pump, state, desc, reports, main_thread, event_loop } = self;
        let mut pumping = Pumping {
            pump,
            state,
            desc,
            reports,
            main_thread,
            stage: Stage::Opening,
            refused: None,
            next_warning: None,
        };
        let ran = event_loop.run_app(&mut pumping);
        let outcome = match (pumping.refused.take(), ran) {
            (Some(refused), _) => Err(refused),
            (None, Err(error)) => Err(Error::new(
                ErrorKind::Platform,
                format!("the operating system's event loop failed: {error}"),
            )),
            (None, Ok(())) if !matches!(pumping.stage, Stage::Closed) => Err(Error::new(
                ErrorKind::Platform,
                "the operating system's event loop ended before the window was let go",
            )),
            (None, Ok(())) => Ok(()),
        };
        // The close report first, then the lane: see `Pumping`'s `Drop`.
        drop(pumping);
        outcome
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
        self.open().clone()
    }

    /// Asks the operating system to give the window's drawable area `size`. The answer
    /// is a resize report or none: the operating system may give another size, or leave
    /// the window as it is.
    pub fn request_size(&mut self, size: PixelSize) {
        let window = self.open();
        let asked = PhysicalSize::new(size.width, size.height);
        // winit returns a size it applied at once, for which no resize event follows: that
        // is the operating system's delivery of the resize, and is reported as one.
        if let Some(given) = window.os().request_inner_size(asked) {
            window.delivered(pixels(given));
        }
    }

    /// Asks the operating system to minimise the window, or to restore it. The answer is
    /// what [`is_minimised`](Self::is_minimised) reports, or none.
    pub fn set_minimised(&mut self, minimised: bool) {
        self.open().os().set_minimized(minimised);
    }

    /// Whether the operating system reports the window minimised now, or `None` where it
    /// does not say. A request of [`set_minimised`](Self::set_minimised) that the
    /// operating system refused, or has not yet carried out, reads as the window was.
    #[must_use]
    pub fn is_minimised(&self) -> Option<bool> {
        self.open().os().is_minimized()
    }

    /// Tells the pump that the process is done with the window. It destroys nothing
    /// while an `Arc<Window>` is held: [`EventPump::run`] returns once the window is let
    /// go, and until then it pumps events and drains calls as before, so a call queued
    /// after this one still runs. A second call does nothing.
    pub fn exit(&mut self) {
        self.exit_requested = true;
    }

    /// The pump's own `Arc`. A call is drained only while the window is open.
    fn open(&self) -> &Arc<Window> {
        self.window.as_ref().expect("a main-lane call runs only while the window is open")
    }
}

/// The payload of the wake: the calls themselves are in the lane.
struct Wake;

/// Where the pump is between `run`'s start and its return.
enum Stage {
    /// The window is not open yet, and no call is drained.
    Opening,
    /// The window is open, and calls are drained.
    Open,
    /// The window was let go and dropped at `since`; the pump waits for the operating
    /// system's report that it is destroyed, and drains nothing.
    Closing { since: Instant },
    /// The event loop was told to exit: the window is destroyed, or was refused.
    Closed,
}

/// The pump while it runs: what the operating system's event loop calls back.
///
/// Its `Drop` runs before its fields are dropped, so the close report is turned on before
/// the lane's pump is dropped and closes the lane, on every path out of `run`: a return
/// with a value or an error, and a panic's unwinding. A window still held then is leaked,
/// never destroyed, so the handles a surface on another thread was made from stay valid
/// until the process ends.
struct Pumping {
    pump: LanePump<MainThread>,
    state: MainThread,
    desc: WindowDesc,
    reports: Arc<Reports>,
    main_thread: ThreadId,
    stage: Stage,
    /// Why the window was refused, if it was.
    refused: Option<Error>,
    /// When the next warning is due, from the first look after `exit`.
    next_warning: Option<Instant>,
}

impl Pumping {
    /// Drains the lane once the window is open, then looks for the window's last `Arc`
    /// if `exit` has been called.
    fn turn(&mut self, event_loop: &ActiveEventLoop) {
        match self.stage {
            Stage::Open => {
                self.pump.drain(&mut self.state);
                if self.state.exit_requested {
                    self.let_go(event_loop);
                }
            }
            Stage::Closing { since } => {
                if since.elapsed() >= DESTROYED {
                    tracing::warn!(
                        target: LOG_TARGET,
                        "the operating system did not report the window destroyed within {DESTROYED:?}"
                    );
                    self.stage = Stage::Closed;
                    event_loop.exit();
                } else {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(since + DESTROYED));
                }
            }
            Stage::Opening | Stage::Closed => {}
        }
    }

    /// Takes the window back if no other `Arc` is held, and destroys it; otherwise keeps
    /// it, looks again within 50 ms, and warns once a second.
    fn let_go(&mut self, event_loop: &ActiveEventLoop) {
        let window = self.state.window.take().expect("the window is open");
        match Arc::try_unwrap(window) {
            Ok(window) => {
                drop(window);
                let since = Instant::now();
                self.stage = Stage::Closing { since };
                event_loop.set_control_flow(ControlFlow::WaitUntil(since + DESTROYED));
            }
            Err(window) => {
                let now = Instant::now();
                let due = *self.next_warning.get_or_insert(now + WARN);
                if now >= due {
                    // The count is read for the warning alone, never to decide the drop.
                    tracing::warn!(
                        target: LOG_TARGET,
                        held = Arc::strong_count(&window) - 1,
                        "the pump was told to exit and waits for the window to be let go"
                    );
                    self.next_warning = Some(now + WARN);
                }
                self.state.window = Some(window);
                event_loop.set_control_flow(ControlFlow::WaitUntil(now + LOOK));
            }
        }
    }
}

impl ApplicationHandler<Wake> for Pumping {
    /// Opens the window, once, and drains what was queued before `run`.
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if !matches!(self.stage, Stage::Opening) {
            return;
        }
        let attributes = winit::window::Window::default_attributes()
            .with_title(self.desc.title.clone())
            .with_inner_size(PhysicalSize::new(self.desc.size.width, self.desc.size.height))
            .with_resizable(true)
            .with_visible(true);
        match event_loop.create_window(attributes) {
            Ok(window) => {
                self.reports.set_size(pixels(window.inner_size()));
                let window = Window::new(window, self.reports.clone(), self.main_thread);
                self.state.window = Some(Arc::new(window));
                self.stage = Stage::Open;
                self.turn(event_loop);
            }
            Err(error) => {
                self.refused = Some(Error::new(
                    ErrorKind::Platform,
                    format!("the operating system refused the window: {error}"),
                ));
                self.stage = Stage::Closed;
                event_loop.exit();
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, _: Wake) {
        self.turn(event_loop);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::Resized(size) => self.reports.set_size(pixels(size)),
            WindowEvent::CloseRequested => self.reports.request_close(),
            WindowEvent::Destroyed => {
                if matches!(self.stage, Stage::Closing { .. }) {
                    self.stage = Stage::Closed;
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.turn(event_loop);
    }
}

impl Drop for Pumping {
    fn drop(&mut self) {
        self.reports.request_close();
        if let Some(window) = self.state.window.take() {
            // Never destroyed: a surface made from it on another thread outlives `run`.
            core::mem::forget(window);
        }
    }
}

/// A size as winit gives it, in physical pixels.
fn pixels(size: PhysicalSize<u32>) -> PixelSize {
    PixelSize { width: size.width, height: size.height }
}
