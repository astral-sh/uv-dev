use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::fixture::{ChildPath, FileWriteStr, PathChild};
use indoc::{formatdoc, indoc};
use insta::assert_json_snapshot;
use predicates::str::contains;
use serde::Deserialize;

use uv_test::packse::{PackseServer, scenario::Scenario};
use uv_test::{TestContext, copy_dir_ignore};

fn index() -> Result<PackseServer> {
    let scenario: Scenario = toml::from_str(indoc! {r#"
        name = "nested-workspaces"

        [root]

        [expected]
        satisfiable = true

        [packages.shared.versions."1.0.0"]
        requires_python = ">=3.11"
        sdist = false

        [packages.shared.versions."1.5.0"]
        requires_python = ">=3.11"
        sdist = false

        [packages.shared.versions."2.0.0"]
        requires_python = ">=3.11"
        sdist = false

        [packages.shared.versions."2.5.0"]
        requires_python = ">=3.11"
        sdist = false

        [packages.shared.versions."3.0.0"]
        requires_python = ">=3.11"
        sdist = false

        [packages.stable.versions."1.0.0"]
        requires_python = ">=3.11"
        sdist = false

        [packages.stable.versions."2.0.0"]
        requires_python = ">=3.11"
        sdist = false

        [packages.only-parent.versions."1.0.0"]
        requires_python = ">=3.11"
        sdist = false

        [packages.adapter.versions."1.0.0"]
        requires_python = ">=3.11"
        requires = ["shared>=2,<3"]
        sdist = false

        [packages.consumer.versions."1.0.0"]
        requires_python = ">=3.11"
        requires = ["shared<2"]
        sdist = false

        [packages.consumer.versions."2.0.0"]
        requires_python = ">=3.11"
        requires = ["shared>=2,<3"]
        sdist = false

        [packages.adapter-low.versions."1.0.0"]
        requires_python = ">=3.11"
        requires = ["shared>=1,<2"]
        sdist = false

        [packages.adapter-high.versions."1.0.0"]
        requires_python = ">=3.11"
        requires = ["shared>=2,<3"]
        sdist = false
    "#})?;
    Ok(PackseServer::from_scenario(&scenario))
}

fn write_workspace(
    root: &ChildPath,
    name: &str,
    dependencies: &[&str],
    workspaces: &[&str],
) -> Result<()> {
    root.child("pyproject.toml").write_str(&formatdoc! {r#"
        [project]
        name = "{name}"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = {dependencies:?}

        [tool.uv.workspace]
        members = []
        workspaces = {workspaces:?}
    "#})?;
    Ok(())
}

fn lock(context: &TestContext, root: &Path, server: &PackseServer) -> Command {
    let mut command = context.lock();
    command
        .current_dir(root)
        .args(["--preview-features", "nested-workspaces", "--default-index"])
        .arg(server.index_url());
    command
}

#[derive(Deserialize)]
struct LockedPackages {
    package: Vec<LockedPackage>,
}

#[derive(Deserialize)]
struct LockedPackage {
    name: String,
    version: Option<String>,
    source: BTreeMap<String, toml::Value>,
}

/// Project only registry versions, retaining multiple versions in universal locks.
fn registry_versions(root: &Path) -> Result<BTreeMap<String, Vec<String>>> {
    let lock: LockedPackages = toml::from_str(&fs_err::read_to_string(root.join("uv.lock"))?)?;
    let mut versions = BTreeMap::<String, Vec<String>>::new();
    for package in lock.package {
        if package.source.contains_key("registry")
            && let Some(version) = package.version
        {
            versions.entry(package.name).or_default().push(version);
        }
    }
    for versions in versions.values_mut() {
        versions.sort_unstable();
        versions.dedup();
    }
    Ok(versions)
}

#[test]
fn nested_workspaces_prefer_parent_and_backtrack() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = index()?;
    let parent = context.temp_dir.child("parent");
    let compatible = parent.child("services/compatible");
    let incompatible = parent.child("services/incompatible");
    let backtracking = parent.child("services/backtracking");

    write_workspace(
        &parent,
        "parent",
        &[
            "shared==1.0.0",
            "stable==1.0.0",
            "consumer==1.0.0",
            "only-parent",
        ],
        &["services/*"],
    )?;
    write_workspace(&compatible, "compatible", &["shared>=1", "stable>=1"], &[])?;
    write_workspace(
        &incompatible,
        "incompatible",
        &["adapter", "stable>=1"],
        &[],
    )?;
    write_workspace(
        &backtracking,
        "backtracking",
        &["consumer>=1", "shared>=2,<3", "stable>=1"],
        &[],
    )?;

    lock(&context, parent.path(), &server).assert().success();
    let parent_lock = fs_err::read(parent.join("uv.lock"))?;

    lock(&context, compatible.path(), &server)
        .assert()
        .success();
    assert_json_snapshot!(registry_versions(compatible.path())?, @r#"
    {
      "shared": [
        "1.0.0"
      ],
      "stable": [
        "1.0.0"
      ]
    }
    "#);

    // A transitive requirement rejects the parent's `shared` pin inside the same solve.
    lock(&context, incompatible.path(), &server)
        .assert()
        .success();
    assert_json_snapshot!(registry_versions(incompatible.path())?, @r#"
    {
      "adapter": [
        "1.0.0"
      ],
      "shared": [
        "2.5.0"
      ],
      "stable": [
        "1.0.0"
      ]
    }
    "#);

    // The inherited `consumer` candidate is in range, but its dependencies cannot be satisfied.
    lock(&context, backtracking.path(), &server)
        .assert()
        .success();
    assert_json_snapshot!(registry_versions(backtracking.path())?, @r#"
    {
      "consumer": [
        "2.0.0"
      ],
      "shared": [
        "2.5.0"
      ],
      "stable": [
        "1.0.0"
      ]
    }
    "#);
    assert_eq!(fs_err::read(parent.join("uv.lock"))?, parent_lock);

    context
        .sync()
        .current_dir(incompatible.path())
        .args([
            "--locked",
            "--preview-features",
            "nested-workspaces",
            "--default-index",
        ])
        .arg(server.index_url())
        .assert()
        .success();
    assert!(incompatible.join(".venv").is_dir());
    assert!(!parent.join(".venv").exists());
    context
        .run()
        .current_dir(incompatible.path())
        .args([
            "--no-sync",
            "python",
            "-c",
            "import importlib.util, shared; assert shared.__version__ == '2.5.0'; assert importlib.util.find_spec('only_parent') is None",
        ])
        .assert()
        .success();
    Ok(())
}

#[test]
fn nested_workspaces_preserve_child_locks() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = index()?;
    let parent = context.temp_dir.child("parent");
    let child = parent.child("services/child");

    write_workspace(&parent, "parent", &["shared==1.0.0"], &["services/*"])?;
    write_workspace(&child, "child", &["shared==2.0.0"], &[])?;
    lock(&context, parent.path(), &server).assert().success();
    lock(&context, child.path(), &server).assert().success();

    write_workspace(&child, "child", &["shared>=1"], &[])?;
    lock(&context, child.path(), &server).assert().success();
    assert_json_snapshot!(registry_versions(child.path())?, @r#"
    {
      "shared": [
        "2.0.0"
      ]
    }
    "#);

    // Moving a complete child outside the collection makes it an ordinary workspace.
    let standalone = context.temp_dir.child("standalone");
    copy_dir_ignore(child.path(), standalone.path())?;
    lock(&context, standalone.path(), &server)
        .arg("--upgrade")
        .assert()
        .success();
    assert_json_snapshot!(registry_versions(standalone.path())?, @r#"
    {
      "shared": [
        "3.0.0"
      ]
    }
    "#);

    write_workspace(&parent, "parent", &["shared==1.5.0"], &["services/*"])?;
    lock(&context, parent.path(), &server).assert().success();
    let parent_lock = fs_err::read_to_string(parent.join("uv.lock"))?;
    let child_lock = fs_err::read(child.join("uv.lock"))?;

    lock(&context, child.path(), &server)
        .arg("--locked")
        .assert()
        .success();
    parent.child("uv.lock").write_str("not a lockfile")?;
    lock(&context, child.path(), &server)
        .arg("--locked")
        .assert()
        .success();
    fs_err::remove_file(parent.join("uv.lock"))?;
    for flag in ["--locked", "--frozen"] {
        lock(&context, child.path(), &server)
            .arg(flag)
            .assert()
            .success();
    }
    assert_eq!(fs_err::read(child.join("uv.lock"))?, child_lock);

    // The ordinary upgrade flags drop child pins, not the parent's baseline.
    parent.child("uv.lock").write_str(&parent_lock)?;
    lock(&context, child.path(), &server)
        .arg("--upgrade")
        .assert()
        .success();
    assert_json_snapshot!(registry_versions(child.path())?, @r#"
    {
      "shared": [
        "1.5.0"
      ]
    }
    "#);
    let upgraded_lock = fs_err::read(child.join("uv.lock"))?;
    lock(&context, child.path(), &server)
        .args(["--upgrade-package", "shared"])
        .assert()
        .success();
    assert_eq!(fs_err::read(child.join("uv.lock"))?, upgraded_lock);
    assert_eq!(fs_err::read_to_string(parent.join("uv.lock"))?, parent_lock);
    Ok(())
}

#[test]
fn nested_workspaces_require_parent_lock_for_resolution() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = index()?;
    let parent = context.temp_dir.child("parent");
    let child = parent.child("services/child");

    write_workspace(&parent, "parent", &["shared==1.0.0"], &["services/*"])?;
    write_workspace(&child, "child", &["shared>=1"], &[])?;
    lock(&context, child.path(), &server)
        .assert()
        .code(1)
        .stderr(contains("Unable to find the parent workspace lockfile"))
        .stderr(contains("Run `uv lock` in the parent workspace first"));
    assert!(!parent.join("uv.lock").exists());
    assert!(!child.join("uv.lock").exists());

    parent.child("uv.lock").write_str("not a lockfile")?;
    lock(&context, child.path(), &server)
        .assert()
        .code(1)
        .stderr(contains("Failed to parse the parent workspace lockfile"))
        .stderr(contains(parent.join("uv.lock").display().to_string()));
    assert_eq!(
        fs_err::read_to_string(parent.join("uv.lock"))?,
        "not a lockfile"
    );
    assert!(!child.join("uv.lock").exists());
    Ok(())
}

#[test]
fn nested_workspaces_choose_nearest_parent() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = index()?;
    let parent = context.temp_dir.child("parent");
    let child = parent.child("services/child");
    let grandchild = child.child("plugins/grandchild");

    write_workspace(
        &parent,
        "parent",
        &["shared==1.0.0"],
        &["services/*", "services/*/plugins/*"],
    )?;
    write_workspace(&child, "child", &["shared==2.0.0"], &["plugins/*"])?;
    write_workspace(&grandchild, "grandchild", &["shared>=1"], &[])?;
    lock(&context, parent.path(), &server).assert().success();
    lock(&context, child.path(), &server).assert().success();
    let parent_lock = fs_err::read(parent.join("uv.lock"))?;
    let child_lock = fs_err::read(child.join("uv.lock"))?;

    lock(&context, grandchild.path(), &server)
        .assert()
        .success();
    assert_json_snapshot!(registry_versions(grandchild.path())?, @r#"
    {
      "shared": [
        "2.0.0"
      ]
    }
    "#);
    assert_eq!(fs_err::read(parent.join("uv.lock"))?, parent_lock);
    assert_eq!(fs_err::read(child.join("uv.lock"))?, child_lock);
    Ok(())
}

#[test]
fn nested_workspaces_keep_resolver_settings() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let parent_index = index()?;
    let child_index = index()?;
    let parent = context.temp_dir.child("parent");
    let child = parent.child("services/child");

    parent.child("pyproject.toml").write_str(&formatdoc! {r#"
        [project]
        name = "parent"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["shared==1.0.0", "stable"]

        [tool.uv]
        constraint-dependencies = ["stable==1.0.0"]

        [tool.uv.workspace]
        members = []
        workspaces = ["services/*"]

        [[tool.uv.index]]
        name = "parent"
        url = "{}"
        default = true
    "#, parent_index.index_url()})?;
    child.child("pyproject.toml").write_str(&formatdoc! {r#"
        [project]
        name = "child"
        version = "0.1.0"
        requires-python = ">=3.11"
        dependencies = ["shared>=1", "stable"]

        [tool.uv]
        constraint-dependencies = ["stable==2.0.0"]

        [tool.uv.workspace]
        members = []

        [[tool.uv.index]]
        name = "child"
        url = "{}"
        default = true
    "#, child_index.index_url()})?;

    for root in [parent.path(), child.path()] {
        context
            .lock()
            .current_dir(root)
            .args(["--preview-features", "nested-workspaces"])
            .assert()
            .success();
    }
    assert_json_snapshot!(registry_versions(parent.path())?, @r#"
    {
      "shared": [
        "1.0.0"
      ],
      "stable": [
        "1.0.0"
      ]
    }
    "#);
    assert_json_snapshot!(registry_versions(child.path())?, @r#"
    {
      "shared": [
        "3.0.0"
      ],
      "stable": [
        "2.0.0"
      ]
    }
    "#);
    let child_lock: toml::Value = toml::from_str(&fs_err::read_to_string(child.join("uv.lock"))?)?;
    assert_eq!(child_lock["requires-python"].as_str(), Some(">=3.11"));
    Ok(())
}

#[test]
fn nested_workspaces_ignore_disjoint_parent_environment() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = index()?;
    let parent = context.temp_dir.child("parent");
    let child = parent.child("services/child");

    parent.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "parent"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["shared==1.0.0"]

        [tool.uv]
        environments = ["sys_platform == 'win32'"]

        [tool.uv.workspace]
        members = []
        workspaces = ["services/*"]
    "#})?;
    child.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "child"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["shared>=1"]

        [tool.uv]
        environments = ["sys_platform == 'linux'"]

        [tool.uv.workspace]
        members = []
    "#})?;

    lock(&context, parent.path(), &server).assert().success();
    lock(&context, child.path(), &server).assert().success();
    assert_json_snapshot!(registry_versions(child.path())?, @r#"
    {
      "shared": [
        "3.0.0"
      ]
    }
    "#);
    Ok(())
}

#[test]
fn nested_workspaces_scope_inherited_fork_preferences() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = index()?;
    let parent = context.temp_dir.child("parent");
    let child = parent.child("services/child");

    write_workspace(
        &parent,
        "parent",
        &[
            "shared==1.0.0; python_version < '3.13'",
            "shared==2.0.0; python_version >= '3.13'",
        ],
        &["services/*"],
    )?;
    write_workspace(
        &child,
        "child",
        &[
            "adapter-low; python_version < '3.13'",
            "adapter-high; python_version >= '3.13'",
        ],
        &[],
    )?;
    lock(&context, parent.path(), &server).assert().success();
    lock(&context, child.path(), &server).assert().success();
    assert_json_snapshot!(registry_versions(child.path())?, @r#"
    {
      "adapter-high": [
        "1.0.0"
      ],
      "adapter-low": [
        "1.0.0"
      ],
      "shared": [
        "1.0.0",
        "2.0.0"
      ]
    }
    "#);
    Ok(())
}
