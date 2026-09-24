//! Temporary storage for allocator-aware collections.

use std::cell::Cell;

#[expect(unsafe_code, reason = "implementing Allocator requires raw memory operations")]
mod scratch;

pub use bumpalo::Bump as Arena;
pub use scratch::ScratchAllocator;

thread_local! {
    static ARENA: Cell<Option<Arena>> = const { Cell::new(None) };
}

/// Run an operation with temporary storage backed by a reusable thread-local arena.
///
/// Collections such as `Vec<T, &Arena>` retain their ordinary element destruction semantics.
/// Allocations must remain within the callback. Nested operations use a separate arena, and
/// unusually large allocations are released instead of being retained by the thread.
pub fn with_arena<R>(operation: impl FnOnce(&Arena) -> R) -> R {
    let mut arena = ARENA
        .try_with(Cell::take)
        .ok()
        .flatten()
        .unwrap_or_default();
    let result = operation(&arena);
    if arena.allocated_bytes() <= 64 * 1024 {
        arena.reset();
        let _ = ARENA.try_with(|slot| slot.set(Some(arena)));
    }
    result
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::{ARENA, with_arena};

    #[test]
    fn nested_operations_keep_outer_storage_alive() {
        let result = with_arena(|arena| {
            let mut values = Vec::new_in(arena);
            values.extend([1, 2, 3]);
            let inner = with_arena(|arena| Box::new_in(4, arena).as_ref().to_owned());
            values.push(inner);
            values.iter().sum::<u32>()
        });
        assert_eq!(result, 10);
    }

    #[test]
    fn collections_drop_their_elements() {
        struct CountDrop<'a>(&'a Cell<usize>);
        impl Drop for CountDrop<'_> {
            fn drop(&mut self) {
                self.0.set(self.0.get() + 1);
            }
        }
        let dropped = Cell::new(0);
        with_arena(|arena| {
            let mut values = Vec::new_in(arena);
            values.push(CountDrop(&dropped));
            values.push(CountDrop(&dropped));
            let _value = Box::new_in(CountDrop(&dropped), arena);
        });
        assert_eq!(dropped.get(), 3);
    }

    #[test]
    fn large_arenas_are_released() {
        with_arena(|arena| {
            let mut values = Vec::with_capacity_in(128 * 1024, arena);
            values.resize(128 * 1024, 0u8);
        });
        assert!(ARENA.with(Cell::take).is_none());
    }
}
