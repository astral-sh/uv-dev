use std::collections::BTreeSet;
use std::fmt::{Debug, Formatter};

use rustc_hash::FxHashSet;
use tracing::debug;

use uv_normalize::PackageName;

/// Minimal view of a package used to apply install filters.
#[derive(Debug, Clone, Copy)]
pub struct InstallTarget<'a> {
    /// The package name.
    pub name: &'a PackageName,
    /// Whether the package refers to a local source (path, directory, editable, etc.).
    pub is_local: bool,
}

#[derive(Debug, Clone, Default)]
pub struct InstallOptions {
    /// Omit the project itself from the resolution.
    no_install_project: bool,
    /// Include only the project itself in the resolution.
    only_install_project: bool,
    /// Omit all workspace members (including the project itself) from the resolution.
    no_install_workspace: bool,
    /// Include only workspace members (including the project itself) in the resolution.
    only_install_workspace: bool,
    /// Omit all local packages from the resolution.
    no_install_local: bool,
    /// Include only local packages in the resolution.
    only_install_local: bool,
    /// Omit the specified packages from the resolution.
    no_install_package: InstallPackages,
    /// Include only the specified packages in the resolution.
    only_install_package: InstallPackages,
}

#[derive(Clone, Default)]
struct InstallPackages {
    packages: Vec<PackageName>,
    lookup: Option<FxHashSet<PackageName>>,
}

impl Debug for InstallPackages {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        self.packages.fmt(formatter)
    }
}

impl InstallPackages {
    fn new(packages: Vec<PackageName>) -> Self {
        let lookup = if packages.len() > 1 {
            Some(packages.iter().cloned().collect())
        } else {
            None
        };

        Self { packages, lookup }
    }

    fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }

    fn contains(&self, package_name: &PackageName) -> bool {
        if let Some(lookup) = &self.lookup {
            lookup.contains(package_name)
        } else {
            self.packages.contains(package_name)
        }
    }
}

impl InstallOptions {
    #[expect(clippy::fn_params_excessive_bools)]
    pub fn new(
        no_install_project: bool,
        only_install_project: bool,
        no_install_workspace: bool,
        only_install_workspace: bool,
        no_install_local: bool,
        only_install_local: bool,
        no_install_package: Vec<PackageName>,
        only_install_package: Vec<PackageName>,
    ) -> Self {
        Self {
            no_install_project,
            only_install_project,
            no_install_workspace,
            only_install_workspace,
            no_install_local,
            only_install_local,
            no_install_package: InstallPackages::new(no_install_package),
            only_install_package: InstallPackages::new(only_install_package),
        }
    }

    /// Returns `true` if a package passes the install filters.
    pub fn include_package(
        &self,
        target: InstallTarget<'_>,
        project_name: Option<&PackageName>,
        members: &BTreeSet<PackageName>,
    ) -> bool {
        let package_name = target.name;

        // If `--only-install-package` is set, only include specified packages.
        if !self.only_install_package.is_empty() {
            if self.only_install_package.contains(package_name) {
                return true;
            }
            debug!("Omitting `{package_name}` from resolution due to `--only-install-package`");
            return false;
        }

        // If `--only-install-local` is set, only include local packages.
        if self.only_install_local {
            if target.is_local {
                return true;
            }
            debug!("Omitting `{package_name}` from resolution due to `--only-install-local`");
            return false;
        }

        // If `--only-install-workspace` is set, only include the project and workspace members.
        if self.only_install_workspace {
            // Check if it's the project itself
            if let Some(project_name) = project_name
                && package_name == project_name
            {
                return true;
            }

            // Check if it's a workspace member
            if members.contains(package_name) {
                return true;
            }

            // Otherwise, exclude it
            debug!("Omitting `{package_name}` from resolution due to `--only-install-workspace`");
            return false;
        }

        // If `--only-install-project` is set, only include the project itself.
        if self.only_install_project {
            if let Some(project_name) = project_name
                && package_name == project_name
            {
                return true;
            }
            debug!("Omitting `{package_name}` from resolution due to `--only-install-project`");
            return false;
        }

        // If `--no-install-project` is set, remove the project itself.
        if self.no_install_project
            && let Some(project_name) = project_name
            && package_name == project_name
        {
            debug!("Omitting `{package_name}` from resolution due to `--no-install-project`");
            return false;
        }

        // If `--no-install-workspace` is set, remove the project and any workspace members.
        if self.no_install_workspace {
            // In some cases, the project root might be omitted from the list of workspace members
            // encoded in the lockfile. (But we already checked this above if `--no-install-project`
            // is set.)
            if !self.no_install_project
                && let Some(project_name) = project_name
                && package_name == project_name
            {
                debug!("Omitting `{package_name}` from resolution due to `--no-install-workspace`");
                return false;
            }

            if members.contains(package_name) {
                debug!("Omitting `{package_name}` from resolution due to `--no-install-workspace`");
                return false;
            }
        }

        // If `--no-install-local` is set, remove local packages.
        if self.no_install_local {
            if target.is_local {
                debug!("Omitting `{package_name}` from resolution due to `--no-install-local`");
                return false;
            }
        }

        // If `--no-install-package` is provided, remove the requested packages.
        if self.no_install_package.contains(package_name) {
            debug!("Omitting `{package_name}` from resolution due to `--no-install-package`");
            return false;
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::str::FromStr;

    use uv_normalize::PackageName;

    use super::{InstallOptions, InstallTarget};

    fn package_name(name: &str) -> PackageName {
        PackageName::from_str(name).expect("valid package name")
    }

    fn includes(
        options: &InstallOptions,
        name: &PackageName,
        is_local: bool,
        project: Option<&PackageName>,
    ) -> bool {
        options.include_package(
            InstallTarget { name, is_local },
            project,
            &BTreeSet::default(),
        )
    }

    #[test]
    fn install_package_filters() {
        let included = package_name("Included_Package");
        let excluded = package_name("excluded-package");
        let other = package_name("other-package");
        let options = InstallOptions::new(
            false,
            false,
            false,
            false,
            false,
            false,
            vec![excluded.clone(), excluded.clone()],
            Vec::new(),
        );
        assert!(includes(&options, &included, false, None));
        assert!(!includes(&options, &excluded, false, None));

        let options = InstallOptions::new(
            true,
            true,
            true,
            true,
            true,
            true,
            vec![included.clone()],
            vec![included.clone(), package_name("INCLUDED-PACKAGE")],
        );
        assert!(includes(&options, &included, false, Some(&other)));
        assert!(!includes(&options, &other, true, Some(&other)));
    }

    #[test]
    fn single_install_package_filters() {
        let selected = package_name("selected-package");
        let other = package_name("other-package");
        let options = InstallOptions::new(
            false,
            false,
            false,
            false,
            false,
            false,
            Vec::new(),
            vec![selected.clone()],
        );
        assert!(includes(&options, &selected, false, None));
        assert!(!includes(&options, &other, false, None));

        let options = InstallOptions::new(
            false,
            false,
            false,
            false,
            false,
            false,
            vec![selected.clone()],
            Vec::new(),
        );
        assert!(!includes(&options, &selected, false, None));
        assert!(includes(&options, &other, false, None));
    }

    #[test]
    fn install_package_filter_debug() {
        let package = package_name("selected-package");
        let options = InstallOptions::new(
            false,
            false,
            false,
            false,
            false,
            false,
            vec![package.clone(), package.clone()],
            vec![package.clone()],
        );

        assert_eq!(
            format!("{:?}", options.no_install_package),
            format!("{:?}", vec![package.clone(), package.clone()]),
        );
        assert_eq!(
            format!("{:?}", options.only_install_package),
            format!("{:?}", vec![package]),
        );
        assert_eq!(
            format!("{:?}", InstallOptions::default().no_install_package),
            "[]",
        );
    }
}
