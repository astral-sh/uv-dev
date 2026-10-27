use std::collections::VecDeque;
use std::hash::Hash;

use rustc_hash::FxHashSet;

/// A first-in, first-out queue that accepts each distinct value at most once.
///
/// Popped values remain recorded and cannot be pushed again.
#[derive(Debug)]
pub struct OnceQueue<T> {
    queue: VecDeque<T>,
    seen: FxHashSet<T>,
}

impl<T> Default for OnceQueue<T> {
    fn default() -> Self {
        Self {
            queue: VecDeque::new(),
            seen: FxHashSet::default(),
        }
    }
}

impl<T> OnceQueue<T>
where
    T: Clone + Eq + Hash,
{
    /// Push a value onto the queue if it has not been seen before.
    ///
    /// Returns `true` if the value was added.
    pub fn push(&mut self, value: T) -> bool {
        if self.seen.insert(value.clone()) {
            self.queue.push_back(value);
            true
        } else {
            false
        }
    }

    /// Pop the next value from the queue.
    pub fn pop(&mut self) -> Option<T> {
        self.queue.pop_front()
    }

    /// Return all values accepted by the queue, including values already popped.
    pub fn into_seen(self) -> FxHashSet<T> {
        self.seen
    }
}

impl<T: Clone + Eq + Hash> Extend<T> for OnceQueue<T> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        for value in iter {
            self.push(value);
        }
    }
}

impl<T: Clone + Eq + Hash> FromIterator<T> for OnceQueue<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut queue = Self::default();
        queue.extend(iter);
        queue
    }
}

#[cfg(test)]
mod tests {
    use super::OnceQueue;

    #[test]
    fn accepts_each_value_once() {
        let mut queue = OnceQueue::default();

        assert!(queue.push("a"));
        assert!(queue.push("b"));
        assert!(!queue.push("a"));

        assert_eq!(queue.pop(), Some("a"));
        assert!(!queue.push("a"));
        assert_eq!(queue.pop(), Some("b"));
        assert_eq!(queue.pop(), None);
        assert_eq!(queue.into_seen().len(), 2);
    }

    #[test]
    fn collects_and_extends_in_first_seen_order() {
        let mut queue = ["a", "b", "a"].into_iter().collect::<OnceQueue<_>>();
        assert_eq!(queue.pop(), Some("a"));
        queue.extend(["b", "a", "c"]);
        assert_eq!(queue.pop(), Some("b"));
        assert_eq!(queue.pop(), Some("c"));
        assert_eq!(queue.pop(), None);
    }
}
