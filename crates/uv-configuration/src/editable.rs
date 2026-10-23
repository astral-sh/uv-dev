use rustc_hash::FxHashSet;
use uv_normalize::PackageName;

const LINEAR_LOOKUP_LIMIT: usize = 16;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum EditableMode {
    #[default]
    Editable,
    NonEditable,
    NonEditablePackages(Vec<PackageName>),
}

impl From<bool> for EditableMode {
    fn from(value: bool) -> Self {
        if value {
            Self::Editable
        } else {
            Self::NonEditable
        }
    }
}

impl EditableMode {
    /// Determine the editable installation strategy to use for the given arguments.
    pub fn from_args(
        editable: Option<bool>,
        no_editable_package: Vec<PackageName>,
    ) -> Option<Self> {
        match editable {
            Some(editable) => Some(Self::from(editable)),
            None if no_editable_package.is_empty() => None,
            None => Some(Self::NonEditablePackages(no_editable_package)),
        }
    }

    /// Return the editable override for a specific package, if any.
    pub fn for_package(&self, package_name: &PackageName) -> Option<bool> {
        match self {
            Self::Editable => Some(true),
            Self::NonEditable => Some(false),
            Self::NonEditablePackages(packages) if packages.contains(package_name) => Some(false),
            Self::NonEditablePackages(_) => None,
        }
    }

    /// Return an editable override lookup for repeated per-package queries.
    pub fn lookup(&self) -> impl Fn(&PackageName) -> Option<bool> + '_ {
        let packages = match self {
            Self::NonEditablePackages(packages) if packages.len() > LINEAR_LOOKUP_LIMIT => {
                Some(packages.iter().collect::<FxHashSet<_>>())
            }
            Self::Editable | Self::NonEditable | Self::NonEditablePackages(_) => None,
        };

        move |package_name| {
            packages.as_ref().map_or_else(
                || self.for_package(package_name),
                |packages| packages.contains(package_name).then_some(false),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use uv_normalize::PackageName;

    use super::EditableMode;

    #[test]
    fn lookup_preserves_editable_overrides() {
        let project = PackageName::from_str("project").expect("valid package name");
        let child = PackageName::from_str("child").expect("valid package name");
        let unused = PackageName::from_str("unused").expect("valid package name");
        let mut many = (0..17)
            .map(|index| {
                PackageName::from_str(&format!("unused-{index}")).expect("valid package name")
            })
            .collect::<Vec<_>>();
        many.extend([child.clone(), child.clone()]);

        for editable in [
            EditableMode::Editable,
            EditableMode::NonEditable,
            EditableMode::NonEditablePackages(Vec::new()),
            EditableMode::NonEditablePackages(vec![child.clone()]),
            EditableMode::NonEditablePackages(vec![unused.clone(), child.clone(), child.clone()]),
            EditableMode::NonEditablePackages(many),
        ] {
            let lookup = editable.lookup();
            for package in [&project, &child, &unused] {
                assert_eq!(lookup(package), editable.for_package(package));
            }
        }
    }
}
