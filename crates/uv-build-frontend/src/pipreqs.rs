use std::str::FromStr;
use std::sync::LazyLock;

use rustc_hash::FxHashMap;
use uv_normalize::PackageName;

/// Look up the PyPI package name for a Python module.
pub(crate) fn lookup_package(module: &str) -> Option<&'static PackageName> {
    static MODULE_MAPPING: LazyLock<FxHashMap<&str, PackageName>> = LazyLock::new(|| {
        let mut mapping = FxHashMap::default();
        for line in include_str!("pipreqs/mapping").lines() {
            if let Some((module, package)) = line.split_once(':') {
                let module = module.trim();
                let package = PackageName::from_str(package.trim()).unwrap();
                mapping.insert(module, package);
            }
        }
        mapping
    });
    MODULE_MAPPING.get(module)
}
