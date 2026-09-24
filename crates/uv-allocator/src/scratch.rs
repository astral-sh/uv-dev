use std::alloc::{AllocError, Allocator, Global, Layout};
use std::ptr::{self, NonNull};

use crate::Arena;

/// An allocator for temporary collections that can grow beyond a scratch arena.
///
/// Small buffers use the arena. Larger buffers use [`Global`], which can reclaim
/// their previous allocations as they grow instead of retaining them until reset.
#[derive(Clone, Copy)]
pub struct ScratchAllocator<'a> {
    arena: &'a Arena,
}

const ARENA_LIMIT: usize = 16 * 1024;

impl<'a> ScratchAllocator<'a> {
    pub fn new(arena: &'a Arena) -> Self {
        Self { arena }
    }

    /// Report only the requested size so every fitting layout selects the same allocator.
    fn exact_size(block: NonNull<[u8]>, layout: Layout) -> NonNull<[u8]> {
        NonNull::slice_from_raw_parts(block.cast(), layout.size())
    }
}

// SAFETY: Every block is owned by the arena or Global according to its requested size.
// The returned slice has exactly that size, so all fitting layouts select the same
// allocator. Copies share the same arena, which outlives every returned allocation.
unsafe impl Allocator for ScratchAllocator<'_> {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        let block = if layout.size() <= ARENA_LIMIT {
            self.arena.allocate(layout)
        } else {
            Global.allocate(layout)
        }?;
        Ok(Self::exact_size(block, layout))
    }

    fn allocate_zeroed(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        let block = if layout.size() <= ARENA_LIMIT {
            self.arena.allocate_zeroed(layout)
        } else {
            Global.allocate_zeroed(layout)
        }?;
        Ok(Self::exact_size(block, layout))
    }

    unsafe fn deallocate(&self, pointer: NonNull<u8>, layout: Layout) {
        // SAFETY: Exact allocation sizes ensure the fitting layout selects the owner.
        unsafe {
            if layout.size() <= ARENA_LIMIT {
                self.arena.deallocate(pointer, layout);
            } else {
                Global.deallocate(pointer, layout);
            }
        }
    }

    unsafe fn grow(
        &self,
        pointer: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        // SAFETY: Both layouts select the same owner in the delegated cases. When
        // crossing the limit, the new allocation is distinct and has room for all
        // old bytes. The old block remains valid if allocation fails.
        let block = unsafe {
            if new_layout.size() <= ARENA_LIMIT {
                self.arena.grow(pointer, old_layout, new_layout)?
            } else if old_layout.size() > ARENA_LIMIT {
                Global.grow(pointer, old_layout, new_layout)?
            } else {
                let block = self.allocate(new_layout)?;
                ptr::copy_nonoverlapping(
                    pointer.as_ptr(),
                    block.cast().as_ptr(),
                    old_layout.size(),
                );
                self.deallocate(pointer, old_layout);
                block
            }
        };
        Ok(Self::exact_size(block, new_layout))
    }

    unsafe fn grow_zeroed(
        &self,
        pointer: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        // SAFETY: The delegated cases keep the same owner. The crossing case
        // copies only the old bytes into a distinct zeroed allocation, leaving
        // the new tail zeroed and preserving the old block on allocation failure.
        let block = unsafe {
            if new_layout.size() <= ARENA_LIMIT {
                self.arena.grow_zeroed(pointer, old_layout, new_layout)?
            } else if old_layout.size() > ARENA_LIMIT {
                Global.grow_zeroed(pointer, old_layout, new_layout)?
            } else {
                let block = self.allocate_zeroed(new_layout)?;
                ptr::copy_nonoverlapping(
                    pointer.as_ptr(),
                    block.cast().as_ptr(),
                    old_layout.size(),
                );
                self.deallocate(pointer, old_layout);
                block
            }
        };
        Ok(Self::exact_size(block, new_layout))
    }

    unsafe fn shrink(
        &self,
        pointer: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        // SAFETY: Both layouts select the same owner in the delegated cases. When
        // crossing the limit, the distinct new allocation holds the retained prefix.
        // The old block remains valid if allocation fails.
        let block = unsafe {
            if old_layout.size() <= ARENA_LIMIT {
                self.arena.shrink(pointer, old_layout, new_layout)?
            } else if new_layout.size() > ARENA_LIMIT {
                Global.shrink(pointer, old_layout, new_layout)?
            } else {
                let block = self.allocate(new_layout)?;
                ptr::copy_nonoverlapping(
                    pointer.as_ptr(),
                    block.cast().as_ptr(),
                    new_layout.size(),
                );
                self.deallocate(pointer, old_layout);
                block
            }
        };
        Ok(Self::exact_size(block, new_layout))
    }
}

#[cfg(test)]
mod tests {
    use std::alloc::{Allocator, Layout};
    use std::cell::Cell;

    use super::{ARENA_LIMIT, ScratchAllocator};
    use crate::with_arena;

    #[test]
    fn growing_and_shrinking_across_the_limit() {
        with_arena(|arena| {
            let allocator = ScratchAllocator::new(arena);
            let small = Layout::from_size_align(128, 128).unwrap();
            let large = Layout::from_size_align(ARENA_LIMIT * 2, 256).unwrap();
            let block = allocator.allocate_zeroed(small).unwrap();
            assert_eq!(block.len(), small.size());
            // SAFETY: All operations use the block's current layout. Slice lengths
            // are bounded by the allocation, and each successful resize replaces it.
            unsafe {
                assert!(block.as_ref().iter().all(|byte| *byte == 0));
                block.cast::<u8>().as_ptr().write_bytes(42, small.size());
                let block = allocator.grow_zeroed(block.cast(), small, large).unwrap();
                assert_eq!(block.len(), large.size());
                assert_eq!(block.cast::<u8>().as_ptr().addr() % large.align(), 0);
                assert!(
                    block.as_ref()[..small.size()]
                        .iter()
                        .all(|byte| *byte == 42)
                );
                assert!(block.as_ref()[small.size()..].iter().all(|byte| *byte == 0));
                let block = allocator.shrink(block.cast(), large, small).unwrap();
                assert_eq!(block.len(), small.size());
                assert!(block.as_ref().iter().all(|byte| *byte == 42));
                allocator.deallocate(block.cast(), small);
            }
        });
    }

    #[test]
    fn failed_growth_keeps_the_old_allocation() {
        with_arena(|arena| {
            let allocator = ScratchAllocator::new(arena);
            let layout = Layout::new::<u64>();
            let impossible = Layout::from_size_align(isize::MAX as usize, 1).unwrap();
            let block = allocator.allocate(layout).unwrap();
            // SAFETY: The initial block fits a u64. Failed growth leaves it live,
            // so it can still be read and deallocated with its original layout.
            unsafe {
                block.cast::<u64>().as_ptr().write(42);
                assert!(allocator.grow(block.cast(), layout, impossible).is_err());
                assert_eq!(block.cast::<u64>().as_ptr().read(), 42);
                allocator.deallocate(block.cast(), layout);
            }
        });
    }

    #[test]
    fn collections_drop_elements_after_moving_between_allocators() {
        struct CountDrop<'a>(&'a Cell<usize>);
        impl Drop for CountDrop<'_> {
            fn drop(&mut self) {
                self.0.set(self.0.get() + 1);
            }
        }
        let dropped = Cell::new(0);
        with_arena(|arena| {
            let mut values = Vec::new_in(ScratchAllocator::new(arena));
            for _ in 0..4096 {
                values.push(CountDrop(&dropped));
            }
            values.truncate(2);
            values.shrink_to_fit();
        });
        assert_eq!(dropped.get(), 4096);
    }
}
