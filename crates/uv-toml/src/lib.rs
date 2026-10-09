use std::collections::BTreeMap;
use std::hash::{BuildHasher, Hash};

use indexmap::IndexMap;

use serde::{Deserialize, Deserializer};

/// A map implementation used by standardized `pyproject.toml` tables.
///
/// This trait supports both insertion-ordered maps for external tooling and sorted maps for uv's
/// internal representations. It is an implementation detail of the generic table wrappers.
#[doc(hidden)]
pub trait TableMap<K, V>: Default {
    /// Insert a value, returning the previous value for the key, if present.
    fn insert(&mut self, key: K, value: V) -> Option<V>;
}

impl<K: Ord, V> TableMap<K, V> for BTreeMap<K, V> {
    fn insert(&mut self, key: K, value: V) -> Option<V> {
        Self::insert(self, key, value)
    }
}

impl<K, V, S> TableMap<K, V> for IndexMap<K, V, S>
where
    K: Eq + Hash,
    S: BuildHasher + Default,
{
    fn insert(&mut self, key: K, value: V) -> Option<V> {
        Self::insert(self, key, value)
    }
}

/// Deserialize a map while ensuring all keys are unique.
pub fn deserialize_unique_map<'de, D, K, V, F, Map>(
    deserializer: D,
    error_msg: F,
) -> Result<Map, D::Error>
where
    D: Deserializer<'de>,
    K: Deserialize<'de> + Clone,
    V: Deserialize<'de>,
    F: FnOnce(&K) -> String,
    Map: TableMap<K, V>,
{
    struct Visitor<K, V, F, Map>(F, std::marker::PhantomData<(K, V, Map)>);

    impl<'de, K, V, F, Map> serde::de::Visitor<'de> for Visitor<K, V, F, Map>
    where
        K: Deserialize<'de> + Clone,
        V: Deserialize<'de>,
        F: FnOnce(&K) -> String,
        Map: TableMap<K, V>,
    {
        type Value = Map;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a map with unique keys")
        }

        fn visit_map<M>(self, mut access: M) -> Result<Self::Value, M::Error>
        where
            M: serde::de::MapAccess<'de>,
        {
            let mut map = Map::default();
            while let Some((key, value)) = access.next_entry::<K, V>()? {
                if map.insert(key.clone(), value).is_some() {
                    return Err(serde::de::Error::custom((self.0)(&key)));
                }
            }
            Ok(map)
        }
    }

    deserializer.deserialize_map(Visitor(error_msg, std::marker::PhantomData))
}
