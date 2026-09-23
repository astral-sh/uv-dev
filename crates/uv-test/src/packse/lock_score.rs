//! Source-qualified version counts from a serialized universal lockfile.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use uv_normalize::PackageName;
use uv_pep440::Version;

/// An inventory of the versioned package records in one universal lockfile.
///
/// The score counts additional distinct versions for each normalized package name and exact
/// serialized source identity. It does not measure marker coverage, prove dependency correctness,
/// or claim that the resolver found a globally minimal solution. Unversioned packages are reported
/// separately because their versions cannot be inferred from the lockfile.
#[derive(Debug, Serialize)]
pub struct LockVersionScore {
    schema: &'static str,
    lock_version: u32,
    lock_revision: Option<u32>,
    package_records: usize,
    package_names: usize,
    versioned_records: usize,
    unversioned_records: usize,
    excess_versions: usize,
    groups: Vec<SourceVersions>,
}

impl LockVersionScore {
    /// Additional distinct versions after the first in each source-qualified package group.
    pub fn excess_versions(&self) -> usize {
        self.excess_versions
    }
}

#[derive(Debug, Serialize)]
struct SourceVersions {
    name: PackageName,
    source: PackageSource,
    versions: BTreeSet<Version>,
    unversioned: bool,
}

#[derive(Debug, Deserialize)]
struct LockDocument {
    version: u32,
    revision: Option<u32>,
    #[serde(default)]
    package: Vec<PackageRecord>,
}

#[derive(Debug, Deserialize)]
struct PackageRecord {
    name: PackageName,
    version: Option<Version>,
    source: PackageSource,
}

// Keep the source kind and all of its identity-bearing fields. In particular, distinct registries,
// Git revisions, URL subdirectories, and local source kinds must not collapse into one group.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(untagged, deny_unknown_fields)]
enum PackageSource {
    Registry {
        registry: String,
    },
    Git {
        git: String,
    },
    Url {
        url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        subdirectory: Option<String>,
    },
    Path {
        path: String,
    },
    Directory {
        directory: String,
    },
    Editable {
        editable: String,
    },
    Virtual {
        r#virtual: String,
    },
}

/// Count source-qualified duplicate versions without running a resolver or modifying the lock.
pub fn score_lock_versions(contents: &str) -> Result<LockVersionScore> {
    let document: LockDocument =
        toml::from_str(contents).context("failed to read lockfile package identities")?;
    ensure!(
        document.version == 1,
        "unsupported lockfile version {}",
        document.version
    );

    let package_records = document.package.len();
    let mut names = BTreeSet::new();
    let mut groups = BTreeMap::<(PackageName, PackageSource), SourceVersions>::new();
    let mut versioned_records = 0;
    let mut unversioned_records = 0;
    for package in document.package {
        names.insert(package.name.clone());
        let group = groups
            .entry((package.name.clone(), package.source.clone()))
            .or_insert_with(|| SourceVersions {
                name: package.name,
                source: package.source,
                versions: BTreeSet::new(),
                unversioned: false,
            });
        if let Some(version) = package.version {
            ensure!(
                group.versions.insert(version.clone()),
                "duplicate lockfile identity for {}=={version}",
                group.name
            );
            versioned_records += 1;
        } else {
            ensure!(
                !group.unversioned,
                "duplicate unversioned lockfile identity for {}",
                group.name
            );
            group.unversioned = true;
            unversioned_records += 1;
        }
    }

    Ok(LockVersionScore {
        schema: "uv-dev.lock-version-score.v1",
        lock_version: document.version,
        lock_revision: document.revision,
        package_records,
        package_names: names.len(),
        versioned_records,
        unversioned_records,
        excess_versions: groups
            .values()
            .map(|group| group.versions.len().saturating_sub(1))
            .sum(),
        groups: groups.into_values().collect(),
    })
}

#[cfg(test)]
mod tests {
    use std::fmt::Write;

    use super::*;

    #[test]
    fn counts_versions_within_each_source() -> Result<()> {
        let score = score_lock_versions(
            r#"
version = 1
revision = 3
[[package]]
name = "Shared_Package"
version = "2.0"
source = { registry = "https://first.example/simple" }
[[package]]
name = "shared-package"
version = "1.0"
source = { registry = "https://first.example/simple" }
[[package]]
name = "shared-package"
version = "3.0"
source = { registry = "https://second.example/simple" }
[[package]]
name = "project"
source = { virtual = "." }
"#,
        )?;

        assert_eq!(score.package_records, 4);
        assert_eq!(score.package_names, 2);
        assert_eq!(score.versioned_records, 3);
        assert_eq!(score.unversioned_records, 1);
        assert_eq!(score.excess_versions, 1);
        assert_eq!(score.groups.len(), 3);
        insta::assert_json_snapshot!(score, @r###"
        {
          "schema": "uv-dev.lock-version-score.v1",
          "lock_version": 1,
          "lock_revision": 3,
          "package_records": 4,
          "package_names": 2,
          "versioned_records": 3,
          "unversioned_records": 1,
          "excess_versions": 1,
          "groups": [
            {
              "name": "project",
              "source": {
                "virtual": "."
              },
              "versions": [],
              "unversioned": true
            },
            {
              "name": "shared-package",
              "source": {
                "registry": "https://first.example/simple"
              },
              "versions": [
                "1.0",
                "2.0"
              ],
              "unversioned": false
            },
            {
              "name": "shared-package",
              "source": {
                "registry": "https://second.example/simple"
              },
              "versions": [
                "3.0"
              ],
              "unversioned": false
            }
          ]
        }
        "###);
        Ok(())
    }

    #[test]
    fn distinguishes_every_serialized_source_kind() -> Result<()> {
        let sources = [
            r#"{ registry = "https://example.org/source" }"#,
            r#"{ git = "https://example.org/source#1111111" }"#,
            r#"{ git = "https://example.org/source#2222222" }"#,
            r#"{ url = "https://example.org/source" }"#,
            r#"{ url = "https://example.org/source", subdirectory = "one" }"#,
            r#"{ url = "https://example.org/source", subdirectory = "two" }"#,
            r#"{ path = "source" }"#,
            r#"{ directory = "source" }"#,
            r#"{ editable = "source" }"#,
            r#"{ virtual = "source" }"#,
        ];
        let mut contents = String::from("version = 1\n");
        for (index, source) in sources.iter().enumerate() {
            writeln!(
                contents,
                "[[package]]\nname = 'shared'\nversion = '{}.0'\nsource = {source}",
                index + 1
            )?;
        }
        let score = score_lock_versions(&contents)?;
        assert_eq!(score.groups.len(), sources.len());
        assert_eq!(score.excess_versions, 0);
        Ok(())
    }

    #[test]
    fn rejects_duplicate_normalized_identities() {
        let contents = r#"
version = 1
[[package]]
name = "Shared_Package"
version = "1.0"
source = { registry = "https://example.org/simple" }
[[package]]
name = "shared-package"
version = "1.0.0"
source = { registry = "https://example.org/simple" }
"#;
        insta::assert_snapshot!(score_lock_versions(contents).expect_err("duplicate package"),
            @"duplicate lockfile identity for shared-package==1.0.0");
    }

    #[test]
    fn rejects_ambiguous_or_unknown_sources() {
        for source in [
            r#"{ registry = "https://example.org/simple", git = "https://example.org/git" }"#,
            r#"{ url = "https://example.org/archive", branch = "main" }"#,
            r#"{ unknown = "source" }"#,
            "{}",
        ] {
            let contents = format!(
                "version = 1\n[[package]]\nname = 'shared'\nversion = '1.0'\nsource = {source}\n"
            );
            assert!(score_lock_versions(&contents).is_err(), "{source}");
        }
    }

    #[test]
    fn rejects_duplicate_unversioned_identities_and_unknown_versions() {
        let contents = r#"
version = 1
[[package]]
name = "project"
source = { editable = "." }
[[package]]
name = "project"
source = { editable = "." }
"#;
        insta::assert_snapshot!(score_lock_versions(contents).expect_err("duplicate package"),
            @"duplicate unversioned lockfile identity for project");
        insta::assert_snapshot!(score_lock_versions("version = 2").expect_err("unknown format"),
            @"unsupported lockfile version 2");
    }
}
