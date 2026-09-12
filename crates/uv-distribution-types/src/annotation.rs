use std::collections::{BTreeMap, BTreeSet};

use uv_fs::Simplified;
use uv_normalize::PackageName;
use uv_pep508::RequirementOrigin;

/// Source of a dependency, e.g., a `-r requirements.txt` file.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SourceAnnotation {
    /// A `-c constraints.txt` file.
    Constraint(RequirementOrigin),
    /// An `--override overrides.txt` file.
    Override(RequirementOrigin),
    /// A `-r requirements.txt` file.
    Requirement(RequirementOrigin),
}

impl std::fmt::Display for SourceAnnotation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Requirement(origin) => match origin {
                RequirementOrigin::File(path) => {
                    write!(f, "-r {}", path.portable_display())
                }
                RequirementOrigin::Project(path, project_name) => {
                    write!(f, "{project_name} ({})", path.portable_display())
                }
                RequirementOrigin::Group(path, project_name, group) => {
                    if let Some(project_name) = project_name {
                        write!(
                            f,
                            "{project_name} ({}::dependency-groups.{group})",
                            path.portable_display()
                        )
                    } else {
                        write!(
                            f,
                            "({}::dependency-groups.{group})",
                            path.portable_display()
                        )
                    }
                }
                RequirementOrigin::Workspace => {
                    write!(f, "(workspace)")
                }
            },
            Self::Constraint(origin) => {
                write!(f, "-c {}", origin.path().portable_display())
            }
            Self::Override(origin) => match origin {
                RequirementOrigin::File(path) => {
                    write!(f, "--override {}", path.portable_display())
                }
                RequirementOrigin::Project(path, project_name) => {
                    // Project is not used for override
                    write!(f, "--override {project_name} ({})", path.portable_display())
                }
                RequirementOrigin::Group(path, project_name, group) => {
                    // Group is not used for override
                    if let Some(project_name) = project_name {
                        write!(
                            f,
                            "--override {project_name} ({}:{group})",
                            path.portable_display()
                        )
                    } else {
                        write!(f, "--override ({}:{group})", path.portable_display())
                    }
                }
                RequirementOrigin::Workspace => {
                    write!(f, "--override (workspace)")
                }
            },
        }
    }
}

/// A collection of source annotations.
#[derive(Default, Debug, Clone)]
pub struct SourceAnnotations(BTreeMap<PackageName, BTreeSet<SourceAnnotation>>);

impl SourceAnnotations {
    /// Add a source annotation to the collection for the given package.
    pub fn add(&mut self, package: &PackageName, annotation: SourceAnnotation) {
        self.0
            .entry(package.clone())
            .or_default()
            .insert(annotation);
    }

    /// Return the source annotations for a given package.
    pub fn get(&self, package: &PackageName) -> Option<&BTreeSet<SourceAnnotation>> {
        self.0.get(package)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::SourceAnnotation;
    use uv_normalize::{GroupName, PackageName};
    use uv_pep508::RequirementOrigin;

    #[test]
    fn dependency_group_annotations() {
        let project: PackageName = "My_Project".parse().expect("valid package name");
        let group: GroupName = "Docs_Test".parse().expect("valid group name");
        let path = PathBuf::from("sub dir").join("pyproject.toml");

        assert_eq!(
            SourceAnnotation::Requirement(RequirementOrigin::Group(
                path.clone(),
                Some(project),
                group.clone(),
            ))
            .to_string(),
            "my-project (sub dir/pyproject.toml::dependency-groups.docs-test)"
        );
        assert_eq!(
            SourceAnnotation::Requirement(RequirementOrigin::Group(path, None, group)).to_string(),
            "(sub dir/pyproject.toml::dependency-groups.docs-test)"
        );
    }

    #[test]
    fn other_source_annotations() {
        let project: PackageName = "my-project".parse().expect("valid package name");
        let group: GroupName = "docs".parse().expect("valid group name");
        let path = PathBuf::from("..").join("pyproject.toml");
        let cases = [
            (
                RequirementOrigin::File(path.clone()),
                "-r ../pyproject.toml",
                "-c ../pyproject.toml",
                "--override ../pyproject.toml",
            ),
            (
                RequirementOrigin::Project(path.clone(), project.clone()),
                "my-project (../pyproject.toml)",
                "-c ../pyproject.toml",
                "--override my-project (../pyproject.toml)",
            ),
            (
                RequirementOrigin::Group(path.clone(), Some(project), group.clone()),
                "my-project (../pyproject.toml::dependency-groups.docs)",
                "-c ../pyproject.toml",
                "--override my-project (../pyproject.toml:docs)",
            ),
            (
                RequirementOrigin::Group(path, None, group),
                "(../pyproject.toml::dependency-groups.docs)",
                "-c ../pyproject.toml",
                "--override (../pyproject.toml:docs)",
            ),
            (
                RequirementOrigin::Workspace,
                "(workspace)",
                "-c (workspace)",
                "--override (workspace)",
            ),
        ];

        for (origin, requirement, constraint, expected_override) in cases {
            assert_eq!(
                SourceAnnotation::Requirement(origin.clone()).to_string(),
                requirement
            );
            assert_eq!(
                SourceAnnotation::Constraint(origin.clone()).to_string(),
                constraint
            );
            assert_eq!(
                SourceAnnotation::Override(origin).to_string(),
                expected_override
            );
        }
    }
}
