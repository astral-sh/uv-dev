use uv_normalize::PackageName;

/// The packages to export from a workspace.
#[derive(Debug, Clone)]
pub enum ExportPackageSelection {
    /// Export all packages in the workspace.
    All,
    /// Export the root project or the selected packages.
    Selected(Vec<PackageName>),
}

impl ExportPackageSelection {
    pub fn from_args(all_packages: bool, package: Vec<PackageName>) -> Self {
        if all_packages {
            Self::All
        } else {
            Self::Selected(package)
        }
    }
}
