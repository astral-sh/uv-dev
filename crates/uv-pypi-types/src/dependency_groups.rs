use std::collections::BTreeMap;

use uv_normalize::GroupName;

/// A PEP 735 dependency-group specifier retained as source text for contextual lowering.
pub type DependencyGroupSpecifier =
    uv_pyproject_toml::DependencyGroupSpecifier<String, BTreeMap<String, String>>;

/// PEP 735 dependency groups retained as source text for contextual lowering.
pub type DependencyGroups = uv_pyproject_toml::DependencyGroups<
    String,
    BTreeMap<String, String>,
    BTreeMap<GroupName, Vec<DependencyGroupSpecifier>>,
>;
