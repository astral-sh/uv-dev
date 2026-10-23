//! Add related dependencies while retaining their positions in the edited document.

use uv_normalize::DEV_DEPENDENCIES;
use uv_pep508::Requirement;
use uv_workspace::pyproject::{DependencyType, Source};

use crate::{ArrayEdit, Error, PyProjectTomlMut};

/// A dependency edit with its final position after the batch is applied.
#[derive(Debug, Clone)]
pub struct DependencyEdit {
    pub dependency_type: DependencyType,
    pub requirement: Requirement,
    pub source: Option<Source>,
    pub edit: ArrayEdit,
}

/// A batch of related dependency additions.
///
/// Preparing a requirement can fail, so callers add each requirement as it becomes available.
/// Later sorted insertions update the positions of earlier edits before the batch is returned.
pub struct DependencyEditBatch<'a> {
    toml: &'a mut PyProjectTomlMut,
    dependency_type: &'a DependencyType,
    raw: bool,
    edits: Vec<DependencyEdit>,
}

impl<'a> DependencyEditBatch<'a> {
    /// Begin editing dependencies, reserving space for the expected number of edits.
    pub fn with_capacity(
        toml: &'a mut PyProjectTomlMut,
        dependency_type: &'a DependencyType,
        raw: bool,
        capacity: usize,
    ) -> Self {
        Self {
            toml,
            dependency_type,
            raw,
            edits: Vec::with_capacity(capacity),
        }
    }

    /// Add a prepared requirement and update the positions of preceding edits.
    pub fn add(&mut self, requirement: Requirement, source: Option<Source>) -> Result<(), Error> {
        // Determine the dependency type.
        let dependency_type = match self.dependency_type {
            DependencyType::Dev => {
                let existing = self.toml.find_dependency(&requirement.name, None);
                if existing.iter().any(|dependency_type| matches!(dependency_type, DependencyType::Group(group) if group == &*DEV_DEPENDENCIES)) {
                    // If the dependency already exists in `dependency-groups.dev`, use that.
                    DependencyType::Group(DEV_DEPENDENCIES.clone())
                } else if existing.iter().any(|dependency_type| matches!(dependency_type, DependencyType::Dev)) {
                    // If the dependency already exists in `dev-dependencies`, use that.
                    DependencyType::Dev
                } else {
                    // Otherwise, use `dependency-groups.dev`, unless it would introduce a separate table.
                    match (self.toml.has_dev_dependencies(), self.toml.has_dependency_group(&DEV_DEPENDENCIES)) {
                        (true, false) => DependencyType::Dev,
                        (false, true) => DependencyType::Group(DEV_DEPENDENCIES.clone()),
                        (true, true) => DependencyType::Group(DEV_DEPENDENCIES.clone()),
                        (false, false) => DependencyType::Group(DEV_DEPENDENCIES.clone()),
                    }
                }
            }
            DependencyType::Group(group) if group == &*DEV_DEPENDENCIES => {
                let existing = self.toml.find_dependency(&requirement.name, None);
                if existing.iter().any(|dependency_type| matches!(dependency_type, DependencyType::Group(group) if group == &*DEV_DEPENDENCIES)) {
                    // If the dependency already exists in `dependency-groups.dev`, use that.
                    DependencyType::Group(DEV_DEPENDENCIES.clone())
                } else if existing.iter().any(|dependency_type| matches!(dependency_type, DependencyType::Dev)) {
                    // If the dependency already exists in `dev-dependencies`, use that.
                    DependencyType::Dev
                } else {
                    // Otherwise, use `dependency-groups.dev`.
                    DependencyType::Group(DEV_DEPENDENCIES.clone())
                }
            }
            DependencyType::Production => DependencyType::Production,
            DependencyType::Optional(extra) => DependencyType::Optional(extra.clone()),
            DependencyType::Group(group) => DependencyType::Group(group.clone()),
        };

        // Update the `pyproject.toml`.
        let edit = match &dependency_type {
            DependencyType::Production => {
                self.toml
                    .add_dependency(&requirement, source.as_ref(), self.raw)?
            }
            DependencyType::Dev => {
                self.toml
                    .add_dev_dependency(&requirement, source.as_ref(), self.raw)?
            }
            DependencyType::Optional(extra) => {
                self.toml
                    .add_optional_dependency(extra, &requirement, source.as_ref(), self.raw)?
            }
            DependencyType::Group(group) => self.toml.add_dependency_group_requirement(
                group,
                &requirement,
                source.as_ref(),
                self.raw,
            )?,
        };

        // If the edit was inserted before the end of the list, update the existing edits.
        if let ArrayEdit::Add(index) = &edit {
            for edit in &mut self.edits {
                if edit.dependency_type == dependency_type {
                    match &mut edit.edit {
                        ArrayEdit::Add(existing) => {
                            if *existing >= *index {
                                *existing += 1;
                            }
                        }
                        ArrayEdit::Update(existing) => {
                            if *existing >= *index {
                                *existing += 1;
                            }
                        }
                    }
                }
            }
        }

        self.edits.push(DependencyEdit {
            dependency_type,
            requirement,
            source,
            edit,
        });
        Ok(())
    }

    /// Finish the batch and return the edits at their final document positions.
    pub fn into_edits(self) -> Vec<DependencyEdit> {
        self.edits
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Result, bail};
    use uv_configuration::AddBoundsKind;
    use uv_pep440::Version;
    use uv_workspace::pyproject::DependencyType;

    use super::DependencyEditBatch;
    use crate::{ArrayEdit, DependencyTarget, PyProjectTomlMut};

    #[test]
    fn sorted_insertions_retain_bound_targets() -> Result<()> {
        let mut toml = PyProjectTomlMut::from_toml(
            "[project]\ndependencies = [\"middle\"]\n",
            DependencyTarget::PyProjectToml,
        )?;
        let mut batch =
            DependencyEditBatch::with_capacity(&mut toml, &DependencyType::Production, false, 2);
        batch.add("zebra".parse()?, None)?;
        batch.add("alpha".parse()?, None)?;
        for (edit, version) in batch.into_edits().into_iter().zip([1, 2]) {
            let index = match edit.edit {
                ArrayEdit::Add(index) => index,
                ArrayEdit::Update(_) => bail!("expected a new dependency"),
            };
            toml.set_dependency_bound(
                &edit.dependency_type,
                index,
                Version::new([version]),
                AddBoundsKind::Lower,
            )?;
        }
        insta::assert_snapshot!(toml.to_string(), @r#"
        [project]
        dependencies = [
            "alpha>=2",
            "middle",
            "zebra>=1",
        ]
        "#);
        Ok(())
    }
}
