use std::{
    collections::BTreeMap,
    ops::{Deref, DerefMut},
    str::FromStr,
};

use jiff::Timestamp;
use rustc_hash::FxHashSet;
use serde::ser::SerializeMap;
use uv_distribution_types::{ExcludeNewerOverride, ExcludeNewerValue};
use uv_normalize::PackageName;
use uv_preview::PreviewFeature;
use uv_warnings::warn_user_once;

/// The configuration layer that supplied the effective `exclude-newer` cutoff for a package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EffectiveExcludeNewerSource {
    /// The global `exclude-newer` setting.
    Global,
    /// A package-specific `exclude-newer-package` override.
    Package,
    /// An index-specific `[[tool.uv.index]].exclude-newer` override.
    Index,
}

pub struct ExcludeNewerValueWithSpanRef<'a>(pub &'a ExcludeNewerValue);

impl serde::Serialize for ExcludeNewerValueWithSpanRef<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if let Some(span) = self.0.span() {
            let mut map = serializer.serialize_map(Some(2))?;
            map.serialize_entry("timestamp", &self.0.timestamp())?;
            map.serialize_entry("span", span)?;
            map.end()
        } else {
            self.0.timestamp().serialize(serializer)
        }
    }
}

/// A package-specific exclude-newer entry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ExcludeNewerPackageEntry {
    package: PackageName,
    setting: ExcludeNewerOverride,
}

impl FromStr for ExcludeNewerPackageEntry {
    type Err = String;

    /// Parses a [`ExcludeNewerPackageEntry`] from a string in the format `PACKAGE=DATE` or `PACKAGE=false`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let Some((package, value)) = s.split_once('=') else {
            return Err(format!(
                "Invalid `exclude-newer-package` value `{s}`: expected format `PACKAGE=DATE` or `PACKAGE=false`"
            ));
        };

        let package = PackageName::from_str(package).map_err(|err| {
            format!("Invalid `exclude-newer-package` package name `{package}`: {err}")
        })?;

        let setting = if value == "false" {
            ExcludeNewerOverride::Disabled
        } else {
            ExcludeNewerOverride::Enabled(Box::new(ExcludeNewerValue::from_str(value).map_err(
                |err| format!("Invalid `exclude-newer-package` value `{value}`: {err}"),
            )?))
        };

        Ok(Self { package, setting })
    }
}

impl From<(PackageName, ExcludeNewerOverride)> for ExcludeNewerPackageEntry {
    fn from((package, setting): (PackageName, ExcludeNewerOverride)) -> Self {
        Self { package, setting }
    }
}

impl From<(PackageName, ExcludeNewerValue)> for ExcludeNewerPackageEntry {
    fn from((package, timestamp): (PackageName, ExcludeNewerValue)) -> Self {
        Self {
            package,
            setting: ExcludeNewerOverride::Enabled(Box::new(timestamp)),
        }
    }
}

pub fn serialize_exclude_newer_package_with_spans<S>(
    value: &Option<ExcludeNewerPackage>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let Some(value) = value else {
        return serializer.serialize_none();
    };

    let mut map = serializer.serialize_map(Some(value.len()))?;
    for (name, setting) in value {
        match setting {
            ExcludeNewerOverride::Disabled => map.serialize_entry(name, &false)?,
            ExcludeNewerOverride::Enabled(value) => {
                map.serialize_entry(name, &ExcludeNewerValueWithSpanRef(value.as_ref()))?;
            }
        }
    }
    map.end()
}

/// Package-specific `exclude-newer` settings.
///
/// Entries are stored in package-name order for deterministic serialization.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ExcludeNewerPackage(BTreeMap<PackageName, ExcludeNewerOverride>);

impl Deref for ExcludeNewerPackage {
    type Target = BTreeMap<PackageName, ExcludeNewerOverride>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for ExcludeNewerPackage {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl FromIterator<ExcludeNewerPackageEntry> for ExcludeNewerPackage {
    fn from_iter<T: IntoIterator<Item = ExcludeNewerPackageEntry>>(iter: T) -> Self {
        Self(
            iter.into_iter()
                .map(|entry| (entry.package, entry.setting))
                .collect(),
        )
    }
}

impl IntoIterator for ExcludeNewerPackage {
    type Item = (PackageName, ExcludeNewerOverride);
    type IntoIter = std::collections::btree_map::IntoIter<PackageName, ExcludeNewerOverride>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a ExcludeNewerPackage {
    type Item = (&'a PackageName, &'a ExcludeNewerOverride);
    type IntoIter = std::collections::btree_map::Iter<'a, PackageName, ExcludeNewerOverride>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl ExcludeNewerPackage {
    /// Returns true if this map is empty (no package-specific settings).
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A setting that excludes files newer than a timestamp, at a global level or per-package.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ExcludeNewer {
    /// Global timestamp that applies to all packages if no package-specific timestamp is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub global: Option<ExcludeNewerValue>,
    /// Per-package timestamps that override the global timestamp.
    #[serde(default, skip_serializing_if = "ExcludeNewerPackage::is_empty")]
    pub package: ExcludeNewerPackage,
}

impl ExcludeNewer {
    /// Create a new exclude newer configuration with just a global timestamp.
    pub fn global(global: ExcludeNewerValue) -> Self {
        Self {
            global: Some(global),
            package: ExcludeNewerPackage::default(),
        }
    }

    fn warn_index_exclude_newer_preview() {
        if !uv_preview::is_enabled(PreviewFeature::IndexExcludeNewer) {
            warn_user_once!(
                "Setting `exclude-newer` on configured indexes is experimental and may change without warning. Pass `--preview-features {}` to disable this warning.",
                PreviewFeature::IndexExcludeNewer
            );
        }
    }

    /// Create from CLI arguments.
    pub fn from_args(
        global: Option<ExcludeNewerOverride>,
        package: Vec<ExcludeNewerPackageEntry>,
    ) -> Self {
        let global = global.and_then(ExcludeNewerOverride::into_value);
        let package: ExcludeNewerPackage = package.into_iter().collect();

        Self { global, package }
    }

    /// Returns the effective exclude-newer timestamp for a specific package, falling back to the
    /// global value if no package-specific setting exists.
    pub(crate) fn exclude_newer_package(&self, package_name: &PackageName) -> Option<Timestamp> {
        match self.package.get(package_name) {
            Some(ExcludeNewerOverride::Enabled(value)) => Some(value.timestamp()),
            Some(ExcludeNewerOverride::Disabled) => None,
            None => self.global.as_ref().map(ExcludeNewerValue::timestamp),
        }
    }

    /// Returns the effective exclude-newer timestamp for a package resolved from a specific index.
    pub fn exclude_newer_package_for_index(
        &self,
        package_name: &PackageName,
        index: Option<&ExcludeNewerOverride>,
    ) -> Option<Timestamp> {
        self.exclude_newer_package_for_index_with_source(package_name, index)
            .map(|(timestamp, _)| timestamp)
    }

    /// Returns the effective exclude-newer timestamp and its source for a package resolved from a
    /// specific index.
    pub(crate) fn exclude_newer_package_for_index_with_source(
        &self,
        package_name: &PackageName,
        index: Option<&ExcludeNewerOverride>,
    ) -> Option<(Timestamp, EffectiveExcludeNewerSource)> {
        match self.package.get(package_name) {
            Some(ExcludeNewerOverride::Enabled(value)) => {
                Some((value.timestamp(), EffectiveExcludeNewerSource::Package))
            }
            Some(ExcludeNewerOverride::Disabled) => None,
            None => match index {
                Some(ExcludeNewerOverride::Disabled) => {
                    Self::warn_index_exclude_newer_preview();
                    None
                }
                Some(ExcludeNewerOverride::Enabled(value)) => Some((
                    {
                        Self::warn_index_exclude_newer_preview();
                        value.timestamp()
                    },
                    EffectiveExcludeNewerSource::Index,
                )),
                None => self
                    .global
                    .as_ref()
                    .map(|value| (value.timestamp(), EffectiveExcludeNewerSource::Global)),
            },
        }
    }

    /// Returns true if this has any configuration (global or per-package).
    pub(crate) fn is_empty(&self) -> bool {
        self.global.is_none() && self.package.is_empty()
    }
    /// Filter package-specific settings to packages in the resolution.
    #[must_use]
    pub fn filter_packages<'a>(self, packages: impl IntoIterator<Item = &'a PackageName>) -> Self {
        let packages = packages.into_iter().collect::<FxHashSet<_>>();
        Self {
            global: self.global,
            package: ExcludeNewerPackage(
                self.package
                    .into_iter()
                    .filter(|(package, _)| packages.contains(package))
                    .collect(),
            ),
        }
    }
}

impl std::fmt::Display for ExcludeNewer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(global) = &self.global {
            write!(f, "global: {global}")?;
            if !self.package.is_empty() {
                write!(f, ", ")?;
            }
        }
        let mut first = true;
        for (name, setting) in &self.package {
            if !first {
                write!(f, ", ")?;
            }
            match setting {
                ExcludeNewerOverride::Enabled(timestamp) => {
                    write!(f, "{name}: {}", timestamp.as_ref())?;
                }
                ExcludeNewerOverride::Disabled => {
                    write!(f, "{name}: disabled")?;
                }
            }
            first = false;
        }
        Ok(())
    }
}
