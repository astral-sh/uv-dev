use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;
use std::str::FromStr;

use uv_normalize::GroupName;
use uv_toml::deserialize_unique_map_with_expectation;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DependencyGroups(BTreeMap<GroupName, Vec<DependencyGroupSpecifier>>);

impl DependencyGroups {
    /// Returns the names of the dependency groups.
    pub fn keys(&self) -> impl Iterator<Item = &GroupName> {
        self.0.keys()
    }

    /// Returns the dependency group with the given name.
    pub fn get(&self, group: &GroupName) -> Option<&Vec<DependencyGroupSpecifier>> {
        self.0.get(group)
    }

    /// Returns `true` if the dependency group is in the list.
    pub fn contains_key(&self, group: &GroupName) -> bool {
        self.0.contains_key(group)
    }

    /// Returns an iterator over the dependency groups.
    pub(crate) fn iter(
        &self,
    ) -> impl Iterator<Item = (&GroupName, &Vec<DependencyGroupSpecifier>)> {
        self.0.iter()
    }
}

impl<'a> IntoIterator for &'a DependencyGroups {
    type Item = (&'a GroupName, &'a Vec<DependencyGroupSpecifier>);
    type IntoIter = std::collections::btree_map::Iter<'a, GroupName, Vec<DependencyGroupSpecifier>>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

/// Ensure that all keys in the TOML table are unique.
impl<'de> serde::de::Deserialize<'de> for DependencyGroups {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_unique_map_with_expectation(
            deserializer,
            "a table with unique dependency group names",
            |key: &GroupName| format!("duplicate dependency group: `{key}`"),
        )
        .map(Self)
    }
}

/// A specifier item in a [PEP 735](https://peps.python.org/pep-0735/) Dependency Group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum DependencyGroupSpecifier {
    /// A PEP 508-compatible requirement string.
    Requirement(String),
    /// A reference to another dependency group.
    IncludeGroup {
        /// The name of the group to include.
        include_group: GroupName,
    },
    /// A Dependency Object Specifier.
    Object(BTreeMap<String, String>),
}

impl<'de> Deserialize<'de> for DependencyGroupSpecifier {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = DependencyGroupSpecifier;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a string or a map with the `include-group` key")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(DependencyGroupSpecifier::Requirement(value.to_owned()))
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: serde::de::MapAccess<'de>,
            {
                let mut map_data = BTreeMap::new();
                while let Some((key, value)) = map.next_entry()? {
                    map_data.insert(key, value);
                }

                if map_data.is_empty() {
                    return Err(serde::de::Error::custom("missing field `include-group`"));
                }

                if map_data.len() == 1
                    && let Some(include_group) = map_data
                        .get("include-group")
                        .map(String::as_str)
                        .map(GroupName::from_str)
                        .transpose()
                        .map_err(serde::de::Error::custom)?
                {
                    Ok(DependencyGroupSpecifier::IncludeGroup { include_group })
                } else {
                    Ok(DependencyGroupSpecifier::Object(map_data))
                }
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

#[cfg(test)]
mod tests {
    use super::{DependencyGroupSpecifier, DependencyGroups};
    use serde::Deserialize;
    use std::str::FromStr;
    use uv_normalize::GroupName;

    #[test]
    fn dependency_groups_preserve_order_and_values() {
        let json =
            r#"{"z": ["zebra"], "Foo_Bar": ["foo>=1"], "a": [{"include-group": "Foo_Bar"}]}"#;
        let toml = r#"
z = ["zebra"]
Foo_Bar = ["foo>=1"]
a = [{include-group = "Foo_Bar"}]
"#;

        let json_groups: DependencyGroups = serde_json::from_str(json).unwrap();
        let toml_groups: DependencyGroups = toml_edit::de::from_str(toml).unwrap();
        assert_eq!(json_groups, toml_groups);
        assert_eq!(
            json_groups
                .keys()
                .map(GroupName::as_str)
                .collect::<Vec<_>>(),
            ["a", "foo-bar", "z"]
        );
        assert_eq!(
            json_groups.get(&GroupName::from_str("a").unwrap()).unwrap(),
            &[DependencyGroupSpecifier::IncludeGroup {
                include_group: GroupName::from_str("foo-bar").unwrap(),
            }]
        );
        assert_eq!(
            json_groups
                .get(&GroupName::from_str("foo-bar").unwrap())
                .unwrap(),
            &[DependencyGroupSpecifier::Requirement("foo>=1".to_owned())]
        );
    }

    #[test]
    fn dependency_groups_json_errors() {
        let errors = [
            r#"{"Foo_Bar": [], "foo-bar": []}"#,
            "[]",
            "null",
            r#"{"foo-bar": [], "foo_bar": [1]}"#,
        ]
        .into_iter()
        .map(|input| {
            let error = serde_json::from_str::<DependencyGroups>(input).unwrap_err();
            format!("{input}\n{error}")
        })
        .collect::<Vec<_>>()
        .join("\n\n");
        insta::assert_snapshot!(errors, @r#"
        {"Foo_Bar": [], "foo-bar": []}
        duplicate dependency group: `foo-bar` at line 1 column 30

        []
        invalid type: sequence, expected a table with unique dependency group names at line 1 column 0

        null
        invalid type: null, expected a table with unique dependency group names at line 1 column 4

        {"foo-bar": [], "foo_bar": [1]}
        invalid type: integer `1`, expected a string or a map with the `include-group` key at line 1 column 29
        "#);
    }

    #[test]
    fn dependency_groups_toml_errors() {
        #[derive(Debug, Deserialize)]
        struct Document {
            groups: DependencyGroups,
        }

        let empty: Document = toml_edit::de::from_str("groups = {}").unwrap();
        assert_eq!(empty.groups.keys().count(), 0);

        let errors = [
            "groups = { Foo_Bar = [], foo-bar = [] }",
            "groups = []",
            "groups = { foo-bar = [], foo_bar = [1] }",
        ]
        .into_iter()
        .map(|input| {
            let error = toml_edit::de::from_str::<Document>(input).unwrap_err();
            format!("{input}\n{error}")
        })
        .collect::<Vec<_>>()
        .join("\n\n");
        insta::assert_snapshot!(errors, @r"
        groups = { Foo_Bar = [], foo-bar = [] }
        TOML parse error at line 1, column 10
          |
        1 | groups = { Foo_Bar = [], foo-bar = [] }
          |          ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
        duplicate dependency group: `foo-bar`


        groups = []
        TOML parse error at line 1, column 10
          |
        1 | groups = []
          |          ^^
        invalid type: sequence, expected a table with unique dependency group names


        groups = { foo-bar = [], foo_bar = [1] }
        TOML parse error at line 1, column 37
          |
        1 | groups = { foo-bar = [], foo_bar = [1] }
          |                                     ^
        invalid type: integer `1`, expected a string or a map with the `include-group` key
        ");
    }
}
