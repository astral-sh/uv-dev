mod entry;
mod registered;

use entry::Entry;
pub use entry::{Abandoned, Producer};

pub use registered::{RegisteredEntry, RegisteredOnceMap, Registration};

use std::borrow::Borrow;
use std::fmt::{Debug, Formatter};
use std::hash::{BuildHasher, Hash, RandomState};
use std::sync::Arc;

use papaya::{HashMap, ResizeMode};

/// Coordinate shared tasks and store their results in a parallel hash map.
///
/// We often have jobs `Fn(K) -> V` that we only want to run once and memoize, e.g. network
/// requests for metadata. When multiple tasks start the same query in parallel, e.g. through source
/// dist builds, we want to wait until the other task is done and get a reference to the same
/// result.
///
/// Note that this always clones the value out of the underlying map. Because
/// of this, it's common to wrap the `V` in an `Arc<V>` to make cloning cheap.
pub struct OnceMap<K, V, S = RandomState> {
    items: HashMap<K, Arc<Entry<V>>, S>,
}

impl<K: Eq + Hash + Debug, V: Debug, S: BuildHasher + Clone> Debug for OnceMap<K, V, S> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Debug::fmt(&self.items, f)
    }
}

impl<K: Eq + Hash + Clone, V: Clone, H: BuildHasher + Clone> OnceMap<K, V, H> {
    fn entry(&self, key: K) -> Arc<Entry<V>> {
        self.items
            .pin()
            .get_or_insert_with(key, || Arc::new(Entry::vacant()))
            .clone()
    }

    /// Register work whose completion is owned by an external request queue.
    fn register(&self, key: K) -> bool {
        self.entry(key).claim().is_some()
    }

    /// Claim a job, or wait for its cached result.
    ///
    /// [`Registration::New`] owns the obligation to publish with [`Producer::done`]. Dropping
    /// that producer wakes waiters to compete for a new claim. Ordinary results, including
    /// errors, remain cached until removed. Exactly one waiter can claim each retry.
    ///
    /// ```
    /// use uv_once_map::{OnceMap, Registration};
    /// # async fn example(cache: &OnceMap<String, usize>, id: String) -> usize {
    /// match cache.register_or_wait(&id).await {
    ///     Registration::Existing(value) => value,
    ///     Registration::New(producer) => {
    ///         let value = id.len();
    ///         producer.done(value);
    ///         value
    ///     }
    /// }
    /// # }
    /// ```
    pub async fn register_or_wait(&self, key: &K) -> Registration<Producer<V>, V> {
        loop {
            let entry = self.entry(key.clone());
            if let Some(value) = entry.get() {
                return Registration::Existing(value);
            }
            if let Some(generation) = entry.claim() {
                return Registration::New(Producer::new(entry, generation));
            }
            if let Some(value) = entry.wait().await {
                return Registration::Existing(value);
            }
        }
    }

    /// Wait for an existing entry without registering a missing key.
    async fn wait_registered(&self, key: &K) -> Option<V> {
        let entry = self.items.pin().get(key)?.clone();
        entry.wait().await
    }

    /// Submit the result of externally queued work, or seed a completed result.
    pub fn done(&self, key: K, value: V) {
        self.entry(key).done(value);
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
        items.remove(key)?.take()
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
                .map(|(k, v)| (k, Arc::new(Entry::filled(v))))
                .collect(),
        }
    }
}
