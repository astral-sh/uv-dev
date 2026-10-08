//! Detect and describe incompatible environment selections.

use itertools::Itertools;

use uv_command_support::conjunction;
use uv_configuration::{DependencyGroupsWithDefaults, ExtrasSpecification};
use uv_lock::Installable;
use uv_pypi_types::{ConflictItem, ConflictKind, ConflictSet};

use crate::EnvironmentError;
use crate::install_target::InstallTarget;

#[derive(Debug)]
pub struct ConflictError {
    /// The set from which the conflict was derived.
    set: ConflictSet,
    /// The items from the set that were enabled, and thus create the conflict.
    conflicts: Vec<ConflictItem>,
    /// Enabled dependency groups with defaults applied.
    groups: DependencyGroupsWithDefaults,
}

impl std::fmt::Display for ConflictError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Format the set itself.
        let set = self
            .set
            .iter()
            .map(|item| match item.kind() {
                ConflictKind::Project => format!("{}", item.package()),
                ConflictKind::Extra(extra) => format!("`{}[{}]`", item.package(), extra),
                ConflictKind::Group(group) => format!("`{}:{}`", item.package(), group),
            })
            .join(", ");

        // If all the conflicts are of the same kind, show a more succinct error.
        if self
            .conflicts
            .iter()
            .all(|conflict| matches!(conflict.kind(), ConflictKind::Extra(..)))
        {
            write!(
                f,
                "Extras {} are incompatible with the declared conflicts: {{{set}}}",
                conjunction(
                    self.conflicts
                        .iter()
                        .map(|conflict| match conflict.kind() {
                            ConflictKind::Extra(extra) => format!("`{extra}`"),
                            ConflictKind::Group(..) | ConflictKind::Project => unreachable!(),
                        })
                        .collect()
                )
            )
        } else if self
            .conflicts
            .iter()
            .all(|conflict| matches!(conflict.kind(), ConflictKind::Group(..)))
        {
            write!(
                f,
                "Groups {} are incompatible with the conflicts: {{{set}}}",
                conjunction(
                    self.conflicts
                        .iter()
                        .map(|conflict| match conflict.kind() {
                            ConflictKind::Group(group)
                                if self.groups.contains_because_default(group) =>
                                format!("`{group}` (enabled by default)"),
                            ConflictKind::Group(group) => format!("`{group}`"),
                            ConflictKind::Extra(..) | ConflictKind::Project => unreachable!(),
                        })
                        .collect()
                )
            )
        } else {
            write!(
                f,
                "{} are incompatible with the declared conflicts: {{{set}}}",
                conjunction(
                    self.conflicts
                        .iter()
                        .enumerate()
                        .map(|(i, conflict)| {
                            let conflict = match conflict.kind() {
                                ConflictKind::Project => {
                                    format!("package `{}`", conflict.package())
                                }
                                ConflictKind::Extra(extra) => format!("extra `{extra}`"),
                                ConflictKind::Group(group)
                                    if self.groups.contains_because_default(group) =>
                                {
                                    format!("group `{group}` (enabled by default)")
                                }
                                ConflictKind::Group(group) => format!("group `{group}`"),
                            };
                            if i == 0 {
                                capitalize(&conflict)
                            } else {
                                conflict
                            }
                        })
                        .collect()
                )
            )
        }
    }
}

impl std::error::Error for ConflictError {}

/// Capitalize the first letter of a string.
fn capitalize(value: &str) -> String {
    let mut characters = value.chars();
    match characters.next() {
        None => String::new(),
        Some(character) => character.to_uppercase().collect::<String>() + characters.as_str(),
    }
}

/// Validate that we aren't trying to install extras or groups that
/// are declared as conflicting.
pub fn detect_conflicts(
    target: &InstallTarget,
    extras: &ExtrasSpecification,
    groups: &DependencyGroupsWithDefaults,
) -> Result<(), EnvironmentError> {
    // Validate that we aren't trying to install extras or groups that
    // are declared as conflicting. Note that we need to collect all
    // extras and groups that match in a particular set, since extras
    // can be declared as conflicting with groups. So if extra `x` and
    // group `g` are declared as conflicting, then enabling both of
    // those should result in an error.
    let lock = target.lock();
    let packages = target.packages(extras, groups);
    let conflicts = lock.conflicts();
    for set in conflicts.iter() {
        let mut conflicts: Vec<ConflictItem> = vec![];
        for item in set.iter() {
            if !packages.contains(item.package()) {
                // Ignore items that are not in the install targets
                continue;
            }
            let is_conflicting = match item.kind() {
                ConflictKind::Project => groups.prod(),
                ConflictKind::Extra(extra) => extras.contains(extra),
                ConflictKind::Group(group1) => groups.contains(group1),
            };
            if is_conflicting {
                conflicts.push(item.clone());
            }
        }
        if conflicts.len() >= 2 {
            return Err(EnvironmentError::Conflict(ConflictError {
                set: set.clone(),
                conflicts,
                groups: groups.clone(),
            }));
        }
    }
    Ok(())
}
