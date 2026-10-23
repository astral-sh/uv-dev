mod registered;

pub use registered::{ProducerRegistration, RegisteredEntry, RegisteredOnceMap, Registration};

use std::borrow::Borrow;
use std::fmt::{Debug, Formatter};
use std::hash::{BuildHasher, Hash, RandomState};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use papaya::{Compute, HashMap, Operation, ResizeMode};
use tokio::sync::Notify;

/// Run tasks only once and store the results in a parallel hash map.
///
/// We often have jobs `Fn(K) -> V` that we only want to run once and memoize, e.g. network
/// requests for metadata. When multiple tasks start the same query in parallel, e.g. through source
/// dist builds, we want to wait until the other task is done and get a reference to the same
/// result.
///
/// Note that this always clones the value out of the underlying map. Because
/// of this, it's common to wrap the `V` in an `Arc<V>` to make cloning cheap.
pub struct OnceMap<K, V, S = RandomState> {
    items: Arc<HashMap<K, Value<V>, S>>,
}

impl<K: Eq + Hash + Debug, V: Debug, S: BuildHasher + Clone> Debug for OnceMap<K, V, S> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Debug::fmt(&self.items, f)
    }
}

impl<K: Eq + Hash + Clone, V: Clone, H: BuildHasher + Clone> OnceMap<K, V, H> {
    /// Register that you want to start a job.
    ///
    /// If this method returns `true`, you need to start a job and call [`OnceMap::done`] eventually
    /// or other tasks will hang. If it returns `false`, this job is already in progress and you
    /// can call [`OnceMap::register_or_wait`] to wait for the result.
    fn register(&self, key: K) -> bool {
        self.items
            .pin()
            .try_insert_with(key, || Value::Waiting(Arc::new(Notify::new())))
            .is_ok()
    }

    /// Register a job and return its producer, or return `None` if the key is already present.
    pub fn register_with_producer(&self, key: K) -> Option<Producer<K, V, H>> {
        let notify = Arc::new(Notify::new());
        self.items
            .pin()
            .try_insert(key.clone(), Value::Waiting(notify.clone()))
            .ok()
            .map(|_| Producer {
                items: self.items.clone(),
                key,
                notify,
                completed: false,
            })
    }

    /// Register that you want to start a job, unless it was already started, then wait for its
    /// result.
    ///
    /// Use this method for once-only operations.
    ///
    /// Returns `None` if the job needs to be started, otherwise returns the result of the job.
    ///
    ///  # Example
    ///
    /// ```rust,ignore
    /// if let Some(response) = cache.register_or_wait(&id).await {
    ///     response
    /// } else {
    ///     let response = fetch(&id).await;
    ///     cache.done(id, response.clone());
    ///     response
    /// }
    /// ```
    pub async fn register_or_wait(&self, key: &K) -> Option<V> {
        loop {
            let notify = {
                let items = self.items.pin();
                match items.try_insert_with(key.clone(), || Value::Waiting(Arc::new(Notify::new())))
                {
                    Ok(_) => return None,
                    Err(value) => match value {
                        Value::Filled(_) => return value.get(),
                        Value::Waiting(notify) => notify.clone(),
                    },
                }
            };

            if let Some(value) = self.wait_pending(key, notify).await {
                return Some(value);
            }
        }
    }

    /// Wait for an existing entry without registering a missing key.
    async fn wait_registered(&self, key: &K) -> Option<V> {
        let notify = {
            let items = self.items.pin();
            match items.get(key)? {
                value @ Value::Filled(_) => return value.get(),
                Value::Waiting(notify) => notify.clone(),
            }
        };
        self.wait_pending(key, notify).await
    }

    async fn wait_pending(&self, key: &K, notify: Arc<Notify>) -> Option<V> {
        // Register the waiter for calls to `notify_waiters`.
        let notification = notify.notified();

        // Make sure the value wasn't inserted or abandoned in-between us checking the map and
        // registering the waiter.
        {
            let items = self.items.pin();
            match items.get(key) {
                Some(Value::Filled(value)) => return Value::lock(value).clone(),
                Some(Value::Waiting(current)) if Arc::ptr_eq(current, &notify) => {}
                Some(Value::Waiting(_)) | None => return None,
            }
        }

        // Wait until the registration completes or is abandoned.
        notification.await;

        match self.items.pin().get(key) {
            Some(Value::Filled(value)) => Value::lock(value).clone(),
            Some(Value::Waiting(_)) | None => None,
        }
    }

    /// Submit the result of a job you registered.
    pub fn done(&self, key: K, value: V) {
        if let Some(Value::Waiting(notify)) = self.items.pin().insert(key, Value::filled(value)) {
            notify.notify_waiters();
        }
    }

    /// Return the result of a previous job, if any.
    pub fn get<Q: ?Sized + Hash + Eq>(&self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
    {
        let items = self.items.pin();
        items.get(key)?.get()
    }

    /// Remove the result of a previous job, if any.
    pub fn remove<Q: ?Sized + Hash + Eq>(&self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
    {
        let items = self.items.pin();
        let value = items.remove(key)?;
        if let Value::Waiting(notify) = &value {
            notify.notify_waiters();
        }
        value.take()
    }
}

impl<K: Eq + Hash + Clone, V, H: Default + BuildHasher + Clone> Default for OnceMap<K, V, H> {
    fn default() -> Self {
        Self {
            items: Arc::new(
                HashMap::builder()
                    .hasher(H::default())
                    .resize_mode(ResizeMode::Blocking)
                    .build(),
            ),
        }
    }
}

impl<K, V, H> FromIterator<(K, V)> for OnceMap<K, V, H>
where
    K: Eq + Hash,
    H: Default + Clone + BuildHasher,
{
    fn from_iter<T: IntoIterator<Item = (K, V)>>(iter: T) -> Self {
        Self {
            items: Arc::new(
                iter.into_iter()
                    .map(|(k, v)| (k, Value::filled(v)))
                    .collect(),
            ),
        }
    }
}

/// Ownership of a registered job.
///
/// Dropping the producer while it still owns a pending registration abandons that registration
/// and wakes its waiters. A later request can then register the same key again.
#[must_use = "dropping the producer abandons this job"]
pub struct Producer<K: Eq + Hash, V, S: BuildHasher = RandomState> {
    items: Arc<HashMap<K, Value<V>, S>>,
    key: K,
    notify: Arc<Notify>,
    completed: bool,
}

impl<K: Eq + Hash, V, S: BuildHasher> Debug for Producer<K, V, S> {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Producer").finish_non_exhaustive()
    }
}

impl<K: Eq + Hash + Clone, V: Clone, S: BuildHasher + Clone> Producer<K, V, S> {
    /// Publish the result if this producer still owns the pending registration.
    pub fn done(mut self, value: V) {
        let notify = self.notify.clone();
        let items = self.items.pin();
        let result = items.compute(self.key.clone(), |entry| {
            if let Some((_, Value::Waiting(current))) = entry
                && Arc::ptr_eq(current, &notify)
            {
                Operation::Insert(Value::filled(value.clone()))
            } else {
                Operation::Abort(())
            }
        });
        self.completed = true;
        if matches!(result, Compute::Updated { .. }) {
            self.notify.notify_waiters();
        }
    }
}

impl<K: Eq + Hash, V, S: BuildHasher> Drop for Producer<K, V, S> {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        let notify = &self.notify;
        if matches!(
            self.items.pin().remove_if(&self.key, |_, value| {
                matches!(value, Value::Waiting(current) if Arc::ptr_eq(current, notify))
            }),
            Ok(Some(_))
        ) {
            self.notify.notify_waiters();
        }
    }
}

#[derive(Debug)]
enum Value<V> {
    Waiting(Arc<Notify>),
    /// The mutex is a workaround to papaya always returning borrowed instead of owned values.
    Filled(Mutex<Option<V>>),
}

impl<V> Value<V> {
    fn filled(value: V) -> Self {
        Self::Filled(Mutex::new(Some(value)))
    }

    fn lock(value: &Mutex<Option<V>>) -> MutexGuard<'_, Option<V>> {
        value.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn take(&self) -> Option<V> {
        match self {
            Self::Filled(value) => Self::lock(value).take(),
            Self::Waiting(_) => None,
        }
    }
}

impl<V: Clone> Value<V> {
    fn get(&self) -> Option<V> {
        match self {
            Self::Filled(value) => Self::lock(value).clone(),
            Self::Waiting(_) => None,
        }
    }
}
