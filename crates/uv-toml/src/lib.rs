use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer};

/// Deserialize a map while ensuring all keys are unique.
pub fn deserialize_unique_map<'de, D, K, V, F>(
    deserializer: D,
    error_msg: F,
) -> Result<BTreeMap<K, V>, D::Error>
where
    D: Deserializer<'de>,
    K: Deserialize<'de> + Ord,
    V: Deserialize<'de>,
    F: FnOnce(&K) -> String,
{
    deserialize_unique_map_with_expectation(deserializer, "a map with unique keys", error_msg)
}

/// Deserialize a map with unique keys and a caller-provided description of the expected value.
pub fn deserialize_unique_map_with_expectation<'de, D, K, V, F>(
    deserializer: D,
    expectation: &'static str,
    error_msg: F,
) -> Result<BTreeMap<K, V>, D::Error>
where
    D: Deserializer<'de>,
    K: Deserialize<'de> + Ord,
    V: Deserialize<'de>,
    F: FnOnce(&K) -> String,
{
    struct Visitor<K, V, F>(&'static str, F, std::marker::PhantomData<(K, V)>);

    impl<'de, K, V, F> serde::de::Visitor<'de> for Visitor<K, V, F>
    where
        K: Deserialize<'de> + Ord,
        V: Deserialize<'de>,
        F: FnOnce(&K) -> String,
    {
        type Value = BTreeMap<K, V>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str(self.0)
        }

        fn visit_map<M>(self, mut access: M) -> Result<Self::Value, M::Error>
        where
            M: serde::de::MapAccess<'de>,
        {
            use std::collections::btree_map::Entry;

            let mut map = BTreeMap::new();
            while let Some((key, value)) = access.next_entry::<K, V>()? {
                match map.entry(key) {
                    Entry::Occupied(entry) => {
                        return Err(serde::de::Error::custom((self.1)(entry.key())));
                    }
                    Entry::Vacant(entry) => {
                        entry.insert(value);
                    }
                }
            }
            Ok(map)
        }
    }

    deserializer.deserialize_map(Visitor(expectation, error_msg, std::marker::PhantomData))
}

#[cfg(test)]
mod tests {
    use serde::de::value::{Error, I32Deserializer};

    use super::deserialize_unique_map;

    #[test]
    fn unique_map_type_error() {
        let error = deserialize_unique_map::<_, String, String, _>(
            I32Deserializer::<Error>::new(42),
            |key| format!("duplicate key `{key}`"),
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "invalid type: integer `42`, expected a map with unique keys"
        );
    }
}
