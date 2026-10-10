use std::collections::BTreeMap;

use uv_normalize::{ExtraName, GroupName, PackageName};
use uv_pep508::MarkerTree;

use crate::pyproject::{Source, Sources};

/// The location of a selected `tool.uv.sources` declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceOrigin {
    /// The sources were read from the project.
    Project,
    /// The sources were read from the workspace root.
    Workspace,
}

/// The configured sources and marker partitions for a requirement.
///
/// A project declaration replaces the workspace declaration for the package, even when none of
/// its entries apply to the selected extra or dependency group.
#[derive(Debug)]
pub struct SourceSelection<'a> {
    sources: &'a Sources,
    origin: SourceOrigin,
    marker: MarkerTree,
    extra: Option<&'a ExtraName>,
    group: Option<&'a GroupName>,
}

impl<'a> SourceSelection<'a> {
    /// Select the applicable declaration, retaining its source scope and requirement markers.
    pub fn new(
        name: &PackageName,
        marker: MarkerTree,
        project_sources: Option<&'a BTreeMap<PackageName, Sources>>,
        workspace_sources: &'a BTreeMap<PackageName, Sources>,
        extra: Option<&'a ExtraName>,
        group: Option<&'a GroupName>,
    ) -> Option<Self> {
        let (sources, origin) =
            if let Some(sources) = project_sources.and_then(|sources| sources.get(name)) {
                (sources, SourceOrigin::Project)
            } else {
                (workspace_sources.get(name)?, SourceOrigin::Workspace)
            };
        Some(Self {
            sources,
            origin,
            marker,
            extra,
            group,
        })
    }

    /// Return the location whose paths and other source settings apply.
    pub fn origin(&self) -> SourceOrigin {
        self.origin
    }

    /// Iterate over sources in the selected scope and their intersected requirement markers.
    ///
    /// Entries with false markers remain available so callers can validate their source settings.
    pub fn iter(&self) -> impl Iterator<Item = (&'a Source, MarkerTree)> + '_ {
        self.sources
            .iter()
            .filter(|source| {
                source.extra().is_none_or(|extra| self.extra == Some(extra))
                    && source.group().is_none_or(|group| self.group == Some(group))
            })
            .map(|source| (source, self.marker.and(source.marker())))
    }

    /// Return the requirement region not covered by a configured source in the selected scope.
    pub fn remaining_marker(&self) -> MarkerTree {
        let covered = self
            .iter()
            .fold(MarkerTree::FALSE, |covered, (_, marker)| covered.or(marker));
        self.marker.and(covered.negate())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use anyhow::Result;
    use uv_normalize::{ExtraName, GroupName, PackageName};
    use uv_pep508::MarkerTree;

    use crate::pyproject::Sources;

    use super::{SourceOrigin, SourceSelection};

    #[test]
    fn project_declaration_shadows_workspace_before_scope_filtering() -> Result<()> {
        let project_sources: BTreeMap<PackageName, Sources> =
            toml::from_str(r#"dependency = { workspace = true, extra = "optional" }"#)?;
        let workspace_sources: BTreeMap<PackageName, Sources> =
            toml::from_str("dependency = { workspace = true }")?;
        let name = "dependency".parse()?;
        let marker = "python_full_version >= '3.12'".parse()?;
        let selection = SourceSelection::new(
            &name,
            marker,
            Some(&project_sources),
            &workspace_sources,
            None,
            None,
        )
        .expect("the project declares sources");

        assert_eq!(selection.origin(), SourceOrigin::Project);
        assert_eq!(selection.iter().count(), 0);
        assert_eq!(selection.remaining_marker(), marker);
        Ok(())
    }

    #[test]
    fn workspace_sources_partition_the_requirement_domain() -> Result<()> {
        let project_sources = BTreeMap::new();
        let workspace_sources: BTreeMap<PackageName, Sources> = toml::from_str(
            r#"dependency = { workspace = true, marker = "sys_platform == 'linux'" }"#,
        )?;
        let name = "dependency".parse()?;
        let marker: MarkerTree = "python_full_version >= '3.12'".parse()?;
        let selection = SourceSelection::new(
            &name,
            marker,
            Some(&project_sources),
            &workspace_sources,
            None,
            None,
        )
        .expect("the workspace declares sources");
        let selected = "python_full_version >= '3.12' and sys_platform == 'linux'".parse()?;
        let remaining = "python_full_version >= '3.12' and sys_platform != 'linux'".parse()?;

        assert_eq!(selection.origin(), SourceOrigin::Workspace);
        assert_eq!(
            selection
                .iter()
                .map(|(_, marker)| marker)
                .collect::<Vec<_>>(),
            vec![selected]
        );
        assert_eq!(selection.remaining_marker(), remaining);
        assert_eq!(selected.or(remaining), marker);
        assert!(selected.and(remaining).is_false());
        assert!(
            SourceSelection::new(
                &"undeclared".parse()?,
                marker,
                Some(&project_sources),
                &workspace_sources,
                None,
                None,
            )
            .is_none()
        );
        Ok(())
    }

    #[test]
    fn sources_apply_only_to_the_selected_extra_or_group() -> Result<()> {
        let project_sources: BTreeMap<PackageName, Sources> = toml::from_str(
            r#"
            dependency = [
                { workspace = true, marker = "sys_platform == 'linux'" },
                { workspace = true, marker = "sys_platform == 'win32'", extra = "optional" },
                { workspace = true, marker = "sys_platform == 'darwin'", group = "development" },
            ]
            "#,
        )?;
        let workspace_sources = BTreeMap::new();
        let name = "dependency".parse()?;
        let extra: ExtraName = "optional".parse()?;
        let group: GroupName = "development".parse()?;
        let production = SourceSelection::new(
            &name,
            MarkerTree::TRUE,
            Some(&project_sources),
            &workspace_sources,
            None,
            None,
        )
        .expect("the project declares sources");
        assert_eq!(production.iter().count(), 1);
        assert_eq!(
            production.remaining_marker(),
            "sys_platform != 'linux'".parse()?
        );

        let optional = SourceSelection::new(
            &name,
            MarkerTree::TRUE,
            Some(&project_sources),
            &workspace_sources,
            Some(&extra),
            None,
        )
        .expect("the project declares sources");
        assert_eq!(optional.iter().count(), 2);
        assert_eq!(
            optional.remaining_marker(),
            "sys_platform != 'linux' and sys_platform != 'win32'".parse()?
        );

        let development = SourceSelection::new(
            &name,
            MarkerTree::TRUE,
            Some(&project_sources),
            &workspace_sources,
            None,
            Some(&group),
        )
        .expect("the project declares sources");
        assert_eq!(development.iter().count(), 2);
        assert_eq!(
            development.remaining_marker(),
            "sys_platform != 'linux' and sys_platform != 'darwin'".parse()?
        );
        Ok(())
    }

    #[test]
    fn disjoint_sources_remain_available_for_validation() -> Result<()> {
        let project_sources: BTreeMap<PackageName, Sources> = toml::from_str(
            r#"dependency = { workspace = true, marker = "sys_platform == 'win32'" }"#,
        )?;
        let workspace_sources = BTreeMap::new();
        let marker = "sys_platform == 'linux'".parse()?;
        let selection = SourceSelection::new(
            &"dependency".parse()?,
            marker,
            Some(&project_sources),
            &workspace_sources,
            None,
            None,
        )
        .expect("the project declares sources");

        assert_eq!(
            selection
                .iter()
                .map(|(_, marker)| marker)
                .collect::<Vec<_>>(),
            vec![MarkerTree::FALSE]
        );
        assert_eq!(selection.remaining_marker(), marker);
        Ok(())
    }
}
