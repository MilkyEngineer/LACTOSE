//! `StorageFull` from a replace on a size-limited file system (`README.md`, "Proof for
//! milestone 1", the errors row). It runs where the environment names a directory on one
//! with less than 64 MiB free, in `LACTOSE_IO_SMALL_FS`: a `tmpfs` mounted with `size=` on
//! Linux, a small disk image on macOS. Elsewhere it passes without running and says so, and
//! the mapping of `ENOSPC` is `lactose-core`'s test.

#![forbid(unsafe_code)]

mod common;

use lactose_core::ErrorKind;

use common::{start, text, wait};

/// More than the file system the variable names has free.
const TOO_LARGE: usize = 64 << 20;

#[test]
fn a_replace_that_fills_the_disk_is_storage_full_and_keeps_the_previous_content() {
    let Some(root) = std::env::var_os("LACTOSE_IO_SMALL_FS") else {
        eprintln!("skipped: LACTOSE_IO_SMALL_FS names no size-limited directory");
        return;
    };
    let dir = std::path::Path::new(&root).join(format!("lactose-io-full-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let slot = dir.join("slot");
    std::fs::write(&slot, b"old").unwrap();
    let io = start(1);
    let bytes = text(TOO_LARGE, 1);
    let pointer = bytes.as_ptr();
    let failed = wait(io.replace(&slot, bytes)).unwrap_err();
    io.shutdown();
    let names: Vec<_> =
        std::fs::read_dir(&dir).unwrap().map(|entry| entry.unwrap().file_name()).collect();
    let previous = std::fs::read(&slot).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(failed.error.kind(), ErrorKind::StorageFull, "{}", failed.error);
    assert_eq!((failed.bytes.as_ptr(), failed.bytes.len()), (pointer, TOO_LARGE));
    assert_eq!(previous, b"old");
    assert_eq!(names, ["slot"], "the temporary was left");
}
