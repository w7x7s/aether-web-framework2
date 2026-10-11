//! # Aether Request-Scoped Arena Allocator
//!
//! Provides zero-allocation bump allocation for the lifetime of a single HTTP request.
//! When a request finishes, resetting the arena is an O(1) pointer reset.

use std::alloc::{alloc, dealloc, Layout};
use std::cell::Cell;
use std::ptr::NonNull;

/// Default arena capacity per connection worker: 4KB (fits 99% of requests)
pub const DEFAULT_ARENA_CAPACITY: usize = 4096;

/// A cache-friendly, bump-pointer memory arena scoped to a single request lifecycle.
pub struct RequestArena {
    ptr: NonNull<u8>,
    capacity: usize,
    offset: Cell<usize>,
}

impl RequestArena {
    /// Creates a new arena with the specified capacity aligned to 64-byte cache lines.
    pub fn with_capacity(capacity: usize) -> Self {
        let layout = Layout::from_size_align(capacity, 64)
            .expect("Valid memory layout for RequestArena");
        let raw = unsafe { alloc(layout) };
        let ptr = NonNull::new(raw).expect("Memory allocation succeeded");

        Self {
            ptr,
            capacity,
            offset: Cell::new(0),
        }
    }

    /// Allocates `size` bytes with the specified alignment from the arena.
    ///
    /// # Safety
    /// Returned memory is valid for the duration of the request until `reset()` is called.
    #[inline(always)]
    pub fn alloc(&self, layout: Layout) -> Option<NonNull<u8>> {
        let current_offset = self.offset.get();
        let align_mask = layout.align() - 1;
        let aligned_offset = (current_offset + align_mask) & !align_mask;

        let next_offset = aligned_offset.checked_add(layout.size())?;
        if next_offset <= self.capacity {
            self.offset.set(next_offset);
            unsafe {
                Some(NonNull::new_unchecked(self.ptr.as_ptr().add(aligned_offset)))
            }
        } else {
            // Buffer exceeded; fallback to auxiliary heap or growth
            None
        }
    }

    /// Allocates and initializes a borrowed slice `&'req [T]` in the arena.
    #[inline(always)]
    pub fn alloc_slice<T: Copy>(&self, slice: &[T]) -> Option<&[T]> {
        let layout = Layout::for_value(slice);
        let dest = self.alloc(layout)?;
        unsafe {
            std::ptr::copy_nonoverlapping(slice.as_ptr(), dest.as_ptr() as *mut T, slice.len());
            Some(std::slice::from_raw_parts(dest.as_ptr() as *const T, slice.len()))
        }
    }

    /// O(1) Instant reset for the next incoming request on the same persistent connection.
    #[inline(always)]
    pub fn reset(&self) {
        self.offset.set(0);
    }

    /// Returns current allocated bytes in the arena.
    #[inline(always)]
    pub fn bytes_used(&self) -> usize {
        self.offset.get()
    }
}

impl Drop for RequestArena {
    fn drop(&mut self) {
        let layout = Layout::from_size_align(self.capacity, 64).unwrap();
        unsafe {
            dealloc(self.ptr.as_ptr(), layout);
        }
    }
}

// RequestArena exclusively owns its heap allocation and is safe to transfer between threads
unsafe impl Send for RequestArena {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arena_bump_and_reset() {
        let arena = RequestArena::with_capacity(1024);
        let s = arena.alloc_slice(b"hello aether").expect("Alloc slice");
        assert_eq!(s, b"hello aether");
        assert!(arena.bytes_used() >= 12);

        arena.reset();
        assert_eq!(arena.bytes_used(), 0);
    }
}
