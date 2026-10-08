use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use uv_normalize::{ExtraName, GroupName};

/// Select extras or dependency groups from a package. An empty selection requests the base package.
#[derive(Debug, Clone)]
pub enum RequirementSelection {
    Extras(Box<[ExtraName]>),
    Groups(Box<[GroupName]>),
}

impl Default for RequirementSelection {
    fn default() -> Self {
        Self::Extras(Box::default())
    }
}

impl RequirementSelection {
    /// Remove extra requests without changing dependency group requests.
    pub fn clear_extras(&mut self) {
        match self {
            Self::Extras(extras) => *extras = Box::default(),
            Self::Groups(_) => {}
        }
    }

    pub fn extras(&self) -> &[ExtraName] {
        match self {
            Self::Extras(extras) => extras,
            Self::Groups(_) => &[],
        }
    }

    pub fn groups(&self) -> &[GroupName] {
        match self {
            Self::Extras(_) => &[],
            Self::Groups(groups) => groups,
        }
    }

    pub fn into_extras(self) -> Box<[ExtraName]> {
        match self {
            Self::Extras(extras) => extras,
            Self::Groups(_) => Box::default(),
        }
    }
}

impl Serialize for RequirementSelection {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        match self {
            Self::Extras(extras) => {
                if !extras.is_empty() {
                    map.serialize_entry("extras", extras)?;
                }
            }
            Self::Groups(groups) => {
                if !groups.is_empty() {
                    map.serialize_entry("groups", groups)?;
                }
            }
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for RequirementSelection {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Fields {
            #[serde(default)]
            extras: Box<[ExtraName]>,
            #[serde(default)]
            groups: Box<[GroupName]>,
        }

        let Fields { extras, groups } = Fields::deserialize(deserializer)?;
        if groups.is_empty() {
            Ok(Self::Extras(extras))
        } else if extras.is_empty() {
            Ok(Self::Groups(groups))
        } else {
            Err(serde::de::Error::custom(
                "requirement extras and groups are mutually exclusive",
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use serde_json::json;
    use uv_cache_key::{cache_digest, hash_digest};

    use crate::Requirement;

    use super::RequirementSelection;

    #[test]
    fn requirement_selection_wire_compatibility() -> Result<(), Box<dyn std::error::Error>> {
        for (wire, cache) in [
            (
                json!({"name": "demo", "specifier": ">1,<2", "index": null, "conflict": null}),
                "0378bcfbbdcbf90a",
            ),
            (
                json!({"name": "demo", "specifier": ">1,<2", "extras": ["b", "a"], "index": null, "conflict": null}),
                "3886036b66966183",
            ),
            (
                json!({"name": "demo", "specifier": ">1,<2", "groups": ["test", "dev"], "index": null, "conflict": null}),
                "9720b2cea9cdc20f",
            ),
        ] {
            let requirement: Requirement = serde_json::from_value(wire.clone())?;
            assert_eq!(serde_json::to_value(&requirement)?, wire);
            assert_eq!(
                toml::from_str::<Requirement>(&toml::to_string(&requirement)?)?,
                requirement
            );
            assert_eq!(cache_digest(&requirement), cache);
            assert_eq!(
                hash_digest(&requirement),
                hash_digest(&(
                    &requirement.name,
                    requirement.extras(),
                    requirement.groups(),
                    &requirement.marker,
                    &requirement.source,
                    &requirement.scope,
                ))
            );
        }
        Ok(())
    }

    #[test]
    fn requirement_selection_rejects_mixed_fields() {
        let error = serde_json::from_value::<Requirement>(json!({
            "name": "demo", "specifier": "", "extras": ["extra"], "groups": ["group"]
        }))
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "requirement extras and groups are mutually exclusive"
        );
    }

    #[test]
    fn empty_selections_request_the_same_package() -> Result<(), Box<dyn std::error::Error>> {
        let base: Requirement = serde_json::from_value(json!({"name": "demo", "specifier": ""}))?;
        let groups = Requirement {
            selection: RequirementSelection::Groups(Box::default()),
            ..base.clone()
        };
        assert_eq!(base, groups);
        assert_eq!(base.cmp(&groups), Ordering::Equal);
        assert_eq!(hash_digest(&base), hash_digest(&groups));
        assert_eq!(cache_digest(&base), cache_digest(&groups));
        assert_eq!(serde_json::to_value(&base)?, serde_json::to_value(&groups)?);
        Ok(())
    }
}
