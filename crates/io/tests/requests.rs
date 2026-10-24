//! The requests through the public surface (`README.md`, "Proof for milestone 1"): every
//! row of the error table a test can produce here, short reads, the decode, the atomic
//! replace as a caller sees it, directories, and what a stopped pool answers. The proofs
//! that hold a pool thread at a chosen point are the crate's unit tests in `src/tests.rs`.
//!
//! The counts of `AllocTag::Io` and the largest block asked of the allocator assume one
//! process per test, as `cargo nextest` runs them.
//!
//! The one `unsafe` in the crate is here: the tests' allocator records the largest block
//! asked of it, to prove a request above the cap allocates nothing near it (`SAFETY.md`).

#![deny(unsafe_code)]

mod common;

use std::sync::{Arc, Mutex};
use std::task::{Poll, Wake, Waker};
use std::thread::ThreadId;
use std::time::Instant;

use lactose_core::{AllocTag, ErrorKind, TaggedAlloc};
use lactose_io::{Encoding, EntryKind, MAX_READ_BYTES};
use zstd::zstd_safe::CParameter;

use common::{LIMIT, Scratch, frame, poll_with, start, text, wait};

#[global_allocator]
static ALLOCATOR: TaggedAlloc<largest::Largest> = TaggedAlloc::new(largest::Largest);

/// The system allocator, recording the largest block asked of it.
#[allow(unsafe_code, reason = "a `GlobalAlloc` is an `unsafe` trait; see `SAFETY.md`")]
mod largest {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static LARGEST: AtomicUsize = AtomicUsize::new(0);

    /// The size of the largest block asked for since the last call, on any thread (the
    /// tagged allocator above asks for one byte more than its caller).
    pub(crate) fn take() -> usize {
        LARGEST.swap(0, Ordering::SeqCst)
    }

    pub(crate) struct Largest;

    // SAFETY: every method forwards its arguments unchanged to `System` and returns what
    // `System` returns, so `System`'s guarantees are this allocator's; recording a size
    // touches no block.
    unsafe impl GlobalAlloc for Largest {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            LARGEST.fetch_max(layout.size(), Ordering::Relaxed);
            // SAFETY: the caller upholds `alloc`'s contract, which is `System`'s.
            unsafe { System.alloc(layout) }
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            LARGEST.fetch_max(layout.size(), Ordering::Relaxed);
            // SAFETY: the caller upholds `alloc_zeroed`'s contract, which is `System`'s.
            unsafe { System.alloc_zeroed(layout) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            // SAFETY: `ptr` was allocated by `System` with `layout`, through this allocator.
            unsafe { System.dealloc(ptr, layout) }
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            LARGEST.fetch_max(new_size, Ordering::Relaxed);
            // SAFETY: `ptr` was allocated by `System` with `layout`, through this allocator,
            // and the caller upholds the rest of `realloc`'s contract, which is `System`'s.
            unsafe { System.realloc(ptr, layout, new_size) }
        }
    }
}

fn zstd(decoded_len: usize) -> Encoding {
    Encoding::Zstd { decoded_len: decoded_len as u64 }
}

// ---- Errors ----

#[test]
fn a_missing_file_or_directory_is_not_found() {
    let scratch = Scratch::new();
    let io = start(1);
    assert_eq!(wait(io.open(scratch.path("missing"))).unwrap_err().kind(), ErrorKind::NotFound);
    let error = wait(io.read_file(scratch.path("missing"))).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert_eq!(wait(io.read_dir(scratch.path("missing"))).unwrap_err().kind(), ErrorKind::NotFound);
    // A replace into a directory that does not exist, before anything is created.
    let bytes = b"save".to_vec();
    let pointer = bytes.as_ptr();
    let failed = wait(io.replace(scratch.path("missing").join("slot"), bytes)).unwrap_err();
    assert_eq!(failed.error.kind(), ErrorKind::NotFound);
    assert_eq!((failed.bytes.as_ptr(), failed.bytes.as_slice()), (pointer, &b"save"[..]));
    assert_eq!(scratch.names(), Vec::<String>::new());
    io.shutdown();
}

#[test]
fn a_range_past_the_end_or_overflowing_is_invalid_data_and_allocates_nothing() {
    let scratch = Scratch::new();
    std::fs::write(scratch.path("pack"), text(4096, 1)).unwrap();
    let io = start(1);
    let file = wait(io.open(scratch.path("pack"))).unwrap();
    let live = ALLOCATOR.stats(AllocTag::Io).live_bytes;
    assert_eq!(wait(file.read_at(4000, 97)).unwrap_err().kind(), ErrorKind::InvalidData);
    assert_eq!(wait(file.read_at(5000, 1)).unwrap_err().kind(), ErrorKind::InvalidData);
    assert_eq!(ALLOCATOR.stats(AllocTag::Io).live_bytes, live, "a partial buffer is held");
    let overflow = wait(file.read_at(u64::MAX, 2)).unwrap_err();
    assert_eq!(overflow.kind(), ErrorKind::InvalidData);
    let overflow = wait(file.read_encoded_at(u64::MAX - 1, 2, zstd(16))).unwrap_err();
    assert_eq!(overflow.kind(), ErrorKind::InvalidData);
    let empty = wait(file.read_encoded_at(0, 0, zstd(16))).unwrap_err();
    assert_eq!(empty.kind(), ErrorKind::InvalidData);
    // A length of zero touches nothing, whatever the offset.
    assert_eq!(wait(file.read_at(u64::MAX, 0)).unwrap(), Vec::<u8>::new());
    io.shutdown();
}

#[test]
fn a_length_above_the_cap_is_resource_exhausted_before_any_allocation() {
    let scratch = Scratch::new();
    let path = scratch.path("pack");
    std::fs::write(&path, text(64, 2)).unwrap();
    let io = start(1);
    let file = wait(io.open(&path)).unwrap();
    let total = ALLOCATOR.stats(AllocTag::Io).total_allocations;
    let above = MAX_READ_BYTES + 1;
    assert_eq!(wait(file.read_at(0, above)).unwrap_err().kind(), ErrorKind::ResourceExhausted);
    let error = wait(file.read_encoded_at(0, above, zstd(16))).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::ResourceExhausted);
    let error =
        wait(file.read_encoded_at(0, 16, Encoding::Zstd { decoded_len: above })).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::ResourceExhausted);
    // The cap is checked before the overflow.
    let error = wait(file.read_at(u64::MAX, above)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::ResourceExhausted);
    assert_eq!(ALLOCATOR.stats(AllocTag::Io).total_allocations, total);
    // A whole file above the cap, sparse so the test writes nothing. Its size is learnt on
    // the pool thread, which allocates the error there; a buffer for it would have read
    // the file and resolved `Ok`.
    let large = scratch.path("large");
    std::fs::File::create(&large).unwrap().set_len(above).unwrap();
    let mut read = io.read_file(&large);
    largest::take();
    assert_eq!(wait(&mut read).unwrap_err().kind(), ErrorKind::ResourceExhausted);
    let largest = largest::take();
    assert!(largest < 1 << 20, "a block of {largest} bytes was asked for");
    io.shutdown();
}

#[test]
fn the_tests_allocator_records_the_largest_block_asked_of_it() {
    largest::take();
    let mut bytes = vec![0u8; 3000];
    bytes.reserve_exact(5000);
    bytes.shrink_to(10);
    drop(bytes);
    drop(Vec::<u8>::with_capacity(100));
    // The tagged allocator asks for one byte more; only the reallocation asks for 8000.
    assert!(largest::take() >= 8001);
    assert!(largest::take() < 8001);
}

#[test]
fn a_directory_opened_or_read_as_a_file_is_io() {
    let scratch = Scratch::new();
    std::fs::create_dir(scratch.path("dir")).unwrap();
    std::fs::write(scratch.path("file"), b"file").unwrap();
    let io = start(1);
    assert_eq!(wait(io.open(scratch.path("dir"))).unwrap_err().kind(), ErrorKind::Io);
    assert_eq!(wait(io.read_file(scratch.path("dir"))).unwrap_err().kind(), ErrorKind::Io);
    assert_eq!(wait(io.read_dir(scratch.path("file"))).unwrap_err().kind(), ErrorKind::Io);
    io.shutdown();
}

/// Skipped when the process can write into a directory it made read-only, as root can.
#[cfg(unix)]
#[test]
fn a_permission_failure_is_io() {
    use std::os::unix::fs::PermissionsExt;

    let scratch = Scratch::new();
    let locked = scratch.path("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::write(locked.join("slot"), b"old").unwrap();
    std::fs::write(scratch.path("secret"), b"secret").unwrap();
    std::fs::set_permissions(scratch.path("secret"), std::fs::Permissions::from_mode(0o000))
        .unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
    let restore = || {
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    if std::fs::write(locked.join("probe"), b"").is_ok() {
        restore();
        eprintln!("skipped: this process writes into a read-only directory");
        return;
    }
    let io = start(1);
    assert_eq!(wait(io.open(scratch.path("secret"))).unwrap_err().kind(), ErrorKind::Io);
    assert_eq!(wait(io.read_file(scratch.path("secret"))).unwrap_err().kind(), ErrorKind::Io);
    let bytes = b"new".to_vec();
    let pointer = bytes.as_ptr();
    let failed = wait(io.replace(locked.join("slot"), bytes)).unwrap_err();
    assert_eq!(failed.error.kind(), ErrorKind::Io, "{}", failed.error);
    assert_eq!(failed.bytes.as_ptr(), pointer);
    let error = wait(io.create_dir_all(locked.join("sub"))).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Io);
    io.shutdown();
    restore();
    assert_eq!(std::fs::read(locked.join("slot")).unwrap(), b"old");
    let names: Vec<_> =
        std::fs::read_dir(&locked).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(names, ["slot"]);
}

#[test]
fn a_request_after_shutdown_is_cancelled() {
    let scratch = Scratch::new();
    std::fs::write(scratch.path("file"), b"file").unwrap();
    let io = start(2);
    let clone = io.clone();
    let file = wait(io.open(scratch.path("file"))).unwrap();
    io.shutdown();
    assert_eq!(
        wait(clone.read_file(scratch.path("file"))).unwrap_err().kind(),
        ErrorKind::Cancelled
    );
    assert_eq!(wait(clone.open(scratch.path("file"))).unwrap_err().kind(), ErrorKind::Cancelled);
    let error = wait(clone.create_dir_all(scratch.path("dir"))).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Cancelled);
    assert_eq!(wait(clone.read_dir(&scratch.root)).unwrap_err().kind(), ErrorKind::Cancelled);
    assert_eq!(wait(file.read_at(0, 4)).unwrap_err().kind(), ErrorKind::Cancelled);
    let bytes = b"new".to_vec();
    let pointer = bytes.as_ptr();
    let failed = wait(clone.replace(scratch.path("file"), bytes)).unwrap_err();
    assert_eq!(failed.error.kind(), ErrorKind::Cancelled);
    assert_eq!(failed.bytes.as_ptr(), pointer);
    assert_eq!(std::fs::read(scratch.path("file")).unwrap(), b"file");
    assert!(!scratch.path("dir").exists());
}

#[test]
fn a_read_on_a_file_after_the_last_io_is_dropped_is_cancelled() {
    let scratch = Scratch::new();
    let pack = frame(&text(4096, 3));
    std::fs::write(scratch.path("pack"), &pack).unwrap();
    let io = start(1);
    let file = wait(io.open(scratch.path("pack"))).unwrap();
    drop(io);
    assert_eq!(wait(file.read_at(0, 4)).unwrap_err().kind(), ErrorKind::Cancelled);
    let encoded = file.read_encoded_at(0, pack.len() as u64, zstd(4096));
    assert_eq!(wait(encoded).unwrap_err().kind(), ErrorKind::Cancelled);
    assert_eq!(file.size(), pack.len() as u64);
}

// ---- Short reads ----

#[test]
fn a_file_one_byte_short_is_invalid_data_and_one_exactly_long_enough_is_read() {
    let scratch = Scratch::new();
    let content = text(10_000, 4);
    std::fs::write(scratch.path("pack"), &content).unwrap();
    let io = start(1);
    let file = wait(io.open(scratch.path("pack"))).unwrap();
    assert_eq!(file.size(), 10_000);
    let short = wait(file.read_at(1000, 9001)).unwrap_err();
    assert_eq!(short.kind(), ErrorKind::InvalidData);
    let bytes = wait(file.read_at(1000, 9000)).unwrap();
    assert_eq!((bytes.len(), bytes.capacity()), (9000, 9000));
    assert_eq!(bytes, content[1000..]);
    let whole = wait(io.read_file(scratch.path("pack"))).unwrap();
    assert_eq!(whole, content);
    io.shutdown();
}

#[test]
fn reads_on_one_file_in_flight_at_once_each_get_their_own_range() {
    let scratch = Scratch::new();
    let content = text(1 << 20, 5);
    std::fs::write(scratch.path("pack"), &content).unwrap();
    let io = start(4);
    let file = wait(io.open(scratch.path("pack"))).unwrap();
    let readers: Vec<_> = (0..16u64)
        .map(|i| {
            let read = file.read_at(i * 65536, 65536);
            std::thread::spawn(move || wait(read).unwrap())
        })
        .collect();
    for (i, reader) in readers.into_iter().enumerate() {
        assert_eq!(reader.join().unwrap(), content[i * 65536..(i + 1) * 65536]);
    }
    io.shutdown();
}

// ---- Decode ----

#[test]
fn a_frame_round_trips_at_its_offset_into_a_buffer_of_exactly_decoded_len() {
    let scratch = Scratch::new();
    let content = text(300_000, 6);
    let stored = frame(&content);
    let mut pack = text(777, 7);
    let offset = pack.len() as u64;
    pack.extend_from_slice(&stored);
    pack.extend_from_slice(&text(333, 8));
    std::fs::write(scratch.path("pack"), &pack).unwrap();
    let io = start(1);
    let file = wait(io.open(scratch.path("pack"))).unwrap();
    let bytes =
        wait(file.read_encoded_at(offset, stored.len() as u64, zstd(content.len()))).unwrap();
    assert_eq!((bytes.len(), bytes.capacity()), (content.len(), content.len()));
    assert_eq!(bytes, content);
    io.shutdown();
}

#[test]
fn every_stored_range_but_one_whole_frame_of_the_stated_size_is_invalid_data() {
    let scratch = Scratch::new();
    let content = text(100_000, 9);
    let whole = frame(&content);
    let half = content.len() / 2;
    let no_size = {
        let mut compressor = zstd::bulk::Compressor::new(3).unwrap();
        compressor.set_parameter(CParameter::ContentSizeFlag(false)).unwrap();
        compressor.compress(&content).unwrap()
    };
    let mut skippable = vec![0x50, 0x2A, 0x4D, 0x18, 4, 0, 0, 0, 1, 2, 3, 4];
    skippable.extend_from_slice(&whole);
    let mut trailing = whole.clone();
    trailing.extend_from_slice(b"xyz");
    let mut two = frame(&content[..half]);
    two.extend_from_slice(&frame(&content[half..]));
    let cases: [(&str, &[u8], usize); 8] = [
        ("truncated", &whole[..whole.len() - 3], content.len()),
        ("trailing bytes", &trailing, content.len()),
        ("two frames", &two, content.len()),
        ("a skippable frame first", &skippable, content.len()),
        ("no content size", &no_size, content.len()),
        ("content size above decoded_len", &whole, content.len() - 1),
        ("content size below decoded_len", &whole, content.len() + 1),
        ("not zstd", &content[..1000], 1000),
    ];
    let mut pack = Vec::new();
    let mut ranges = Vec::new();
    for (_, stored, decoded_len) in &cases {
        ranges.push((pack.len() as u64, stored.len() as u64, *decoded_len));
        pack.extend_from_slice(stored);
    }
    let control = pack.len() as u64;
    pack.extend_from_slice(&whole);
    std::fs::write(scratch.path("pack"), &pack).unwrap();
    let io = start(2);
    let file = wait(io.open(scratch.path("pack"))).unwrap();
    for ((case, ..), (offset, len, decoded_len)) in cases.iter().zip(ranges) {
        let error = wait(file.read_encoded_at(offset, len, zstd(decoded_len))).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData, "{case}: {error}");
    }
    let bytes = wait(file.read_encoded_at(control, whole.len() as u64, zstd(content.len())));
    assert_eq!(bytes.unwrap(), content);
    io.shutdown();
}

/// A waker that records the thread of each wake and unparks the test's thread.
struct Recording {
    wakes: Mutex<Vec<ThreadId>>,
    test: std::thread::Thread,
}

impl Wake for Recording {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.wakes.lock().unwrap().push(std::thread::current().id());
        self.test.unpark();
    }
}

#[test]
fn a_frame_of_several_mib_decodes_in_many_polls_on_the_polling_thread() {
    let scratch = Scratch::new();
    let content = text(16 << 20, 10);
    let stored = frame(&content);
    std::fs::write(scratch.path("pack"), &stored).unwrap();
    let io = start(1);
    let file = wait(io.open(scratch.path("pack"))).unwrap();
    let mut read = file.read_encoded_at(0, stored.len() as u64, zstd(content.len()));
    let recording =
        Arc::new(Recording { wakes: Mutex::new(Vec::new()), test: std::thread::current() });
    let waker = Waker::from(Arc::clone(&recording));
    let deadline = Instant::now() + LIMIT;
    let mut polls = 0;
    let bytes = loop {
        polls += 1;
        if let Poll::Ready(result) = poll_with(&mut read, &waker) {
            break result.unwrap();
        }
        let now = Instant::now();
        assert!(now < deadline, "the read did not resolve");
        std::thread::park_timeout(deadline - now);
    };
    assert_eq!(bytes, content);
    let me = std::thread::current().id();
    let wakes = recording.wakes.lock().unwrap();
    let own = wakes.iter().filter(|&&thread| thread == me).count();
    let pool = wakes.len() - own;
    assert!(own > 1, "the decode woke itself {own} times");
    assert!(pool <= 1, "the pool woke the read {pool} times");
    assert!(polls > own, "{polls} polls for {own} wakes of its own");
    io.shutdown();
}

#[test]
fn a_window_above_two_to_the_27_is_invalid_data_and_one_at_it_decodes() {
    let scratch = Scratch::new();
    let content = vec![0u8; (1 << 27) + 4096];
    let compress = |window_log| {
        let mut compressor = zstd::bulk::Compressor::new(1).unwrap();
        compressor.set_parameter(CParameter::WindowLog(window_log)).unwrap();
        compressor.compress(&content).unwrap()
    };
    let wide = compress(28);
    let narrow = compress(27);
    let mut pack = wide.clone();
    pack.extend_from_slice(&narrow);
    std::fs::write(scratch.path("pack"), &pack).unwrap();
    let io = start(1);
    let file = wait(io.open(scratch.path("pack"))).unwrap();
    let error = wait(file.read_encoded_at(0, wide.len() as u64, zstd(content.len()))).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
    let read = file.read_encoded_at(wide.len() as u64, narrow.len() as u64, zstd(content.len()));
    let bytes = wait(read).unwrap();
    assert_eq!(bytes.len(), content.len());
    assert!(bytes.iter().all(|&byte| byte == 0));
    io.shutdown();
}

#[test]
fn an_encoded_read_whose_stored_bytes_arrived_before_the_stop_still_decodes() {
    let scratch = Scratch::new();
    let content = text(4 << 20, 11);
    let stored = frame(&content);
    std::fs::write(scratch.path("pack"), &stored).unwrap();
    let io = start(1);
    let file = wait(io.open(scratch.path("pack"))).unwrap();
    let mut read = file.read_encoded_at(0, stored.len() as u64, zstd(content.len()));
    let recording =
        Arc::new(Recording { wakes: Mutex::new(Vec::new()), test: std::thread::current() });
    let waker = Waker::from(Arc::clone(&recording));
    // Woken once: by the pool with the stored bytes, or by the first slice of the decode.
    assert!(poll_with(&mut read, &waker).is_pending());
    let deadline = Instant::now() + LIMIT;
    while recording.wakes.lock().unwrap().is_empty() {
        let now = Instant::now();
        assert!(now < deadline, "the stored bytes never arrived");
        std::thread::park_timeout(deadline - now);
    }
    io.shutdown();
    assert_eq!(wait(read).unwrap(), content);
}

// ---- The atomic replace ----

#[test]
fn a_replace_is_seen_whole_after_ok_and_by_the_next_read_file() {
    let scratch = Scratch::new();
    let slot = scratch.path("slot");
    let io = start(2);
    for (round, len) in [(0u64, 3usize), (1, 1 << 20), (2, 0), (3, 70_000)] {
        let bytes = text(len, round + 20);
        wait(io.replace(&slot, bytes.clone())).unwrap();
        assert_eq!(std::fs::read(&slot).unwrap(), bytes);
        assert_eq!(wait(io.read_file(&slot)).unwrap(), bytes);
    }
    io.shutdown();
    assert_eq!(scratch.names(), ["slot"]);
}

#[test]
fn a_replace_of_a_path_that_is_a_directory_fails_with_its_bytes_and_leaves_nothing() {
    let scratch = Scratch::new();
    let slot = scratch.path("slot");
    std::fs::create_dir(&slot).unwrap();
    std::fs::write(slot.join("inside"), b"inside").unwrap();
    let io = start(1);
    let bytes = text(100_000, 21);
    let (pointer, len) = (bytes.as_ptr(), bytes.len());
    let failed = wait(io.replace(&slot, bytes)).unwrap_err();
    assert_eq!(failed.error.kind(), ErrorKind::Io, "{}", failed.error);
    assert_eq!((failed.bytes.as_ptr(), failed.bytes.len()), (pointer, len));
    assert_eq!(failed.bytes, text(100_000, 21));
    io.shutdown();
    assert_eq!(scratch.names(), ["slot"], "a temporary was left");
    assert_eq!(std::fs::read(slot.join("inside")).unwrap(), b"inside");
}

#[test]
fn a_file_opened_before_a_replace_reads_the_previous_content_after_it() {
    let scratch = Scratch::new();
    let slot = scratch.path("slot");
    let old = text(50_000, 22);
    std::fs::write(&slot, &old).unwrap();
    let io = start(1);
    let file = wait(io.open(&slot)).unwrap();
    // A handle of our own, held open over the replace, does not stop it.
    let held = std::fs::File::open(&slot).unwrap();
    let new = text(60_000, 23);
    wait(io.replace(&slot, new.clone())).unwrap();
    assert_eq!(wait(file.read_at(0, old.len() as u64)).unwrap(), old);
    assert_eq!(wait(io.read_file(&slot)).unwrap(), new);
    drop(held);
    io.shutdown();
}

#[test]
fn a_replace_of_a_bare_name_lands_in_the_current_directory() {
    let scratch = Scratch::new();
    // The current directory is this test's own process's under `cargo nextest`.
    std::env::set_current_dir(&scratch.root).unwrap();
    let io = start(1);
    wait(io.replace("slot0.carton", b"save".to_vec())).unwrap();
    wait(io.replace("slot0.carton", b"newer".to_vec())).unwrap();
    io.shutdown();
    assert_eq!(std::fs::read(scratch.path("slot0.carton")).unwrap(), b"newer");
    assert_eq!(scratch.names(), ["slot0.carton"]);
}

/// A replace whose last component is a symbolic link renames over the link: the path
/// becomes a file holding the bytes, and the link's target keeps its content.
#[cfg(unix)]
#[test]
fn a_replace_of_a_symbolic_link_replaces_the_link_and_leaves_its_target() {
    let scratch = Scratch::new();
    std::fs::write(scratch.path("target"), b"old").unwrap();
    std::os::unix::fs::symlink(scratch.path("target"), scratch.path("link")).unwrap();
    let io = start(1);
    wait(io.replace(scratch.path("link"), b"new".to_vec())).unwrap();
    io.shutdown();
    assert!(std::fs::symlink_metadata(scratch.path("link")).unwrap().file_type().is_file());
    assert_eq!(std::fs::read(scratch.path("link")).unwrap(), b"new");
    assert_eq!(std::fs::read(scratch.path("target")).unwrap(), b"old");
    assert_eq!(scratch.names(), ["link", "target"]);
}

// ---- Directories ----

#[test]
fn a_listing_is_sorted_by_name_with_each_entry_s_own_kind() {
    let scratch = Scratch::new();
    for name in ["b", "a.1a2b3c4d00000000.tmp", "Z", "a"] {
        std::fs::write(scratch.path(name), name).unwrap();
    }
    std::fs::create_dir(scratch.path("dir")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(scratch.path("dir"), scratch.path("link")).unwrap();
    let io = start(1);
    let listing = wait(io.read_dir(&scratch.root)).unwrap();
    io.shutdown();
    let seen: Vec<(String, EntryKind)> =
        listing.into_iter().map(|entry| (entry.name.into_string().unwrap(), entry.kind)).collect();
    let mut expected = vec![
        ("Z".to_owned(), EntryKind::File),
        ("a".to_owned(), EntryKind::File),
        ("a.1a2b3c4d00000000.tmp".to_owned(), EntryKind::File),
        ("b".to_owned(), EntryKind::File),
        ("dir".to_owned(), EntryKind::Directory),
    ];
    if cfg!(unix) {
        expected.push(("link".to_owned(), EntryKind::Other));
    }
    assert_eq!(seen, expected);
}

#[test]
fn create_dir_all_creates_the_chain_and_succeeds_again() {
    let scratch = Scratch::new();
    let io = start(1);
    let deep = scratch.path("x").join("y").join("z");
    wait(io.create_dir_all(&deep)).unwrap();
    assert!(deep.is_dir());
    wait(io.create_dir_all(&deep)).unwrap();
    wait(io.create_dir_all(scratch.path("x"))).unwrap();
    assert_eq!(wait(io.read_dir(scratch.path("x"))).unwrap().len(), 1);
    std::fs::write(scratch.path("file"), b"in the way").unwrap();
    let error = wait(io.create_dir_all(scratch.path("file").join("sub"))).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Io);
    let error = wait(io.create_dir_all(scratch.path("file"))).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Io);
    io.shutdown();
}
