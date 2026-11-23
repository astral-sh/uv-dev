use std::fmt::{self, Debug};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::Notify;

/// A stable slot whose pending generations can be abandoned and retried.
#[derive(Debug)]
pub(super) struct Entry<V>(Mutex<Value<V>>);

#[derive(Debug)]
enum Value<V> {
    Vacant,
    Removed,
    Waiting(Arc<Generation>),
    Filled(V),
}

#[derive(Debug, Default)]
pub(super) struct Generation {
    notify: Notify,
    abandoned: AtomicBool,
}

impl<V> Entry<V> {
    pub(super) fn vacant() -> Self {
        Self(Mutex::new(Value::Vacant))
    }

    pub(super) fn filled(value: V) -> Self {
        Self(Mutex::new(Value::Filled(value)))
    }

    fn lock(&self) -> MutexGuard<'_, Value<V>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn claim(&self) -> Option<Arc<Generation>> {
        let mut value = self.lock();
        match *value {
            Value::Vacant => {
                let generation = Arc::new(Generation::default());
                *value = Value::Waiting(generation.clone());
                Some(generation)
            }
            Value::Waiting(_) | Value::Filled(_) | Value::Removed => None,
        }
    }

    pub(super) fn done(&self, result: V) {
        let previous = {
            let mut value = self.lock();
            if let Value::Removed = &*value {
                return;
            }
            std::mem::replace(&mut *value, Value::Filled(result))
        };
        if let Value::Waiting(generation) = previous {
            generation.notify.notify_waiters();
        }
    }

    pub(super) fn take(&self) -> Option<V> {
        let previous = {
            let mut value = self.lock();
            let previous = std::mem::replace(&mut *value, Value::Removed);
            if let Value::Waiting(generation) = &previous {
                generation.abandoned.store(true, Ordering::Release);
            }
            previous
        };
        match previous {
            Value::Vacant | Value::Removed => None,
            Value::Waiting(generation) => {
                generation.notify.notify_waiters();
                None
            }
            Value::Filled(value) => Some(value),
        }
    }
}

impl<V: Clone> Entry<V> {
    pub(super) fn get(&self) -> Option<V> {
        match &*self.lock() {
            Value::Filled(value) => Some(value.clone()),
            Value::Waiting(_) | Value::Vacant | Value::Removed => None,
        }
    }

    /// Wait for a result, or report that the producer abandoned its generation.
    pub(super) async fn wait(&self) -> Option<V> {
        let generation = match &*self.lock() {
            Value::Filled(value) => return Some(value.clone()),
            Value::Vacant | Value::Removed => return None,
            Value::Waiting(generation) => generation.clone(),
        };
        // Notify records calls to notify_waiters made after this future is created, even if
        // the future has not yet been polled. Recheck the state after creating it.
        let notification = generation.notify.notified();
        {
            let value = self.lock();
            if generation.abandoned.load(Ordering::Acquire) {
                return None;
            }
            match &*value {
                Value::Filled(value) => return Some(value.clone()),
                Value::Vacant | Value::Removed => return None,
                Value::Waiting(current) => {
                    if !Arc::ptr_eq(current, &generation) {
                        return None;
                    }
                }
            }
        }
        notification.await;
        // A new generation may have started already. Its eventual result does not complete
        // a waiter registered to the abandoned generation.
        let value = self.lock();
        if generation.abandoned.load(Ordering::Acquire) {
            return None;
        }
        match &*value {
            Value::Filled(value) => Some(value.clone()),
            Value::Waiting(_) | Value::Vacant | Value::Removed => None,
        }
    }
}

/// The obligation to complete one generation of a cached job.
///
/// Dropping this handle without calling [`Self::done`] abandons the generation and wakes its
/// waiters. A late completion cannot replace a result seeded separately or a newer generation.
#[derive(Debug)]
#[must_use = "dropping the producer abandons this job"]
pub struct Producer<V> {
    entry: Arc<Entry<V>>,
    generation: Arc<Generation>,
}

impl<V> Producer<V> {
    pub(super) fn new(entry: Arc<Entry<V>>, generation: Arc<Generation>) -> Self {
        Self { entry, generation }
    }

    /// Publish the result if this producer still owns the pending generation.
    pub fn done(self, result: V) {
        let published = {
            let mut value = self.entry.lock();
            if let Value::Waiting(current) = &*value
                && Arc::ptr_eq(current, &self.generation)
            {
                *value = Value::Filled(result);
                true
            } else {
                false
            }
        };
        if published {
            self.generation.notify.notify_waiters();
        }
    }
}

impl<V> Drop for Producer<V> {
    fn drop(&mut self) {
        let abandoned = {
            let mut value = self.entry.lock();
            if let Value::Waiting(current) = &*value
                && Arc::ptr_eq(current, &self.generation)
            {
                *value = Value::Vacant;
                self.generation.abandoned.store(true, Ordering::Release);
                true
            } else {
                false
            }
        };
        if abandoned {
            self.generation.notify.notify_waiters();
        }
    }
}

/// A registered job was abandoned before publishing a result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Abandoned;

impl fmt::Display for Abandoned {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("The registered task was abandoned before completion")
    }
}

impl std::error::Error for Abandoned {}
