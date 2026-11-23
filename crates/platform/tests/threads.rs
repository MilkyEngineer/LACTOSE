//! `ClientPlatform` against `StdPlatform`, and `event_pump` off the main thread: what needs
//! no window. The window's own tests are `tests/window.rs`.

use std::sync::mpsc;

use lactose_core::thread_class::fp;
use lactose_core::{ErrorKind, ThreadClass};
use lactose_platform::{ClientPlatform, PixelSize, WindowDesc, event_pump};
use lactose_task::{Platform, StdPlatform, ThreadSpec};

#[test]
fn the_client_platform_reports_what_the_standard_one_does() {
    let platform = ClientPlatform::new();
    assert_eq!(platform.topology(), StdPlatform.topology());
    assert_eq!(platform.hardware_threads(), StdPlatform.hardware_threads());
}

/// Each thread is named as its spec says and runs its body once it has entered its class,
/// which sets the floating-point state on every operating system. The scheduling half is
/// read back in `tests/window.rs`, where the present thread's class is checked through the
/// operating system.
#[test]
fn the_client_platform_names_each_thread_and_enters_its_class() {
    let platform = ClientPlatform::new();
    for (class, name) in [
        (ThreadClass::Present, "present"),
        (ThreadClass::Background, "background"),
        (ThreadClass::FrameCritical, "frame"),
    ] {
        let (answer, answered) = mpsc::channel();
        let thread = platform
            .spawn_thread(
                ThreadSpec { name: name.to_owned(), class, index_in_class: 0 },
                Box::new(move || {
                    let named = std::thread::current().name().map(str::to_owned);
                    let _ = answer.send((named, cfg!(miri) || fp::is_deterministic()));
                }),
            )
            .expect("a thread");
        let (named, deterministic) = answered.recv().expect("the body ran");
        assert_eq!(named.as_deref(), Some(name));
        assert!(deterministic, "{class:?} did not set the floating-point state");
        thread.join();
    }
}

/// Row 208 where there is no window system, in CI on Linux too: winit's main-thread check
/// comes before its search for a display, so the call panics on every operating system.
/// The test's own thread is not taken to be off the main thread: the call is made on one
/// this test starts.
#[test]
#[cfg_attr(miri, ignore = "reaches the operating system's event loop")]
fn event_pump_panics_off_the_main_thread_and_a_second_is_unsupported() {
    let desc = WindowDesc { title: "off".to_owned(), size: PixelSize { width: 64, height: 64 } };
    let first = {
        let desc = desc.clone();
        std::thread::Builder::new()
            .name("off the main thread".to_owned())
            .spawn(move || drop(event_pump(desc)))
            .expect("a thread")
            .join()
    };
    let panic = first.expect_err("event_pump panics off the main thread");
    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap_or_default();
    assert!(message.contains("main thread"), "panicked with {message:?}");
    let second = event_pump(desc).err().expect("a second pump is refused");
    assert_eq!(second.kind(), ErrorKind::Unsupported);
}
