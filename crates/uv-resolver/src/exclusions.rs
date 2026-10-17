use std::sync::Arc;

use rustc_hash::FxHashSet;
use uv_configuration::Reinstall;

use crate::UpgradePackages;
use uv_normalize::PackageName;

/// Tracks locally installed packages that should not be selected during resolution.
#[derive(Debug, Default, Clone)]
pub struct Exclusions {
    reinstall: Reinstall,
    reinstall_packages: Option<Arc<FxHashSet<PackageName>>>,
    upgrade: UpgradePackages,
}

impl Exclusions {
    pub fn new(reinstall: Reinstall, upgrade: UpgradePackages) -> Self {
        let reinstall_packages = match &reinstall {
            Reinstall::Packages(packages, _) if packages.len() > 1 => {
                Some(Arc::new(packages.iter().cloned().collect()))
            }
            _ => None,
        };

        Self {
            reinstall,
            reinstall_packages,
            upgrade,
        }
    }

    pub(crate) fn reinstall(&self, package: &PackageName) -> bool {
        if let Some(packages) = &self.reinstall_packages {
            packages.contains(package)
        } else {
            self.reinstall.contains_package(package)
        }
    }

    pub(crate) fn upgrade(&self, package: &PackageName) -> bool {
        self.upgrade.contains(package)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::str::FromStr;

    use uv_configuration::Reinstall;
    use uv_normalize::PackageName;

    use crate::UpgradePackages;

    use super::Exclusions;

    #[test]
    fn reinstall_package_lookup() {
        let selected = PackageName::from_str("Selected_Package").expect("valid package name");
        let equivalent = PackageName::from_str("selected-package").expect("valid package name");
        let other = PackageName::from_str("other-package").expect("valid package name");
        let missing = PackageName::from_str("missing-package").expect("valid package name");

        let none = Exclusions::new(Reinstall::None, UpgradePackages::default());
        assert!(!none.reinstall(&selected));
        assert!(none.reinstall_packages.is_none());

        let all = Exclusions::new(Reinstall::All, UpgradePackages::default());
        assert!(all.reinstall(&selected));
        assert!(all.reinstall(&missing));
        assert!(all.reinstall_packages.is_none());

        let single = Exclusions::new(
            Reinstall::Packages(vec![selected.clone()], Vec::new()),
            UpgradePackages::default(),
        );
        assert!(single.reinstall(&equivalent));
        assert!(!single.reinstall(&missing));
        assert!(single.reinstall_packages.is_none());

        let multiple = Exclusions::new(
            Reinstall::Packages(
                vec![selected.clone(), other.clone(), equivalent.clone()],
                vec![Path::new("ignored-by-resolver").into()],
            ),
            UpgradePackages::default(),
        );
        assert!(multiple.reinstall(&selected));
        assert!(multiple.reinstall(&equivalent));
        assert!(multiple.reinstall(&other));
        assert!(!multiple.reinstall(&missing));
        assert!(multiple.reinstall_packages.is_some());

        let cloned = multiple.clone();
        assert!(cloned.reinstall(&selected));
        assert!(!cloned.reinstall(&missing));
    }
}
