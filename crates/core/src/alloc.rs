//! The tagged global allocator: live memory accounted per subsystem.
//!
//! A binary installs [`TaggedAlloc`] as its `#[global_allocator]`. Code attributes its
//! allocations by holding a [`TagScope`]; the executor carries the tag of a job or task across
//! threads. Each allocation remembers its tag in the byte before the returned pointer, so a
//! free on any thread, under any scope, is charged back to the right subsystem.
//!
//! The current tag is the one thread-local in the boundary. It is written by scopes and read
//! by the allocator only, and no simulation result can depend on it.

use core::alloc::{GlobalAlloc, Layout};
use core::cell::Cell;
use core::marker::PhantomData;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::alloc::System;

/// The subsystem an allocation is charged to.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[repr(u8)]
pub enum AllocTag {
    /// No scope was active.
    #[default]
    Untagged = 0,
    /// Executor queues, job storage and task state.
    Task,
    /// Chunks, archetype tables and command buffers.
    Ecs,
    /// Regions, tick loop and snapshots.
    Sim,
    /// The physics backend.
    Physics,
    /// Animation state and the simulation skeleton.
    Anim,
    /// Transport, replication and interpolation buffers.
    Net,
    /// Loaded assets and their dependency tables.
    Asset,
    /// IO buffers and in-flight requests.
    Io,
    /// Save overlays and containers.
    Persist,
    /// The renderer, render graph and RHI backends.
    Render,
    /// The per-frame presentation schedule.
    Presentation,
    /// The mixer and its event bridge.
    Audio,
    /// Game UI and text.
    Ui,
    /// Gameplay crates.
    Game,
    /// Cooker, debug overlay and other tooling.
    Tools,
}

impl AllocTag {
    /// The number of tags.
    pub const COUNT: usize = Self::ALL.len();

    /// Every tag, in discriminant order.
    pub const ALL: [Self; 16] = [
        Self::Untagged,
        Self::Task,
        Self::Ecs,
        Self::Sim,
        Self::Physics,
        Self::Anim,
        Self::Net,
        Self::Asset,
        Self::Io,
        Self::Persist,
        Self::Render,
        Self::Presentation,
        Self::Audio,
        Self::Ui,
        Self::Game,
        Self::Tools,
    ];

    /// Charges allocations on this thread to `self` until the returned scope is dropped.
    #[must_use = "the tag applies only while the scope is alive"]
    pub fn scope(self) -> TagScope {
        TagScope { previous: with_current(|current| current.replace(self)), not_send: PhantomData }
    }

    /// The tag allocations on this thread are currently charged to.
    #[must_use]
    pub fn current() -> Self {
        with_current(Cell::get)
    }

    fn from_byte(byte: u8) -> Self {
        Self::ALL.get(byte as usize).copied().unwrap_or(Self::Untagged)
    }
}

/// Runs `f` on the calling thread's current tag.
#[expect(
    clippy::disallowed_types,
    reason = "the allocation tag is the one sanctioned thread-local"
)]
fn with_current<R>(f: impl FnOnce(&Cell<AllocTag>) -> R) -> R {
    thread_local! {
        // Const-initialised and without a destructor, so reading it inside the allocator
        // neither allocates nor fails during thread teardown.
        static CURRENT: Cell<AllocTag> = const { Cell::new(AllocTag::Untagged) };
    }
    CURRENT.with(f)
}

/// Restores the previous allocation tag when dropped. Scopes nest.
pub struct TagScope {
    previous: AllocTag,
    not_send: PhantomData<*const ()>,
}

impl Drop for TagScope {
    fn drop(&mut self) {
        with_current(|current| current.set(self.previous));
    }
}

/// A snapshot of one tag's counters.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct TagStats {
    /// Bytes currently allocated, as requested by callers (headers excluded).
    pub live_bytes: usize,
    /// Allocations currently alive.
    pub live_allocations: usize,
    /// Allocations ever made.
    pub total_allocations: u64,
}

// Aligned so two tags never share a cache line.
#[repr(align(128))]
struct Counters {
    live_bytes: AtomicUsize,
    live_allocations: AtomicUsize,
    total_allocations: AtomicU64,
}

impl Counters {
    const fn new() -> Self {
        Self {
            live_bytes: AtomicUsize::new(0),
            live_allocations: AtomicUsize::new(0),
            total_allocations: AtomicU64::new(0),
        }
    }
}

/// A [`GlobalAlloc`] wrapper that accounts live memory per [`AllocTag`].
///
/// Costs one alignment's worth of header per allocation and three relaxed atomic updates per
/// call. Hot paths allocate from arenas and chunks, not from here.
pub struct TaggedAlloc<A = System> {
    inner: A,
    counters: [Counters; AllocTag::COUNT],
}

impl TaggedAlloc<System> {
    /// A tagged allocator over the system allocator.
    #[must_use]
    pub const fn system() -> Self {
        Self::new(System)
    }
}

impl<A> TaggedAlloc<A> {
    /// A tagged allocator over `inner`.
    #[must_use]
    pub const fn new(inner: A) -> Self {
        Self { inner, counters: [const { Counters::new() }; AllocTag::COUNT] }
    }

    /// The counters for `tag`. Fields are read one at a time, so a snapshot taken while
    /// other threads allocate is approximate.
    #[must_use]
    pub fn stats(&self, tag: AllocTag) -> TagStats {
        let counters = &self.counters[tag as usize];
        TagStats {
            live_bytes: counters.live_bytes.load(Ordering::Relaxed),
            live_allocations: counters.live_allocations.load(Ordering::Relaxed),
            total_allocations: counters.total_allocations.load(Ordering::Relaxed),
        }
    }

    /// Writes the tag header and returns the caller's pointer.
    ///
    /// # Safety
    ///
    /// `base` must be null or point to an allocation of at least `align` bytes more than
    /// `size`, aligned to `align`.
    unsafe fn finish(&self, base: *mut u8, size: usize, align: usize) -> *mut u8 {
        if base.is_null() {
            return base;
        }
        let tag = AllocTag::current();
        let counters = &self.counters[tag as usize];
        counters.live_bytes.fetch_add(size, Ordering::Relaxed);
        counters.live_allocations.fetch_add(1, Ordering::Relaxed);
        counters.total_allocations.fetch_add(1, Ordering::Relaxed);
        // SAFETY: the allocation is `align` bytes longer than the caller asked for, so
        // `base + align` is in bounds and the byte before it is ours. `base + align` keeps
        // the alignment of `base`.
        unsafe {
            let user = base.add(align);
            user.sub(1).write(tag as u8);
            user
        }
    }
}

/// The layout actually requested from the inner allocator: `layout` plus a header of one
/// alignment. `None` if that overflows.
fn with_header(layout: Layout) -> Option<Layout> {
    let size = layout.size().checked_add(layout.align())?;
    Layout::from_size_align(size, layout.align()).ok()
}

// SAFETY: every method forwards to the inner allocator with a layout that differs from the
// caller's only by a header of `layout.align()` bytes placed before the returned pointer.
// The header keeps the caller's alignment, is never exposed, and is reconstructed from the
// same `layout` on free and realloc, so the inner allocator always sees the layout it issued.
unsafe impl<A: GlobalAlloc> GlobalAlloc for TaggedAlloc<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let Some(full) = with_header(layout) else { return core::ptr::null_mut() };
        // SAFETY: `full` has a non-zero size because the header is at least one byte.
        let base = unsafe { self.inner.alloc(full) };
        // SAFETY: `base` is null or an allocation of `full`.
        unsafe { self.finish(base, layout.size(), layout.align()) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let Some(full) = with_header(layout) else { return core::ptr::null_mut() };
        // SAFETY: `full` has a non-zero size because the header is at least one byte.
        let base = unsafe { self.inner.alloc_zeroed(full) };
        // SAFETY: `base` is null or an allocation of `full`.
        unsafe { self.finish(base, layout.size(), layout.align()) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let align = layout.align();
        // SAFETY: `ptr` came from this allocator with `layout`, so the tag byte precedes it,
        // the inner allocation starts `align` bytes earlier, and the sum below is the size
        // that `with_header` validated when the block was allocated.
        unsafe {
            let tag = AllocTag::from_byte(ptr.sub(1).read());
            let counters = &self.counters[tag as usize];
            counters.live_bytes.fetch_sub(layout.size(), Ordering::Relaxed);
            counters.live_allocations.fetch_sub(1, Ordering::Relaxed);
            let full = Layout::from_size_align_unchecked(layout.size() + align, align);
            self.inner.dealloc(ptr.sub(align), full);
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let align = layout.align();
        let Some(new_full_size) = new_size.checked_add(align) else {
            return core::ptr::null_mut();
        };
        // SAFETY: `ptr` came from this allocator with `layout`, so the inner allocation
        // starts `align` bytes earlier with the header-extended layout. The caller
        // guarantees `new_size` is a valid size for `align`; the header-extended size is
        // checked for overflow above and the inner allocator rejects the rest. The inner
        // realloc preserves the leading `align` bytes, which hold the tag.
        unsafe {
            let tag = AllocTag::from_byte(ptr.sub(1).read());
            let full = Layout::from_size_align_unchecked(layout.size() + align, align);
            let base = self.inner.realloc(ptr.sub(align), full, new_full_size);
            if base.is_null() {
                return base;
            }
            let live_bytes = &self.counters[tag as usize].live_bytes;
            if new_size >= layout.size() {
                live_bytes.fetch_add(new_size - layout.size(), Ordering::Relaxed);
            } else {
                live_bytes.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
            base.add(align)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(alloc: &TaggedAlloc, tag: AllocTag) -> (usize, usize, u64) {
        let stats = alloc.stats(tag);
        (stats.live_bytes, stats.live_allocations, stats.total_allocations)
    }

    #[test]
    fn scopes_nest_and_restore() {
        assert_eq!(AllocTag::current(), AllocTag::Untagged);
        {
            let _ecs = AllocTag::Ecs.scope();
            assert_eq!(AllocTag::current(), AllocTag::Ecs);
            {
                let _net = AllocTag::Net.scope();
                assert_eq!(AllocTag::current(), AllocTag::Net);
            }
            assert_eq!(AllocTag::current(), AllocTag::Ecs);
        }
        assert_eq!(AllocTag::current(), AllocTag::Untagged);
    }

    #[test]
    fn all_matches_discriminants() {
        for (index, tag) in AllocTag::ALL.into_iter().enumerate() {
            assert_eq!(tag as usize, index);
            assert_eq!(AllocTag::from_byte(index as u8), tag);
        }
    }

    #[test]
    fn frees_are_charged_to_the_allocating_tag() {
        let alloc = TaggedAlloc::system();
        let layout = Layout::from_size_align(100, 8).unwrap();
        let ptr = {
            let _scope = AllocTag::Render.scope();
            // SAFETY: `layout` has a non-zero size.
            unsafe { alloc.alloc(layout) }
        };
        assert!(!ptr.is_null());
        assert_eq!(ptr.addr() % 8, 0);
        assert_eq!(stats(&alloc, AllocTag::Render), (100, 1, 1));
        {
            let _scope = AllocTag::Audio.scope();
            // SAFETY: `ptr` was allocated above with `layout` and is freed once.
            unsafe { alloc.dealloc(ptr, layout) };
        }
        assert_eq!(stats(&alloc, AllocTag::Render), (0, 0, 1));
        assert_eq!(stats(&alloc, AllocTag::Audio), (0, 0, 0));
    }

    #[test]
    fn realloc_keeps_contents_tag_and_alignment() {
        let alloc = TaggedAlloc::system();
        for align in [1, 2, 8, 64, 4096] {
            let layout = Layout::from_size_align(64, align).unwrap();
            let _scope = AllocTag::Asset.scope();
            // SAFETY: the layouts have non-zero sizes; each pointer is used with the layout
            // it currently has and freed once.
            unsafe {
                let ptr = alloc.alloc_zeroed(layout);
                assert!((0..64).all(|i| ptr.add(i).read() == 0));
                for i in 0..64 {
                    ptr.add(i).write(i as u8);
                }
                let grown = {
                    let _other = AllocTag::Ui.scope();
                    alloc.realloc(ptr, layout, 10_000)
                };
                assert_eq!(grown.addr() % align, 0);
                assert!((0..64).all(|i| grown.add(i).read() == i as u8));
                assert_eq!(alloc.stats(AllocTag::Asset).live_bytes, 10_000);
                assert_eq!(alloc.stats(AllocTag::Ui).live_bytes, 0);

                let grown_layout = Layout::from_size_align(10_000, align).unwrap();
                let shrunk = alloc.realloc(grown, grown_layout, 16);
                assert!((0..16).all(|i| shrunk.add(i).read() == i as u8));
                assert_eq!(alloc.stats(AllocTag::Asset).live_bytes, 16);
                alloc.dealloc(shrunk, Layout::from_size_align(16, align).unwrap());
            }
            assert_eq!(alloc.stats(AllocTag::Asset).live_bytes, 0);
            assert_eq!(alloc.stats(AllocTag::Asset).live_allocations, 0);
        }
    }

    #[test]
    fn oversized_request_returns_null() {
        let alloc = TaggedAlloc::system();
        let layout = Layout::from_size_align(isize::MAX as usize - 7, 8).unwrap();
        // SAFETY: `layout` has a non-zero size.
        assert!(unsafe { alloc.alloc(layout) }.is_null());
        assert_eq!(stats(&alloc, AllocTag::Untagged), (0, 0, 0));
    }
}
