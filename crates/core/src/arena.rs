//! A bump arena for per-job scratch memory and per-scope job storage.

use core::alloc::Layout;
use core::cell::{Cell, RefCell};
use core::ptr::NonNull;

const CHUNK_ALIGN: usize = 16;
const DEFAULT_CHUNK_SIZE: usize = 64 * 1024;
const POISON: u8 = 0xCD;

struct Chunk {
    ptr: NonNull<u8>,
    layout: Layout,
}

/// A bump allocator that frees everything at once in [`Arena::reset`].
///
/// Values are never dropped, so the typed methods accept `Copy` types only. An arena that
/// outgrows its first chunk remembers the total and asks for one chunk of that size after the
/// next reset, so a steady workload settles at one chunk and no allocator traffic.
pub struct Arena {
    chunks: RefCell<Vec<Chunk>>,
    /// Bytes handed out from the last chunk.
    used: Cell<usize>,
    chunk_size: Cell<usize>,
}

// SAFETY: the arena owns its chunks outright and hands out no pointer that outlives a
// borrow of the arena, so moving it to another thread moves sole ownership of the memory.
unsafe impl Send for Arena {}

impl Default for Arena {
    fn default() -> Self {
        Self::new()
    }
}

impl Arena {
    /// An empty arena. Does not allocate until first use.
    #[must_use]
    pub fn new() -> Self {
        Self::with_chunk_size(DEFAULT_CHUNK_SIZE)
    }

    /// An empty arena whose chunks hold at least `chunk_size` bytes.
    #[must_use]
    pub fn with_chunk_size(chunk_size: usize) -> Self {
        Self {
            chunks: RefCell::new(Vec::new()),
            used: Cell::new(0),
            chunk_size: Cell::new(chunk_size.max(CHUNK_ALIGN)),
        }
    }

    /// Reserves memory for `layout` and returns a pointer to it.
    ///
    /// The memory is uninitialised, unaliased and valid until the next [`Arena::reset`] or
    /// until the arena is dropped. Calling this is safe; using the pointer is not.
    ///
    /// # Panics
    ///
    /// Panics if the request overflows `isize`. Aborts through the allocation error handler
    /// if the system allocator fails.
    pub fn alloc_layout(&self, layout: Layout) -> NonNull<u8> {
        if layout.size() == 0 {
            let dangling = core::ptr::without_provenance_mut::<u8>(layout.align());
            return NonNull::new(dangling).expect("alignment is non-zero");
        }

        let mut chunks = self.chunks.borrow_mut();
        if let Some(chunk) = chunks.last() {
            let base = chunk.ptr.addr().get();
            let start = (base + self.used.get()).next_multiple_of(layout.align()) - base;
            if let Some(end) = start.checked_add(layout.size())
                && end <= chunk.layout.size()
            {
                self.used.set(end);
                // SAFETY: `start < end <= chunk.layout.size()`, so the offset stays inside
                // the chunk's allocation.
                return unsafe { chunk.ptr.add(start) };
            }
        }

        let size = layout.size().max(self.chunk_size.get());
        let chunk_layout = Layout::from_size_align(size, layout.align().max(CHUNK_ALIGN))
            .expect("arena request overflows isize");
        // SAFETY: `chunk_layout` has a non-zero size because `layout.size()` is non-zero.
        let raw = unsafe { std::alloc::alloc(chunk_layout) };
        let Some(ptr) = NonNull::new(raw) else { std::alloc::handle_alloc_error(chunk_layout) };
        chunks.push(Chunk { ptr, layout: chunk_layout });
        self.used.set(layout.size());
        ptr
    }

    /// Moves `value` into the arena.
    #[expect(clippy::mut_from_ref, reason = "every call returns a distinct allocation")]
    pub fn alloc<T: Copy>(&self, value: T) -> &mut T {
        let ptr = self.alloc_layout(Layout::new::<T>()).cast::<T>();
        // SAFETY: `ptr` is aligned for `T`, valid for writes of one `T`, and unaliased. The
        // returned borrow is tied to `&self`, and `reset` and `drop` need `&mut self`.
        unsafe {
            ptr.write(value);
            &mut *ptr.as_ptr()
        }
    }

    /// Copies `values` into the arena.
    #[expect(clippy::mut_from_ref, reason = "every call returns a distinct allocation")]
    pub fn alloc_slice_copy<T: Copy>(&self, values: &[T]) -> &mut [T] {
        let ptr = self.alloc_layout(Layout::for_value(values)).cast::<T>();
        // SAFETY: `ptr` is aligned for `T`, valid for writes of `values.len()` elements,
        // unaliased, and cannot overlap `values` because it was just reserved. The returned
        // borrow is tied to `&self`.
        unsafe {
            ptr.copy_from_nonoverlapping(NonNull::from(values).cast::<T>(), values.len());
            core::slice::from_raw_parts_mut(ptr.as_ptr(), values.len())
        }
    }

    /// Allocates a slice of `len` copies of `value`.
    ///
    /// # Panics
    ///
    /// Panics if the slice would overflow `isize` bytes.
    #[expect(clippy::mut_from_ref, reason = "every call returns a distinct allocation")]
    pub fn alloc_slice_fill<T: Copy>(&self, len: usize, value: T) -> &mut [T] {
        let layout = Layout::array::<T>(len).expect("arena request overflows isize");
        let ptr = self.alloc_layout(layout).cast::<T>();
        // SAFETY: `ptr` is aligned for `T`, valid for writes of `len` elements and
        // unaliased; every element is written before the slice is formed. The returned
        // borrow is tied to `&self`.
        unsafe {
            for i in 0..len {
                ptr.add(i).write(value);
            }
            core::slice::from_raw_parts_mut(ptr.as_ptr(), len)
        }
    }

    /// The bytes currently reserved from the system allocator.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.chunks.borrow().iter().map(|chunk| chunk.layout.size()).sum()
    }

    /// Frees every allocation made from the arena.
    ///
    /// In debug builds the retained memory is overwritten with `0xCD`, so a value that was
    /// smuggled past a reset reads as garbage rather than as its old contents.
    pub fn reset(&mut self) {
        let used = self.used.replace(0);
        let chunks = self.chunks.get_mut();
        if chunks.len() > 1 {
            let total = chunks.iter().map(|chunk| chunk.layout.size()).sum::<usize>();
            self.chunk_size.set(self.chunk_size.get().max(total));
            for chunk in chunks.drain(..) {
                // SAFETY: the chunk came from `std::alloc::alloc` with this layout and is
                // freed exactly once because `drain` removes it from the list.
                unsafe { std::alloc::dealloc(chunk.ptr.as_ptr(), chunk.layout) };
            }
        } else if cfg!(debug_assertions)
            && let Some(chunk) = chunks.first()
        {
            // SAFETY: `used <= chunk.layout.size()`, and `&mut self` proves no borrow of
            // the chunk's contents is alive.
            unsafe { chunk.ptr.write_bytes(POISON, used) };
        }
    }
}

impl Drop for Arena {
    fn drop(&mut self) {
        for chunk in self.chunks.get_mut().drain(..) {
            // SAFETY: the chunk came from `std::alloc::alloc` with this layout and is freed
            // exactly once because `drain` removes it from the list.
            unsafe { std::alloc::dealloc(chunk.ptr.as_ptr(), chunk.layout) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocations_are_distinct_and_aligned() {
        let arena = Arena::with_chunk_size(64);
        let a = arena.alloc(1u8);
        let b = arena.alloc(2u64);
        let c = arena.alloc(3u16);
        let d = arena.alloc(4u128);
        assert_eq!((*a, *b, *c, *d), (1, 2, 3, 4));
        assert_eq!(core::ptr::from_mut(b).addr() % align_of::<u64>(), 0);
        assert_eq!(core::ptr::from_mut(d).addr() % align_of::<u128>(), 0);
        *a = 9;
        *b = 10;
        assert_eq!((*a, *b, *c, *d), (9, 10, 3, 4));
    }

    #[test]
    fn slices_and_zero_sized_requests() {
        let arena = Arena::new();
        let copied = arena.alloc_slice_copy(&[1u32, 2, 3]);
        let filled = arena.alloc_slice_fill(1000, 7u64);
        let empty = arena.alloc_slice_copy::<u64>(&[]);
        arena.alloc(());
        copied[0] = 5;
        assert_eq!(copied, [5, 2, 3]);
        assert!(filled.iter().all(|&value| value == 7));
        assert!(empty.is_empty());
    }

    #[test]
    fn grows_past_a_chunk_then_settles_at_one() {
        let mut arena = Arena::with_chunk_size(128);
        for round in 0..3 {
            let values: Vec<&mut u64> = (0..100).map(|i| arena.alloc(i)).collect();
            assert!(values.iter().enumerate().all(|(i, value)| **value == i as u64));
            let chunks = arena.chunks.borrow().len();
            if round == 0 {
                assert!(chunks > 1);
            } else {
                assert_eq!(chunks, 1);
            }
            arena.reset();
        }
    }

    #[test]
    fn oversized_and_overaligned_requests() {
        #[derive(Clone, Copy)]
        #[repr(align(64))]
        struct Wide([u8; 64]);

        let arena = Arena::with_chunk_size(32);
        let big = arena.alloc_slice_fill(4096, 1u8);
        let wide = arena.alloc(Wide([2; 64]));
        assert_eq!(big.len(), 4096);
        assert_eq!(core::ptr::from_mut(wide).addr() % 64, 0);
        assert_eq!(wide.0[63], 2);
    }

    #[test]
    fn arena_moves_between_threads() {
        let mut arena = Arena::new();
        arena.alloc(1u32);
        std::thread::spawn(move || {
            arena.reset();
            assert_eq!(*arena.alloc(2u32), 2);
        })
        .join()
        .unwrap();
    }
}
