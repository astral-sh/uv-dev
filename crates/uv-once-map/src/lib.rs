mod registered;

pub use registered::{RegisteredEntry, RegisteredOnceMap, Registration};

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
    items: HashMap<K, Value<V>, S>,
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
        match self.claim_or_wait(key).await {
            Claim::Ready(value) => Some(value),
            Claim::New(_) => None,
        }
    }

    /// Return a cached result or initialize it once. If initialization is cancelled, another
    /// caller can take over the job. If another caller publishes a result while initialization is
    /// in progress, that result takes precedence.
    pub async fn get_or_init(&self, key: K, init: impl Future<Output = V>) -> V {
        match self.claim_or_wait(&key).await {
            Claim::Ready(value) => value,
            Claim::New(notify) => {
                let claim = PendingClaim {
                    map: self,
                    key,
                    notify,
                    completed: false,
                };
                claim.complete(init.await)
            }
        }
    }

    async fn claim_or_wait(&self, key: &K) -> Claim<V> {
        loop {
            let notify = {
                let items = self.items.pin();
                match items.try_insert_with(key.clone(), || Value::Waiting(Arc::new(Notify::new())))
                {
                    Ok(Value::Waiting(notify)) => return Claim::New(Arc::clone(notify)),
                    Ok(Value::Filled(_)) => unreachable!("inserted a pending job"),
                    Err(Value::Waiting(notify)) => Arc::clone(notify),
                    Err(value) => {
                        if let Some(value) = value.get() {
                            return Claim::Ready(value);
                        }
                        continue;
                    }
                }
            };

            if let Some(value) = self.wait_pending(key, notify).await {
                return Claim::Ready(value);
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

        // Check that this is still the same pending job after registering the waiter.
        match self.items.pin().get(key) {
            Some(Value::Waiting(current)) if Arc::ptr_eq(current, &notify) => {}
            Some(value) => return value.get(),
            None => return None,
        }

        notification.await;
        self.get(key)
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
        match items.remove(key)? {
            Value::Waiting(notify) => {
                notify.notify_waiters();
                None
            }
            value @ Value::Filled(_) => value.take(),
        }
    }
}

enum Claim<V> {
    Ready(V),
    New(Arc<Notify>),
}

struct PendingClaim<'a, K: Eq + Hash, V, H: BuildHasher> {
    map: &'a OnceMap<K, V, H>,
    key: K,
    notify: Arc<Notify>,
    completed: bool,
}

impl<K: Eq + Hash + Clone, V: Clone, H: BuildHasher + Clone> PendingClaim<'_, K, V, H> {
    fn complete(mut self, value: V) -> V {
        let items = self.map.items.pin();
        let result = items.compute(self.key.clone(), |entry| match entry {
            Some((_, Value::Waiting(notify))) if Arc::ptr_eq(notify, &self.notify) => {
                Operation::Insert(Value::filled(value.clone()))
            }
            entry => Operation::Abort(entry.and_then(|(_, current)| current.get())),
        });
        self.completed = true;
        match result {
            Compute::Updated { .. } => {
                self.notify.notify_waiters();
                value
            }
            Compute::Aborted(published) => published.unwrap_or(value),
            Compute::Inserted(..) | Compute::Removed(..) => {
                unreachable!("claim completion can only update or abort")
            }
        }
    }
}

impl<K: Eq + Hash, V, H: BuildHasher> Drop for PendingClaim<'_, K, V, H> {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        let removed = matches!(
            self.map.items.pin().remove_if(&self.key, |_, value| {
                matches!(value, Value::Waiting(notify) if Arc::ptr_eq(notify, &self.notify))
            }),
            Ok(Some(_))
        );
        if removed {
            self.notify.notify_waiters();
        }
    }
}

impl<K: Eq + Hash + Clone, V, H: Default + BuildHasher + Clone> Default for OnceMap<K, V, H> {
    fn default() -> Self {
        Self {
            items: HashMap::builder()
                .hasher(H::default())
                .resize_mode(ResizeMode::Blocking)
                .build(),
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
            items: iter
                .into_iter()
                .map(|(k, v)| (k, Value::filled(v)))
                .collect(),
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
