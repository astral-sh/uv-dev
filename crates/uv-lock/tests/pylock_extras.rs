use std::path::Path;

use uv_configuration::{DependencyGroups, ExtrasSpecification, InstallOptions};
use uv_lock::{Installable, Lock, PylockToml, PylockTomlErrorKind};
use uv_normalize::{DefaultExtras, DefaultGroups, PackageName};

struct Project<'a> {
    lock: &'a Lock,
    name: PackageName,
}

impl<'a> Installable<'a> for Project<'a> {
    fn install_path(&self) -> &'a Path {
        Path::new(".")
    }

    fn lock(&self) -> &'a Lock {
        self.lock
    }

    fn roots(&self) -> impl Iterator<Item = &PackageName> {
        std::iter::once(&self.name)
    }

    fn project_name(&self) -> Option<&PackageName> {
        Some(&self.name)
    }
}

const LOCK: &str = r#"
version = 1
revision = 5
requires-python = ">=3.12"

[[package]]
name = "project"
version = "1.0.0"
source = { editable = "." }
dependencies = [{ name = "dependency" }, { name = "conditional", marker = "extra == 'empty' or (extra == 'fast' and extra != 'slow')" }]
[package.optional-dependencies]
fast = [{ name = "helper", marker = "sys_platform == 'linux'" }]
slow = [{ name = "helper", marker = "sys_platform == 'win32'" }]
[package.dev-dependencies]
dev = [{ name = "helper" }]
empty-group = []
[package.metadata]
requires-dist = [
    { name = "dependency" },
    { name = "conditional", marker = "extra == 'empty' or (extra == 'fast' and extra != 'slow')" },
    { name = "helper", marker = "extra == 'fast' and sys_platform == 'linux'" },
    { name = "helper", marker = "extra == 'slow' and sys_platform == 'win32'" },
]
provides-extras = ["empty", "fast", "slow"]
[package.metadata.requires-dev]
dev = [{ name = "helper" }]
empty-group = []

[[package]]
name = "conditional"
version = "1.0.0"
source = { registry = "https://pypi.org/simple" }
wheels = [{ url = "https://files.pythonhosted.org/packages/conditional-1.0.0-py3-none-any.whl", hash = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef" }]

[[package]]
name = "dependency"
version = "1.0.0"
source = { registry = "https://pypi.org/simple" }
wheels = [{ url = "https://files.pythonhosted.org/packages/dependency-1.0.0-py3-none-any.whl", hash = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef" }]
[package.optional-dependencies]
feature = [{ name = "leaf" }]

[[package]]
name = "helper"
version = "1.0.0"
source = { registry = "https://pypi.org/simple" }
dependencies = [{ name = "dependency", extra = ["feature"] }]
wheels = [{ url = "https://files.pythonhosted.org/packages/helper-1.0.0-py3-none-any.whl", hash = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef" }]

[[package]]
name = "leaf"
version = "1.0.0"
source = { registry = "https://pypi.org/simple" }
wheels = [{ url = "https://files.pythonhosted.org/packages/leaf-1.0.0-py3-none-any.whl", hash = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef" }]

[[package]]
name = "unrelated"
version = "1.0.0"
source = { virtual = "unrelated" }
dependencies = [{ name = "dependency", marker = "extra != 'unrelated'" }]
"#;

fn export_project(lock: &Lock) -> Result<PylockToml, PylockTomlErrorKind> {
    let project = Project {
        lock,
        name: "project".parse()?,
    };
    let install_options =
        InstallOptions::new(true, false, false, false, false, false, vec![], vec![]);
    let groups = DependencyGroups::default().with_defaults(DefaultGroups::default());
    PylockToml::from_lock_with_selection_markers(
        &project,
        Path::new("."),
        &[],
        &groups,
        None,
        &install_options,
    )
}

/// Negative extra markers outside the selected project's dependency graph do not affect its export.
#[test]
fn selectable_extras_and_groups() -> Result<(), Box<dyn std::error::Error>> {
    let lock = Lock::from_toml(LOCK)?;
    let contents = export_project(&lock)?.to_toml()?;
    insta::assert_snapshot!(contents, @r#"
    lock-version = "1.0"
    created-by = "uv"
    requires-python = ">=3.12"
    extras = [
        "empty",
        "fast",
        "slow",
    ]
    dependency-groups = [
        "dev",
        "empty-group",
    ]

    [[packages]]
    name = "conditional"
    version = "1.0.0"
    marker = "'empty' in extras or ('fast' in extras and 'slow' not in extras)"
    index = "https://pypi.org/simple"
    wheels = [{ url = "https://files.pythonhosted.org/packages/conditional-1.0.0-py3-none-any.whl", hashes = { sha256 = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef" } }]

    [[packages]]
    name = "dependency"
    version = "1.0.0"
    index = "https://pypi.org/simple"
    wheels = [{ url = "https://files.pythonhosted.org/packages/dependency-1.0.0-py3-none-any.whl", hashes = { sha256 = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef" } }]

    [[packages]]
    name = "helper"
    version = "1.0.0"
    marker = "(sys_platform == 'linux' and 'fast' in extras) or (sys_platform == 'win32' and 'slow' in extras) or 'dev' in dependency_groups"
    index = "https://pypi.org/simple"
    wheels = [{ url = "https://files.pythonhosted.org/packages/helper-1.0.0-py3-none-any.whl", hashes = { sha256 = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef" } }]

    [[packages]]
    name = "leaf"
    version = "1.0.0"
    marker = "(sys_platform == 'linux' and 'fast' in extras) or (sys_platform == 'win32' and 'slow' in extras) or 'dev' in dependency_groups"
    index = "https://pypi.org/simple"
    wheels = [{ url = "https://files.pythonhosted.org/packages/leaf-1.0.0-py3-none-any.whl", hashes = { sha256 = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef" } }]
    "#);

    Ok(())
}

/// An empty multi-use selection list is represented explicitly in either TOML writer.
#[test]
fn empty_selection_fields() -> Result<(), Box<dyn std::error::Error>> {
    let lock = Lock::from_toml(indoc::indoc! {r#"
        version = 1
        revision = 5
        requires-python = ">=3.12"

        [[package]]
        name = "project"
        version = "1.0.0"
        source = { virtual = "." }
    "#})?;
    let project = Project {
        lock: &lock,
        name: "project".parse()?,
    };
    let extras = ExtrasSpecification::default().with_defaults(DefaultExtras::default());
    let groups = DependencyGroups::default().with_defaults(DefaultGroups::default());
    let install_options = InstallOptions::default();

    let flat = PylockToml::from_lock(
        &project,
        Path::new("."),
        &[],
        &extras,
        &groups,
        false,
        None,
        &install_options,
    )?;
    let flat_contents = flat.to_toml()?;
    insta::assert_snapshot!(flat_contents, @r#"
    lock-version = "1.0"
    created-by = "uv"
    requires-python = ">=3.12"
    packages = []
    "#);
    assert_eq!(
        flat_contents.parse::<toml::Table>()?,
        toml::to_string(&flat)?.parse::<toml::Table>()?,
    );
    let parsed: PylockToml = toml::from_str(&flat_contents)?;
    assert_eq!(parsed.to_toml()?, flat_contents);

    let multi_use = export_project(&lock)?;
    let multi_use_contents = multi_use.to_toml()?;
    insta::assert_snapshot!(multi_use_contents, @r#"
    lock-version = "1.0"
    created-by = "uv"
    requires-python = ">=3.12"
    extras = []
    dependency-groups = []
    packages = []
    "#);
    assert_eq!(
        multi_use_contents.parse::<toml::Table>()?,
        toml::to_string(&multi_use)?.parse::<toml::Table>()?,
    );
    let parsed: PylockToml = toml::from_str(&multi_use_contents)?;
    assert_eq!(parsed.to_toml()?, multi_use_contents);

    Ok(())
}
