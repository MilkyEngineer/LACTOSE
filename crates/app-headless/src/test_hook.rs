//! The binary's test hook, `LACTOSE_TEST_PANIC` (README, "Usage"): a frame callback that
//! panics in place of the run's, so that `tests/crash.rs` sees the crash hook report and
//! abort in a subprocess. Unset or empty, nothing here runs.

use std::fmt;
use std::sync::{Mutex, mpsc};
use std::time::Duration;

use lactose_core::Tick;
use lactose_sim::TickInput;

/// The variable. Its value names the panic: `stderr-held`, `stderr-blocked`, or any other
/// value for a plain panic.
const VARIABLE: &str = "LACTOSE_TEST_PANIC";

/// The callback that panics on its first call, on the frame worker that runs the timer root,
/// when [`VARIABLE`] is set and not empty.
pub(crate) fn frames() -> Option<Box<dyn FnMut(Tick) -> TickInput + Send>> {
    let value = std::env::var_os(VARIABLE).filter(|value| !value.is_empty())?;
    Some(match value.to_str() {
        Some("stderr-held") => Box::new(|_| stderr_held()),
        Some("stderr-blocked") => Box::new(|_| stderr_blocked()),
        _ => Box::new(|_| panic!("{VARIABLE} is set: the first frame panics")),
    })
}

/// A panic while another thread holds stderr's lock, inside `eprintln!`, and panics there a
/// second later: a hook that took the lock after its own would wait for that thread, and
/// that thread's hook for it.
fn stderr_held() -> ! {
    let (ready, started) = mpsc::channel();
    let holder = std::thread::Builder::new()
        .name("stderr-holder".to_owned())
        .spawn(move || eprintln!("{}", PanicsWithStderrHeld(ready)));
    drop(holder);
    let _ = started.recv();
    panic!("{VARIABLE}=stderr-held: the first frame panics while another thread holds stderr");
}

/// Sends once its `Display` runs, inside `eprintln!`, then waits a second and panics.
struct PanicsWithStderrHeld(mpsc::Sender<()>);

impl fmt::Display for PanicsWithStderrHeld {
    fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = self.0.send(());
        std::thread::sleep(Duration::from_secs(1));
        panic!("the second panic, inside eprintln!");
    }
}

/// Held by the panicking frame, and waited on by a thread inside `eprintln!`.
static HELD: Mutex<()> = Mutex::new(());

/// A panic while holding a lock that another thread waits on inside `eprintln!`, so that
/// thread holds stderr's lock until the abort: a hook that takes that lock never writes.
fn stderr_blocked() -> ! {
    let _held = HELD.lock();
    let (ready, started) = mpsc::channel();
    let waiter = std::thread::Builder::new()
        .name("stderr-waiter".to_owned())
        .spawn(move || eprintln!("{}", WaitsForHeld(ready)));
    drop(waiter);
    let _ = started.recv();
    // The waiter sends before it waits on `HELD`: the moment to get there.
    std::thread::sleep(Duration::from_millis(100));
    panic!("{VARIABLE}=stderr-blocked: the first frame panics holding what stderr waits on");
}

/// Sends once its `Display` runs, inside `eprintln!`, then waits for [`HELD`].
struct WaitsForHeld(mpsc::Sender<()>);

impl fmt::Display for WaitsForHeld {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = self.0.send(());
        let _held = HELD.lock();
        f.write_str("HELD was released")
    }
}
