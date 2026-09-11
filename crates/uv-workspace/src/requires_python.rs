use std::collections::{BTreeMap, BTreeSet};

use uv_normalize::{GroupName, PackageName};
use uv_pep440::VersionSpecifiers;

use crate::dependency_groups::GroupInclude;

/// The effective Python requirements of the selected workspace members and groups.
pub type RequiresPythonSources = BTreeMap<(PackageName, Option<GroupName>), VersionSpecifiers>;

/// The original declarations contributing to the selected workspace Python requirements.
pub type RequiresPythonDeclarations =
    BTreeMap<(PackageName, Option<GroupName>), RequiresPythonDeclaration>;

/// Semantic workspace bounds and their separately retained declaration provenance.
#[derive(Debug, Default)]
pub struct WorkspaceRequiresPython {
    pub requirements: RequiresPythonSources,
    pub declarations: RequiresPythonDeclarations,
}

/// One authored Python requirement and the include edges that make it active.
///
/// A declaration reached through several paths is represented once. Include occurrences are
/// deduplicated independently of the effective specifier intersection, so provenance does not
/// enumerate every path through a dependency-group graph.
#[derive(Debug, Clone)]
pub struct RequiresPythonDeclaration {
    pub specifiers: VersionSpecifiers,
    pub includes: BTreeSet<GroupInclude>,
}

impl RequiresPythonDeclaration {
    pub(crate) fn new(specifiers: VersionSpecifiers) -> Self {
        Self {
            specifiers,
            includes: BTreeSet::new(),
        }
    }

    pub(crate) fn extend_includes(&mut self, other: Self) {
        self.includes.extend(other.includes);
    }
}
