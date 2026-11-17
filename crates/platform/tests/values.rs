//! What runs in the facade: the plain values, which no body of the implementation decides.

use std::sync::Arc;

use lactose_platform::{ClientPlatform, LOG_TARGET, PixelSize, WindowDesc};
use lactose_task::Platform;

#[test]
fn a_size_is_empty_when_either_dimension_is_zero() {
    assert!(PixelSize { width: 0, height: 0 }.is_empty());
    assert!(PixelSize { width: 0, height: 720 }.is_empty());
    assert!(PixelSize { width: 1280, height: 0 }.is_empty());
    assert!(!PixelSize { width: 1, height: 1 }.is_empty());
    assert!(!PixelSize { width: u32::MAX, height: u32::MAX }.is_empty());
}

#[test]
fn a_size_is_decided_at_compile_time() {
    const MINIMISED: PixelSize = PixelSize { width: 0, height: 0 };
    const SHOWN: PixelSize = PixelSize { width: 1280, height: 720 };
    const { assert!(MINIMISED.is_empty() && !SHOWN.is_empty()) }
}

#[test]
fn a_description_is_plain_data() {
    let desc =
        WindowDesc { title: "client".to_owned(), size: PixelSize { width: 1280, height: 720 } };
    assert_eq!(desc.clone(), desc);
    assert_eq!(desc.size, PixelSize { width: 1280, height: 720 });
}

#[test]
fn the_client_platform_is_made_anywhere_and_held_as_the_executor_holds_one() {
    const PLATFORM: ClientPlatform = ClientPlatform::new();
    let on_another_thread = std::thread::spawn(|| {
        let platform: Arc<dyn Platform> = Arc::new(ClientPlatform::new());
        platform
    });
    let _platform: Arc<dyn Platform> = on_another_thread.join().expect("nothing panics");
    let _default: Arc<dyn Platform> = Arc::new(ClientPlatform::default());
    let _const: Arc<dyn Platform> = Arc::new(PLATFORM);
}

#[test]
fn the_log_target_is_the_one_the_readme_names() {
    assert_eq!(LOG_TARGET, "pail");
}
