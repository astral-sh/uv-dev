use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::fixture::{FileTouch, FileWriteStr, PathChild};
use indoc::{formatdoc, indoc};
use uv_test::packse::PackseServer;
use uv_test::packse::scenario::Scenario;
use uv_test::uv_snapshot;

#[test]
fn explicit_roots_metadata_sync_intersects_python() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"]);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv.workspace]
        members = ["root-a", "root-b"]
        roots = ["root-a", "root-b"]
    "#})?;
    context
        .temp_dir
        .child("root-a/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "root-a"
        version = "0.1.0"
        requires-python = ">=3.12"

        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("root-b/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "root-b"
        version = "0.1.0"
        requires-python = ">=3.13"

        [tool.uv]
        package = false
    "#})?;
    context
        .workspace_metadata()
        .args([
            "--sync",
            "--offline",
            "--no-index",
            "--preview-features",
            "workspace-metadata",
        ])
        .assert()
        .success();
    uv_snapshot!(context.python_command().arg("-c").arg("import sys; print(sys.version_info[:2])"), @"
    exit_code: 0 (success)
    ----- stdout -----
    (3, 13)
    ");
    Ok(())
}

#[test]
fn explicit_roots_frozen_python_uses_locked_domain() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"]);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv.workspace]
        members = ["root-a", "root-b"]
        roots = ["root-a", "root-b"]
    "#})?;
    context
        .temp_dir
        .child("root-a/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "root-a"
        version = "0.1.0"
        requires-python = ">=3.12"

        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("root-b/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "root-b"
        version = "0.1.0"
        requires-python = ">=3.13"

        [tool.uv]
        package = false
    "#})?;
    context
        .lock()
        .args(["--offline", "--no-index", "--python", "3.12"])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.sync().args(["--frozen", "--offline", "--package", "root-b"]), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.13.[X] interpreter at: [PYTHON-3.13]
    Creating virtual environment at: .venv
    Checked in [TIME]
    "#);
    uv_snapshot!(context.python_command().arg("-c").arg("import sys; print(sys.version_info[:2])"), @"
    exit_code: 0 (success)
    ----- stdout -----
    (3, 13)
    ");

    context
        .venv()
        .args(["--clear", "--python", "3.12"])
        .assert()
        .success();
    fs_err::remove_file(context.temp_dir.child("pyproject.toml"))?;
    fs_err::remove_file(context.temp_dir.child("root-a/pyproject.toml"))?;
    fs_err::remove_file(context.temp_dir.child("root-b/pyproject.toml"))?;
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--offline", "--package", "root-b", "--preview-features", "frozen-lockfile",
    ]), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.13.[X] interpreter at: [PYTHON-3.13]
    Removed virtual environment at: .venv
    Creating virtual environment at: .venv
    Checked in [TIME]
    "#);
    uv_snapshot!(context.python_command().arg("-c").arg("import sys; print(sys.version_info[:2])"), @"
    exit_code: 0 (success)
    ----- stdout -----
    (3, 13)
    ");
    Ok(())
}

#[test]
fn explicit_roots_unconstrained_python_domain() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv.workspace]
        members = ["root-a", "root-b"]
        roots = ["root-a", "root-b"]
    "#})?;
    context
        .temp_dir
        .child("root-a/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "root-a"
        version = "0.1.0"
        requires-python = ">=3.13"

        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("root-b/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "root-b"
        version = "0.1.0"

        [tool.uv]
        package = false
    "#})?;
    uv_snapshot!(context.filters(), context.sync().args([
        "--offline", "--no-index", "--package", "root-b", "--python", "3.12",
    ]), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    warning: The workspace `requires-python` value (``) does not contain a lower bound. Add a lower bound to indicate the minimum compatible Python version (e.g., `>=3.12`).
    Resolved 2 packages in [TIME]
    Checked in [TIME]
    "#);
    let lock: toml::Value = toml::from_str(&context.read("uv.lock"))?;
    insta::assert_json_snapshot!(lock["requires-python"], @r#""""#);
    Ok(())
}

#[test]
fn explicit_roots_inherit_requested_group_python() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"]);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "root-a"
        version = "0.1.0"
        requires-python = ">=3.12"

        [dependency-groups]
        docs = []

        [tool.uv]
        package = false
        default-groups = []

        [tool.uv.dependency-groups]
        docs = { requires-python = ">=3.13" }

        [tool.uv.workspace]
        members = ["root-b"]
        roots = ["root-a", "root-b"]
    "#})?;
    let root_b = context.temp_dir.child("root-b");
    context
        .temp_dir
        .child("root-b/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "root-b"
        version = "0.1.0"
        requires-python = ">=3.12"

        [tool.uv]
        package = false
    "#})?;
    uv_snapshot!(context.filters(), context.sync().args([
        "--offline", "--no-index", "--package", "root-b", "--group", "docs", "--python", "3.12",
    ]), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    error: The requested interpreter resolved to Python 3.12.[X], which is incompatible with the project's Python requirement: `>=3.13` (from workspace member `root-a`'s `tool.uv.dependency-groups.docs.requires-python`).
    "#);
    uv_snapshot!(context.filters(), context.sync().args([
        "--offline", "--no-index", "--package", "root-b", "--group", "docs", "--python", "3.13",
    ]), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.13.[X] interpreter at: [PYTHON-3.13]
    Creating virtual environment at: .venv
    Resolved 2 packages in [TIME]
    Checked in [TIME]
    "#);

    let member_manifest = fs_err::read_to_string(root_b.child("pyproject.toml"))?;
    root_b
        .child("pyproject.toml")
        .write_str(&(member_manifest + "\n[dependency-groups]\ndocs = []\n"))?;
    uv_snapshot!(context.filters(), context.sync().args([
        "--offline", "--no-index", "--package", "root-b", "--group", "docs", "--python", "3.12",
    ]), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Removed virtual environment at: .venv
    Creating virtual environment at: .venv
    Resolved 2 packages in [TIME]
    Checked in [TIME]
    "#);
    Ok(())
}

#[test]
fn explicit_roots_membership_filters_and_freshness() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["shared"]

        [tool.uv]
        package = false

        [tool.uv.sources]
        shared = { workspace = true }

        [tool.uv.workspace]
        members = ["shared", "unused"]
        roots = ["app"]
    "#})?;
    context
        .temp_dir
        .child("shared/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "shared"
        version = "0.1.0"
        requires-python = ">=3.12"

        [build-system]
        requires = []
        build-backend = "uv_build"
    "#})?;
    context
        .temp_dir
        .child("shared/src/shared/__init__.py")
        .touch()?;
    context
        .temp_dir
        .child("unused/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "unused"
        version = "0.1.0"
        requires-python = ">=3.12"

        [tool.uv]
        package = false
    "#})?;
    context
        .lock()
        .args(["--offline", "--no-index"])
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--offline", "--no-header", "--no-hashes", "--no-emit-workspace",
    ]), @"exit_code: 0 (success)");
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--offline", "--no-header", "--no-hashes", "--only-emit-workspace",
    ]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    -e ./shared
        # via app
    "#);
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--offline", "--dry-run", "--no-install-workspace",
    ]), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Would use project environment at: .venv
    Checked in [TIME]
    Would make no changes
    "#);
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--offline", "--dry-run", "--only-install-workspace",
    ]), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Would use project environment at: .venv
    Would download 1 package
    Would install 1 package
     + shared @ file://[TEMP_DIR]/shared
    "#);

    let manifest = context.read("pyproject.toml").replace(
        "members = [\"shared\", \"unused\"]",
        "members = [\"shared\", \"unused\", \"new\"]",
    );
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&manifest)?;
    context
        .temp_dir
        .child("new/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "new"
        version = "0.1.0"
        requires-python = ">=3.12"

        [tool.uv]
        package = false
    "#})?;
    uv_snapshot!(context.filters(), context.lock().args(["--offline", "--no-index", "--locked"]), @r#"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

    hint: To update the lockfile, run `uv lock`.
    "#);

    // Older lockfiles use the member set for both roots and membership.
    let mut legacy: toml::Value = toml::from_str(&context.read("uv.lock"))?;
    let legacy_manifest = legacy["manifest"]
        .as_table_mut()
        .expect("lockfile manifest");
    legacy_manifest.remove("workspace-members");
    legacy_manifest.insert(
        "members".to_string(),
        toml::Value::try_from(["app", "shared"])?,
    );
    context
        .temp_dir
        .child("uv.lock")
        .write_str(&toml::to_string(&legacy)?)?;
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--offline", "--no-header", "--no-hashes", "--no-emit-workspace",
    ]), @"exit_code: 0 (success)");
    Ok(())
}

#[test]
fn explicit_roots_removed_refreshes_workspace_membership() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv.workspace]
        members = ["app", "shared"]
        roots = ["app"]

        [tool.uv.sources]
        shared = { workspace = true }
    "#})?;
    context
        .temp_dir
        .child("app/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["shared"]

        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("shared/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "shared"
        version = "0.1.0"
        requires-python = ">=3.12"

        [build-system]
        requires = []
        build-backend = "uv_build"
    "#})?;
    context
        .temp_dir
        .child("shared/src/shared/__init__.py")
        .touch()?;
    context
        .lock()
        .args(["--offline", "--no-index"])
        .assert()
        .success();
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv.workspace]
        members = ["app"]

        [tool.uv.sources]
        shared = { path = "shared", editable = true }
    "#})?;
    uv_snapshot!(context.filters(), context.lock().args([
        "--offline", "--no-index", "--locked",
    ]), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

    hint: To update the lockfile, run `uv lock`.
    ");
    context
        .lock()
        .args(["--offline", "--no-index"])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.sync().args([
        "--offline", "--dry-run", "--no-install-workspace",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Would use project environment at: .venv
    Resolved 2 packages in [TIME]
    Found up-to-date lockfile at: uv.lock
    Would download 1 package
    Would install 1 package
     + shared @ file://[TEMP_DIR]/shared
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--offline", "--no-header", "--no-hashes", "--no-emit-workspace",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    -e ./shared
        # via app
    ");
    Ok(())
}

#[test]
fn explicit_roots_reuse_lock_with_omitted_group_metadata() -> Result<()> {
    let server = PackseServer::from_scenario(&toml::from_str(indoc! {r#"
        name = "explicit-roots-freshness"
        [root]
        [expected]
        satisfiable = true
        [packages.dependency.versions."1.0.0"]
        sdist = false
    "#})?);
    let context = uv_test::test_context!("3.12");
    let index = server.index_url();
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["dependency"]
        [tool.uv]
        package = false
        [tool.uv.workspace]
        members = ["unused"]
        roots = ["app"]
        [[tool.uv.index]]
        url = "{index}"
        default = true
    "#})?;
    context
        .temp_dir
        .child("unused/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "unused"
        version = "0.1.0"
        requires-python = ">=3.12"
        [dependency-groups]
        docs = []
        [tool.uv]
        package = false
        default-groups = []
        [tool.uv.dependency-groups]
        docs = { requires-python = ">=3.13" }
    "#})?;
    context.lock().assert().success();
    let locked = context.read("uv.lock");
    // Resolving the registry dependency is impossible without the index or its cache.
    uv_snapshot!(context.filters(), context.lock().args(["--locked", "--offline", "--no-cache"]), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    "#);
    assert_eq!(locked, context.read("uv.lock"));

    let unused = context
        .read("unused/pyproject.toml")
        .replace(">=3.12", ">=3.14");
    context
        .temp_dir
        .child("unused/pyproject.toml")
        .write_str(&unused)?;
    uv_snapshot!(context.filters(), context.lock().args(["--locked", "--offline", "--no-cache"]), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    "#);
    assert_eq!(locked, context.read("uv.lock"));
    Ok(())
}

#[test]
fn explicit_roots_validate_non_root_python_requirement() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["shared"]
        [tool.uv]
        package = false
        [tool.uv.sources]
        shared = { workspace = true }
        [tool.uv.workspace]
        members = ["shared"]
        roots = ["app"]
    "#})?;
    context
        .temp_dir
        .child("shared/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "shared"
        version = "0.1.0"
        requires-python = ">=3.12"

        [tool.uv]
        package = false
    "#})?;
    context
        .lock()
        .args(["--offline", "--no-index"])
        .assert()
        .success();
    let locked = context.read("uv.lock");
    context
        .temp_dir
        .child("shared/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "shared"
        version = "0.1.0"
        requires-python = ">=3.13"

        [tool.uv]
        package = false
    "#})?;
    uv_snapshot!(context.filters(), context.lock().args(["--locked", "--offline", "--no-index"]), @r#"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because the requested Python version (>=3.12) does not satisfy Python>=3.13 and shared depends on Python>=3.13, we can conclude that shared's requirements are unsatisfiable.
             And because app depends on shared, we can conclude that app's requirements are unsatisfiable.
             And because only app==0.1.0 is available and your workspace requires app, we can conclude that your workspace's requirements are unsatisfiable.

    hint: The `requires-python` value (>=3.12) includes Python versions that are not supported by your dependencies (e.g., shared==0.1.0 only supports >=3.13). Consider using a more restrictive `requires-python` value (like >=3.13).
    "#);
    assert_eq!(locked, context.read("uv.lock"));
    Ok(())
}

#[test]
fn explicit_roots_conditional_python_requirement_and_legacy_metadata() -> Result<()> {
    let server = PackseServer::from_scenario(&toml::from_str(indoc! {r#"
        name = "explicit-roots-conditional-python"
        [root]
        [expected]
        satisfiable = true
        [packages.dependency.versions."1.0.0"]
        sdist = false
    "#})?);
    let context = uv_test::test_context!("3.12");
    let index = server.index_url();
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["dependency", "shared; python_version >= '3.13'"]
        [tool.uv]
        package = false
        [tool.uv.sources]
        shared = {{ workspace = true }}
        [tool.uv.workspace]
        members = ["shared"]
        roots = ["app"]
        [[tool.uv.index]]
        url = "{index}"
        default = true
    "#})?;
    context
        .temp_dir
        .child("shared/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "shared"
        version = "0.1.0"
        requires-python = ">=3.13"

        [tool.uv]
        package = false
    "#})?;
    context.lock().assert().success();
    let locked = context.read("uv.lock");
    uv_snapshot!(context.filters(), context.lock().args(["--locked", "--offline", "--no-cache"]), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    "#);
    assert_eq!(locked, context.read("uv.lock"));

    let mut legacy: toml::Value = toml::from_str(&locked)?;
    let shared = legacy["package"]
        .as_array_mut()
        .expect("packages")
        .iter_mut()
        .find(|package| package["name"].as_str() == Some("shared"))
        .expect("shared member");
    shared["metadata"]
        .as_table_mut()
        .expect("shared metadata")
        .remove("requires-python");
    context
        .temp_dir
        .child("uv.lock")
        .write_str(&toml::to_string(&legacy)?)?;
    // A legacy lock needs one refresh to record the non-root declaration.
    context.lock().assert().success();
    assert_eq!(locked, context.read("uv.lock"));
    uv_snapshot!(context.filters(), context.lock().args(["--locked", "--offline", "--no-cache"]), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    "#);

    context
        .temp_dir
        .child("shared/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "shared"
        version = "0.1.0"
        requires-python = ">=3.14"

        [tool.uv]
        package = false
    "#})?;
    uv_snapshot!(context.filters(), context.lock().args(["--locked", "--offline"]), @r#"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies for split (markers: python_full_version >= '3.13')
      cause: Because only shared{python_full_version >= '3.13'}==0.1.0 is available and the requested Python version (>=3.12) does not satisfy Python>=3.14, we can conclude that all versions of shared{python_full_version >= '3.13'} cannot be used.
             And because app depends on shared{python_full_version >= '3.13'}, we can conclude that app's requirements are unsatisfiable.
             And because only app==0.1.0 is available and your workspace requires app, we can conclude that your workspace's requirements are unsatisfiable.

    hint: While the active Python version is 3.12, the resolution failed for other Python versions supported by your project. Consider limiting your project's supported Python versions using `requires-python`.
    "#);
    assert_eq!(locked, context.read("uv.lock"));
    Ok(())
}

#[test]
fn explicit_roots_frozen_selects_resolved_non_root_member() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["shared"]
        [tool.uv]
        package = false
        [tool.uv.sources]
        shared = { workspace = true }
        [tool.uv.workspace]
        members = ["shared", "unused"]
        roots = ["app"]
    "#})?;
    context
        .temp_dir
        .child("shared/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "shared"
        version = "0.1.0"
        requires-python = ">=3.12"
        [build-system]
        requires = []
        build-backend = "uv_build"
    "#})?;
    context
        .temp_dir
        .child("shared/src/shared/__init__.py")
        .touch()?;
    context
        .temp_dir
        .child("unused/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "unused"
        version = "0.1.0"
        requires-python = ">=3.12"

        [tool.uv]
        package = false
    "#})?;
    context
        .lock()
        .args(["--offline", "--no-index"])
        .assert()
        .success();
    fs_err::remove_file(context.temp_dir.child("pyproject.toml"))?;

    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--offline", "--package", "shared", "--no-header", "--no-hashes",
        "--preview-features", "frozen-lockfile",
    ]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    -e ./shared
    "#);
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--offline", "--dry-run", "--package", "shared",
        "--preview-features", "frozen-lockfile",
    ]), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Would use project environment at: .venv
    Would download 1 package
    Would install 1 package
     + shared @ file://[TEMP_DIR]/shared
    "#);
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--offline", "--package", "unused", "--no-header", "--no-hashes",
        "--preview-features", "frozen-lockfile",
    ]), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Package `unused` not found in lockfile workspace
    "#);
    Ok(())
}

/// Metadata-free freshness validates optional sections only for resolution roots or requested extras.
#[test]
fn explicit_roots_metadata_free_ignores_unselected_sections() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let scenario = toml::from_str::<Scenario>(indoc! {r#"
        name = "explicit-roots-metadata-free"

        [root]

        [expected]
        satisfiable = true

        [packages.leaf.versions."1"]
        sdist = false
    "#})?;
    let server = PackseServer::from_scenario(&scenario);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["shared", "leaf"]

        [tool.uv]
        package = false

        [tool.uv.sources]
        shared = { workspace = true }

        [tool.uv.workspace]
        members = ["shared"]
        roots = ["app"]
    "#})?;
    context
        .temp_dir
        .child("shared/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "shared"
        version = "0.1.0"
        requires-python = ">=3.12"

        [project.optional-dependencies]
        unused = ["missing-extra"]

        [dependency-groups]
        dev = ["missing-group"]

        [tool.uv]
        package = false
    "#})?;
    context
        .lock()
        .args(["--preview-features", "lock-without-metadata"])
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.lock()
        .args(["--locked", "--offline", "--no-cache", "--preview-features", "lock-without-metadata"])
        .arg("--index-url").arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    Ok(())
}

/// Selecting a transitive member cannot silently omit its unresolved explicit or default groups.
#[test]
fn explicit_roots_rejects_unresolved_member_groups() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["shared"]

        [tool.uv]
        package = false

        [tool.uv.sources]
        shared = { workspace = true }

        [tool.uv.workspace]
        members = ["shared"]
        roots = ["app"]
    "#})?;
    context
        .temp_dir
        .child("shared/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "shared"
        version = "0.1.0"
        requires-python = ">=3.12"

        [dependency-groups]
        dev = ["missing-group"]

        [tool.uv]
        package = false
    "#})?;
    context
        .lock()
        .args(["--offline", "--no-index"])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--package", "shared", "--group", "dev", "--no-install-workspace",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Group `dev` for workspace member `shared` was not resolved

    hint: Add `shared` to `tool.uv.workspace.roots` and run `uv lock` to resolve its dependency groups.
    ");
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--package", "shared", "--no-install-workspace",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Group `dev` for workspace member `shared` was not resolved

    hint: Add `shared` to `tool.uv.workspace.roots` and run `uv lock` to resolve its dependency groups.
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--package", "shared", "--group", "dev", "--no-header",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Group `dev` for workspace member `shared` was not resolved

    hint: Add `shared` to `tool.uv.workspace.roots` and run `uv lock` to resolve its dependency groups.
    ");
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--package", "shared", "--no-default-groups", "--no-install-workspace",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked in [TIME]
    ");
    context
        .lock()
        .args([
            "--offline",
            "--no-index",
            "--upgrade",
            "--preview-features",
            "lock-without-metadata",
        ])
        .assert()
        .success();
    let lock: toml::Value = toml::from_str(&context.read("uv.lock"))?;
    let shared = lock["package"]
        .as_array()
        .expect("locked packages")
        .iter()
        .find(|package| package["name"].as_str() == Some("shared"))
        .expect("shared package");
    assert!(
        shared
            .get("metadata")
            .and_then(|metadata| metadata.get("requires-dev"))
            .is_none()
    );
    assert!(
        shared["dev-dependencies"]["dev"]
            .as_array()
            .expect("group placeholder")
            .is_empty()
    );
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--package", "shared", "--group", "dev", "--no-install-workspace",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Group `dev` for workspace member `shared` was not resolved

    hint: Add `shared` to `tool.uv.workspace.roots` and run `uv lock` to resolve its dependency groups.
    ");
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--package", "shared", "--no-install-workspace",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Group `dev` for workspace member `shared` was not resolved

    hint: Add `shared` to `tool.uv.workspace.roots` and run `uv lock` to resolve its dependency groups.
    ");
    Ok(())
}

#[test]
fn explicit_roots_rejects_unresolved_member_extras() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["shared"]
        [tool.uv]
        package = false
        [tool.uv.sources]
        shared = { workspace = true }
        [tool.uv.workspace]
        members = ["shared"]
        roots = ["app"]
    "#})?;
    context
        .temp_dir
        .child("shared/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "shared"
        version = "0.1.0"
        requires-python = ">=3.12"
        [project.optional-dependencies]
        feature = ["missing-leaf"]
        [tool.uv]
        package = false
    "#})?;
    context
        .lock()
        .args(["--offline", "--no-index"])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--offline", "--package", "shared", "--only-dev", "--extra", "feature",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked in [TIME]
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--offline", "--package", "shared", "--only-dev", "--extra", "feature", "--no-header",
    ]), @"exit_code: 0 (success)");
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--package", "shared", "--extra", "feature",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Extra `feature` for workspace member `shared` was not resolved for this selection

    hint: Add `shared` to `tool.uv.workspace.roots` and run `uv lock` to resolve its optional dependencies.
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--package", "shared", "--all-extras", "--no-header",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Extra `feature` for workspace member `shared` was not resolved for this selection

    hint: Add `shared` to `tool.uv.workspace.roots` and run `uv lock` to resolve its optional dependencies.
    ");
    context
        .lock()
        .args([
            "--offline",
            "--no-index",
            "--upgrade",
            "--preview-features",
            "lock-without-metadata",
        ])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--offline", "--package", "shared", "--only-dev", "--all-extras",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked in [TIME]
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--offline", "--package", "shared", "--only-dev", "--all-extras", "--no-header",
    ]), @"exit_code: 0 (success)");
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--package", "shared", "--all-extras",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Extra `feature` for workspace member `shared` was not resolved for this selection

    hint: Add `shared` to `tool.uv.workspace.roots` and run `uv lock` to resolve its optional dependencies.
    ");
    Ok(())
}

#[test]
fn explicit_roots_resolved_empty_extra_preserves_platform_domain() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let active_platform = if cfg!(windows) {
        "win32"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    };
    let inactive_target = if cfg!(windows) {
        "x86_64-apple-darwin"
    } else {
        "x86_64-pc-windows-msvc"
    };
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["shared", "shared[empty]; sys_platform == '{active_platform}'"]
        [tool.uv]
        package = false
        [tool.uv.sources]
        shared = {{ workspace = true }}
        [tool.uv.workspace]
        members = ["shared"]
        roots = ["app"]
    "#})?;
    context
        .temp_dir
        .child("shared/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "shared"
        version = "0.1.0"
        requires-python = ">=3.12"
        [project.optional-dependencies]
        empty = []
        [tool.uv]
        package = false
    "#})?;
    context
        .lock()
        .args(["--offline", "--no-index"])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--package", "shared", "--extra", "empty",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked in [TIME]
    ");
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--package", "shared", "--extra", "empty", "--python-platform",
    ]).arg(inactive_target), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Extra `empty` for workspace member `shared` was not resolved for this selection

    hint: Add `shared` to `tool.uv.workspace.roots` and run `uv lock` to resolve its optional dependencies.
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--package", "shared", "--extra", "empty", "--no-header",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Extra `empty` for workspace member `shared` was not resolved for this selection

    hint: Add `shared` to `tool.uv.workspace.roots` and run `uv lock` to resolve its optional dependencies.
    ");
    context
        .lock()
        .args([
            "--offline",
            "--no-index",
            "--upgrade",
            "--preview-features",
            "lock-without-metadata",
        ])
        .assert()
        .success();
    fs_err::remove_file(context.temp_dir.child("pyproject.toml"))?;
    fs_err::remove_file(context.temp_dir.child("shared/pyproject.toml"))?;
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--package", "shared", "--extra", "empty", "--preview-features", "frozen-lockfile",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked in [TIME]
    ");
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--package", "shared", "--extra", "empty", "--preview-features", "frozen-lockfile", "--python-platform",
    ]).arg(inactive_target), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Extra `empty` for workspace member `shared` was not resolved for this selection

    hint: Add `shared` to `tool.uv.workspace.roots` and run `uv lock` to resolve its optional dependencies.
    ");
    // Older metadata-free locks can retain an empty section without evidence of an incoming request.
    context
        .temp_dir
        .child("uv.lock")
        .write_str(&context.read("uv.lock").replace("extra = [\"empty\"], ", ""))?;
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--package", "shared", "--extra", "empty", "--preview-features", "frozen-lockfile",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Extra `empty` for workspace member `shared` was not resolved for this selection

    hint: Add `shared` to `tool.uv.workspace.roots` and run `uv lock` to resolve its optional dependencies.
    ");
    Ok(())
}

#[test]
fn explicit_roots_selected_non_root_python_requirement() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"]);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["shared; python_version >= '3.13'"]
        [tool.uv]
        package = false
        [tool.uv.sources]
        shared = { workspace = true }
        [tool.uv.workspace]
        members = ["shared"]
        roots = ["app"]
    "#})?;
    context
        .temp_dir
        .child("shared/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "shared"
        version = "0.1.0"
        requires-python = ">=3.13"
        [tool.uv]
        package = false
    "#})?;
    context
        .lock()
        .args(["--offline", "--no-index", "--python", "3.12"])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--package", "shared", "--python", "3.12",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    error: The requested interpreter resolved to Python 3.12.[X], which is incompatible with the project's Python requirement: `>=3.13` (from `shared` in `uv.lock`).
    ");
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--package", "shared", "--python", "3.13",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.13.[X] interpreter at: [PYTHON-3.13]
    Creating virtual environment at: .venv
    Checked in [TIME]
    ");
    context
        .lock()
        .args([
            "--offline",
            "--no-index",
            "--upgrade",
            "--python",
            "3.12",
            "--preview-features",
            "lock-without-metadata",
        ])
        .assert()
        .success();
    fs_err::remove_file(context.temp_dir.child("pyproject.toml"))?;
    fs_err::remove_file(context.temp_dir.child("shared/pyproject.toml"))?;
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--package", "shared", "--python", "3.12", "--preview-features", "frozen-lockfile",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    error: The requested interpreter resolved to Python 3.12.[X], which is incompatible with the project's Python requirement: `>=3.13` (from `shared` in `uv.lock`).
    ");
    context.temp_dir.child("uv.lock").write_str(
        &context
            .read("uv.lock")
            .replace("requires-python = \">=3.13\"", ""),
    )?;
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--package", "shared", "--python", "3.13", "--preview-features", "frozen-lockfile",
    ]), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Python requirement for workspace member `shared` is missing from the lockfile

    hint: Run `uv lock` to record the selected workspace member's Python requirement.
    "#);
    Ok(())
}

#[test]
fn explicit_roots_workspace_group_resolves_non_root_empty_extra() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [dependency-groups]
        dev = ["shared[empty]"]
        [tool.uv.workspace]
        members = ["app", "shared"]
        roots = ["app"]
        [tool.uv.sources]
        shared = { workspace = true }
    "#})?;
    context
        .temp_dir
        .child("app/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("shared/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "shared"
        version = "0.1.0"
        requires-python = ">=3.12"
        [project.optional-dependencies]
        empty = []
        [tool.uv]
        package = false
    "#})?;
    context
        .lock()
        .args(["--offline", "--no-index"])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--package", "shared", "--extra", "empty", "--no-default-groups",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked in [TIME]
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--package", "shared", "--extra", "empty", "--no-default-groups", "--no-header",
    ]), @"exit_code: 0 (success)");
    context
        .lock()
        .args([
            "--offline",
            "--no-index",
            "--upgrade",
            "--preview-features",
            "lock-without-metadata",
        ])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--package", "shared", "--extra", "empty", "--no-default-groups",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked in [TIME]
    ");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&context.read("pyproject.toml").replace(
            "[tool.uv.sources]",
            "[tool.uv]\noverride-dependencies = [\"shared\"]\n[tool.uv.sources]",
        ))?;
    context
        .lock()
        .args([
            "--offline",
            "--no-index",
            "--upgrade",
            "--preview-features",
            "lock-without-metadata",
        ])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--package", "shared", "--extra", "empty", "--no-default-groups",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Extra `empty` for workspace member `shared` was not resolved for this selection

    hint: Add `shared` to `tool.uv.workspace.roots` and run `uv lock` to resolve its optional dependencies.
    ");
    Ok(())
}

#[test]
fn explicit_roots_non_root_production_preserves_platform_domain() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let active_platform = if cfg!(windows) {
        "win32"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    };
    let (inactive_platform, inactive_target) = if cfg!(windows) {
        ("darwin", "x86_64-apple-darwin")
    } else {
        ("win32", "x86_64-pc-windows-msvc")
    };
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["shared; sys_platform == '{active_platform}'"]

        [dependency-groups]
        check = []

        [tool.uv]
        package = false

        [tool.uv.sources]
        shared = {{ workspace = true }}
        leaf = {{ workspace = true }}

        [tool.uv.workspace]
        members = ["shared", "leaf"]
        roots = ["app"]
    "#})?;
    context
        .temp_dir
        .child("shared/pyproject.toml")
        .write_str(&formatdoc! {r#"
        [project]
        name = "shared"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["leaf; sys_platform == '{inactive_platform}'"]

        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("leaf/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "leaf"
        version = "0.1.0"
        requires-python = ">=3.12"

        [build-system]
        requires = []
        build-backend = "uv_build"
    "#})?;
    context
        .temp_dir
        .child("leaf/src/leaf/__init__.py")
        .touch()?;
    context
        .lock()
        .args(["--offline", "--no-index"])
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.sync().args([
        "--package", "shared", "--offline", "--dry-run", "--python-platform",
    ]).arg(inactive_target), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Would use project environment at: .venv
    Resolved 2 packages in [TIME]
    Found up-to-date lockfile at: uv.lock
    error: Dependencies for workspace member `shared` were not resolved for this selection

    hint: Add `shared` to `tool.uv.workspace.roots` and run `uv lock` to resolve its dependencies.
    ");
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--package", "shared", "--offline", "--dry-run", "--python-platform",
    ]).arg(inactive_target), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Would use project environment at: .venv
    error: Dependencies for workspace member `shared` were not resolved for this selection

    hint: Add `shared` to `tool.uv.workspace.roots` and run `uv lock` to resolve its dependencies.
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--package", "shared", "--offline", "--no-header", "--no-hashes",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Dependencies for workspace member `shared` were not resolved for this selection

    hint: Add `shared` to `tool.uv.workspace.roots` and run `uv lock` to resolve its dependencies.
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--package", "shared", "--offline", "--format", "cyclonedx1.5",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Dependencies for workspace member `shared` were not resolved for this selection

    hint: Add `shared` to `tool.uv.workspace.roots` and run `uv lock` to resolve its dependencies.
    ");
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--package", "shared", "--offline", "--dry-run",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Would use project environment at: .venv
    Checked in [TIME]
    Would make no changes
    ");
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--package", "shared", "--only-group", "check", "--offline", "--dry-run", "--python-platform",
    ]).arg(inactive_target), @"
    exit_code: 0 (success)
    ----- stderr -----
    Would use project environment at: .venv
    Checked in [TIME]
    Would make no changes
    ");

    context
        .lock()
        .args([
            "--offline",
            "--no-index",
            "--upgrade",
            "--preview-features",
            "lock-without-metadata",
        ])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--package", "shared", "--offline", "--dry-run", "--python-platform",
    ]).arg(inactive_target), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Would use project environment at: .venv
    error: Dependencies for workspace member `shared` were not resolved for this selection

    hint: Add `shared` to `tool.uv.workspace.roots` and run `uv lock` to resolve its dependencies.
    ");

    context.temp_dir.child("pyproject.toml").write_str(
        &context
            .read("pyproject.toml")
            .replace("roots = [\"app\"]", "roots = [\"app\", \"shared\"]"),
    )?;
    context
        .lock()
        .args(["--offline", "--no-index"])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--package", "shared", "--offline", "--dry-run", "--python-platform",
    ]).arg(inactive_target), @"
    exit_code: 0 (success)
    ----- stderr -----
    Would use project environment at: .venv
    Would download 1 package
    Would install 1 package
     + leaf @ file://[TEMP_DIR]/leaf
    ");
    Ok(())
}

#[test]
fn explicit_roots_pylock_matches_selected_python_domain() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"]);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["shared; python_version >= '3.13'", "leaf; python_version < '3.13'"]

        [dependency-groups]
        check = []

        [tool.uv]
        package = false

        [tool.uv.dependency-groups]
        check = { requires-python = ">=3.13" }

        [tool.uv.sources]
        shared = { workspace = true }
        leaf = { workspace = true }

        [tool.uv.workspace]
        members = ["shared", "leaf"]
        roots = ["app"]
    "#})?;
    context
        .temp_dir
        .child("shared/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "shared"
        version = "0.1.0"
        requires-python = ">=3.13"

        [build-system]
        requires = []
        build-backend = "uv_build"
    "#})?;
    context
        .temp_dir
        .child("shared/src/shared/__init__.py")
        .touch()?;
    context
        .temp_dir
        .child("leaf/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "leaf"
        version = "0.1.0"
        requires-python = ">=3.12"

        [build-system]
        requires = []
        build-backend = "uv_build"
    "#})?;
    context
        .temp_dir
        .child("leaf/src/leaf/__init__.py")
        .touch()?;
    context
        .lock()
        .args(["--offline", "--no-index", "--python", "3.12"])
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--offline", "--package", "shared", "--format", "pylock.toml", "--no-header",
    ]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    lock-version = "1.0"
    created-by = "uv"
    requires-python = ">=3.13"

    [[packages]]
    name = "shared"
    directory = { path = "shared", editable = true }
    "#);
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--offline", "--package", "app", "--group", "check", "--format", "pylock.toml", "--no-header",
    ]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    lock-version = "1.0"
    created-by = "uv"
    requires-python = ">=3.13"

    [[packages]]
    name = "shared"
    directory = { path = "shared", editable = true }
    "#);

    // A member with wider bounds cannot widen the lockfile's resolved Python domain.
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&context.read("pyproject.toml").replace(
            r#"requires-python = ">=3.12""#,
            r#"requires-python = ">=3.13""#,
        ))?;
    context.temp_dir.child("shared/pyproject.toml").write_str(
        &context.read("shared/pyproject.toml").replace(
            r#"requires-python = ">=3.13""#,
            r#"requires-python = ">=3.12""#,
        ),
    )?;
    context
        .lock()
        .args(["--offline", "--no-index", "--python", "3.13"])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--offline", "--package", "shared", "--format", "pylock.toml", "--no-header",
    ]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    lock-version = "1.0"
    created-by = "uv"
    requires-python = ">=3.13"

    [[packages]]
    name = "shared"
    directory = { path = "shared", editable = true }
    "#);
    Ok(())
}

/// Registry dependencies retain the local identity of non-root workspace members.
#[cfg(feature = "test-universal")]
#[test]
fn explicit_roots_transitive_registry_workspace_source() -> Result<()> {
    let server = PackseServer::from_scenario(&toml::from_str(indoc! {r#"
        name = "explicit-roots-workspace-source"
        [root]
        [expected]
        satisfiable = true
        [packages.bridge.versions."1.0.0"]
        requires = ["shared>=1"]
        sdist = false
        [packages.shared.versions."2.0.0"]
        sdist = false
    "#})?);
    let context = uv_test::test_context!("3.12").with_cyclonedx_filters();
    let index = server.index_url();
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["bridge"]
        [tool.uv]
        package = false
        [tool.uv.workspace]
        members = ["shared", "unused"]
        roots = ["app"]
        [[tool.uv.index]]
        url = "{index}"
        default = true
    "#})?;
    context
        .temp_dir
        .child("shared/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "shared"
        version = "1.0.0"
        requires-python = ">=3.12"
        [build-system]
        requires = []
        build-backend = "uv_build"
    "#})?;
    context
        .temp_dir
        .child("shared/src/shared/__init__.py")
        .touch()?;
    context
        .temp_dir
        .child("unused/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "unused"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["missing-package"]
        [tool.uv]
        package = false
    "#})?;
    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--offline", "--only-emit-workspace", "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    -e ./shared
    ");
    let locked = context.read("uv.lock");
    uv_snapshot!(context.filters(), context.lock().args(["--locked", "--offline", "--no-cache"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    assert_eq!(locked, context.read("uv.lock"));
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--offline", "--format", "cyclonedx1.5",
    ]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "bomFormat": "CycloneDX",
      "specVersion": "1.5",
      "version": 1,
      "serialNumber": "[SERIAL_NUMBER]",
      "metadata": {
        "timestamp": "[TIMESTAMP]",
        "tools": [
          {
            "vendor": "Astral Software Inc.",
            "name": "uv",
            "version": "[VERSION]"
          }
        ],
        "component": {
          "type": "library",
          "bom-ref": "app-1@0.1.0",
          "name": "app",
          "version": "0.1.0",
          "properties": [
            {
              "name": "uv:package:is_project_root",
              "value": "true"
            }
          ]
        }
      },
      "components": [
        {
          "type": "library",
          "bom-ref": "bridge-2@1.0.0",
          "name": "bridge",
          "version": "1.0.0",
          "purl": "pkg:pypi/bridge@1.0.0?repository_url=http://[LOCALHOST]/simple/",
          "externalReferences": [
            {
              "type": "distribution",
              "url": "http://[LOCALHOST]/files/bridge-1.0.0-py3-none-any.whl",
              "hashes": [
                {
                  "alg": "SHA-256",
                  "content": "b0aeb6ee8c8b30dba556dc6b3103ed87bc682cc043d6088cfb6205f63d3277f3"
                }
              ]
            }
          ]
        },
        {
          "type": "library",
          "bom-ref": "shared-3@1.0.0",
          "name": "shared",
          "version": "1.0.0",
          "properties": [
            {
              "name": "uv:workspace:path",
              "value": "shared"
            }
          ]
        }
      ],
      "dependencies": [
        {
          "ref": "app-1@0.1.0",
          "dependsOn": [
            "bridge-2@1.0.0"
          ]
        },
        {
          "ref": "bridge-2@1.0.0",
          "dependsOn": [
            "shared-3@1.0.0"
          ]
        },
        {
          "ref": "shared-3@1.0.0"
        }
      ]
    }
    ----- stderr -----
    warning: `uv export --format=cyclonedx1.5` is experimental and may change without warning. Pass `--preview-features sbom-export` to disable this warning.
    "#);
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--offline", "--dry-run", "--only-install-workspace",
    ]), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Would use project environment at: .venv
    Would download 1 package
    Would install 1 package
     + shared @ file://[TEMP_DIR]/shared
    ");

    // A published version cannot satisfy a requirement that excludes the local member.
    context.temp_dir.child("shared/pyproject.toml").write_str(
        &context
            .read("shared/pyproject.toml")
            .replace(r#"version = "1.0.0""#, r#"version = "0.1.0""#),
    )?;
    fs_err::remove_file(context.temp_dir.child("uv.lock"))?;
    uv_snapshot!(context.filters(), context.lock(), @r"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because all versions of bridge depend on shared and app depends on bridge, we can conclude that app's requirements are unsatisfiable.
             And because only app==0.1.0 is available and your workspace requires app, we can conclude that your workspace's requirements are unsatisfiable.

    hint: The package `bridge` depends on the package `shared` but the name is shadowed by one of your workspace members. Consider changing the name of the workspace member.
    ");
    Ok(())
}

/// Selecting a root extra does not request the same extra on a bare transitive member.
#[cfg(feature = "test-universal")]
#[test]
fn explicit_roots_conflicting_extras_follow_selected_roots() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context.temp_dir.child("pyproject.toml").write_str(indoc! {r#"
        [tool.uv.workspace]
        members = ["app", "shared", "feature-leaf"]
        roots = ["app"]
        [tool.uv.sources]
        shared = { workspace = true }
        feature-leaf = { workspace = true }
        [tool.uv]
        conflicts = [[{ package = "app", extra = "feature" }, { package = "shared", extra = "feature" }]]
    "#})?;
    context
        .temp_dir
        .child("app/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["shared"]
        [project.optional-dependencies]
        feature = ["feature-leaf"]
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("shared/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "shared"
        version = "0.1.0"
        requires-python = ">=3.12"
        [project.optional-dependencies]
        feature = []
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("feature-leaf/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "feature-leaf"
        version = "0.1.0"
        requires-python = ">=3.12"
        [build-system]
        requires = []
        build-backend = "uv_build"
    "#})?;
    context
        .temp_dir
        .child("feature-leaf/src/feature_leaf/__init__.py")
        .touch()?;
    context
        .lock()
        .args(["--offline", "--no-index"])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--offline", "--dry-run", "--extra", "feature",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Would use project environment at: .venv
    Would download 1 package
    Would install 1 package
     + feature-leaf @ file://[TEMP_DIR]/feature-leaf
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--offline", "--extra", "feature", "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    -e ./feature-leaf
    ");
    // Resolve both members as roots before requesting their conflicting selections together.
    context.temp_dir.child("pyproject.toml").write_str(
        &context
            .read("pyproject.toml")
            .replace(r#"roots = ["app"]"#, r#"roots = ["app", "shared"]"#),
    )?;
    context
        .lock()
        .args(["--offline", "--no-index"])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--offline", "--dry-run", "--extra", "feature", "--package", "app", "--package", "shared",
    ]), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    Would use project environment at: .venv
    error: Extras `feature` and `feature` are incompatible with the declared conflicts: {`app[feature]`, `shared[feature]`}
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--offline", "--extra", "feature", "--package", "app", "--package", "shared",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Extras `feature` and `feature` are incompatible with the declared conflicts: {`app[feature]`, `shared[feature]`}
    ");
    Ok(())
}

/// Group selections apply to installation roots rather than their transitive workspace members.
#[cfg(feature = "test-universal")]
#[test]
fn explicit_roots_conflicting_groups_follow_selected_roots() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context.temp_dir.child("pyproject.toml").write_str(indoc! {r#"
        [tool.uv.workspace]
        members = ["app", "shared", "group-leaf"]
        roots = ["app"]
        [tool.uv.sources]
        shared = { workspace = true }
        group-leaf = { workspace = true }
        [tool.uv]
        conflicts = [[{ package = "app", group = "check" }, { package = "shared", group = "check" }]]
    "#})?;
    context
        .temp_dir
        .child("app/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["shared"]
        [dependency-groups]
        check = ["group-leaf"]
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("shared/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "shared"
        version = "0.1.0"
        requires-python = ">=3.12"
        [dependency-groups]
        check = []
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("group-leaf/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "group-leaf"
        version = "0.1.0"
        requires-python = ">=3.12"
        [build-system]
        requires = []
        build-backend = "uv_build"
    "#})?;
    context
        .temp_dir
        .child("group-leaf/src/group_leaf/__init__.py")
        .touch()?;
    context
        .lock()
        .args(["--offline", "--no-index"])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--offline", "--dry-run", "--only-group", "check",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Would use project environment at: .venv
    Would download 1 package
    Would install 1 package
     + group-leaf @ file://[TEMP_DIR]/group-leaf
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--offline", "--only-group", "check", "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    -e ./group-leaf
    ");
    // Resolve both members as roots before requesting their conflicting selections together.
    context.temp_dir.child("pyproject.toml").write_str(
        &context
            .read("pyproject.toml")
            .replace(r#"roots = ["app"]"#, r#"roots = ["app", "shared"]"#),
    )?;
    context
        .lock()
        .args(["--offline", "--no-index"])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--offline", "--dry-run", "--only-group", "check", "--package", "app", "--package", "shared",
    ]), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    Would use project environment at: .venv
    error: Groups `check` and `check` are incompatible with the conflicts: {`app:check`, `shared:check`}
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--offline", "--only-group", "check", "--package", "app", "--package", "shared",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Groups `check` and `check` are incompatible with the conflicts: {`app:check`, `shared:check`}
    ");
    Ok(())
}

/// A selected member inherits explicitly requested workspace-root groups for conflict checks.
#[cfg(feature = "test-universal")]
#[test]
fn explicit_roots_conflicting_groups_include_inherited_root() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context.temp_dir.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["shared"]
        [dependency-groups]
        check = ["group-leaf"]
        [tool.uv.workspace]
        members = ["shared", "group-leaf"]
        roots = ["app"]
        [tool.uv.sources]
        shared = { workspace = true }
        group-leaf = { workspace = true }
        [tool.uv]
        package = false
        conflicts = [[{ package = "app", group = "check" }, { package = "shared", group = "member-check" }]]
    "#})?;
    context
        .temp_dir
        .child("shared/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "shared"
        version = "0.1.0"
        requires-python = ">=3.12"
        [dependency-groups]
        member-check = []
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("group-leaf/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "group-leaf"
        version = "0.1.0"
        requires-python = ">=3.12"
        [build-system]
        requires = []
        build-backend = "uv_build"
    "#})?;
    context
        .temp_dir
        .child("group-leaf/src/group_leaf/__init__.py")
        .touch()?;
    context
        .lock()
        .args(["--offline", "--no-index"])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--offline", "--dry-run", "--package", "shared", "--only-group", "check",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Would use project environment at: .venv
    Would download 1 package
    Would install 1 package
     + group-leaf @ file://[TEMP_DIR]/group-leaf
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--offline", "--package", "shared", "--only-group", "check",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    -e ./group-leaf
    ");
    // Resolve both members as roots before requesting their conflicting selections together.
    context.temp_dir.child("pyproject.toml").write_str(
        &context
            .read("pyproject.toml")
            .replace(r#"roots = ["app"]"#, r#"roots = ["app", "shared"]"#),
    )?;
    context
        .lock()
        .args(["--offline", "--no-index"])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--offline", "--dry-run", "--package", "shared", "--only-group", "check", "--only-group", "member-check",
    ]), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    Would use project environment at: .venv
    error: Groups `check` and `member-check` are incompatible with the conflicts: {`app:check`, `shared:member-check`}
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--offline", "--package", "shared", "--only-group", "check", "--only-group", "member-check",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Groups `check` and `member-check` are incompatible with the conflicts: {`app:check`, `shared:member-check`}
    ");
    Ok(())
}

/// Resolved members from unselected groups do not activate project conflicts.
#[cfg(feature = "test-universal")]
#[test]
fn explicit_roots_project_conflicts_follow_selected_dependencies() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv.workspace]
        members = ["app", "shared"]
        roots = ["app"]
        [tool.uv.sources]
        shared = { workspace = true }
        [tool.uv]
        conflicts = [[{ package = "app" }, { package = "shared" }]]
    "#})?;
    context
        .temp_dir
        .child("app/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        [dependency-groups]
        dev = ["shared"]
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("shared/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "shared"
        version = "0.1.0"
        requires-python = ">=3.12"
        [build-system]
        requires = []
        build-backend = "uv_build"
    "#})?;
    context
        .temp_dir
        .child("shared/src/shared/__init__.py")
        .touch()?;
    context
        .lock()
        .args([
            "--offline",
            "--no-index",
            "--preview-features",
            "package-conflicts",
        ])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--offline", "--dry-run", "--no-default-groups", "--package", "app",
        "--preview-features", "package-conflicts",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Would use project environment at: .venv
    Checked in [TIME]
    Would make no changes
    ");
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--offline", "--dry-run", "--no-default-groups",
        "--preview-features", "package-conflicts",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Would use project environment at: .venv
    Checked in [TIME]
    Would make no changes
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--offline", "--no-default-groups", "--no-header", "--no-hashes", "--no-annotate",
        "--preview-features", "package-conflicts",
    ]), @"exit_code: 0 (success)");
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--offline", "--dry-run", "--group", "dev",
        "--preview-features", "package-conflicts",
    ]), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    Would use project environment at: .venv
    error: Package `app` and package `shared` are incompatible with the declared conflicts: {app, shared}
    "#);
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--offline", "--group", "dev", "--no-header", "--no-hashes", "--no-annotate",
        "--preview-features", "package-conflicts",
    ]), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Package `app` and package `shared` are incompatible with the declared conflicts: {app, shared}
    "#);
    Ok(())
}

/// Transitive project conflicts follow the selected locked version through registry dependencies.
#[cfg(feature = "test-universal")]
#[test]
fn explicit_roots_project_conflicts_follow_locked_dependency_identity() -> Result<()> {
    let server = PackseServer::from_scenario(&toml::from_str(indoc! {r#"
        name = "explicit-roots-project-conflict-identity"
        [root]
        [expected]
        satisfiable = true
        [packages.bridge.versions."1.0.0"]
        requires = ["shared"]
        sdist = false
        [packages.bridge.versions."2.0.0"]
        sdist = false
    "#})?);
    let context = uv_test::test_context!("3.12");
    let index = server.index_url();
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
        [tool.uv.workspace]
        members = ["app", "shared"]
        roots = ["app"]
        [tool.uv]
        conflicts = [
            [{{ package = "app" }}, {{ package = "shared" }}],
            [{{ package = "app", extra = "legacy" }}, {{ package = "app", extra = "modern" }}],
        ]
        [[tool.uv.index]]
        url = "{index}"
        default = true
    "#})?;
    context
        .temp_dir
        .child("app/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["bridge"]
        [project.optional-dependencies]
        legacy = ["bridge<2"]
        modern = ["bridge>=2"]
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("shared/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "shared"
        version = "0.1.0"
        requires-python = ">=3.12"
        [tool.uv]
        package = false
    "#})?;
    context
        .lock()
        .args(["--preview-features", "package-conflicts"])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.sync().args(["--package", "app"]).args([
        "--frozen", "--offline", "--dry-run", "--extra", "legacy", "--preview-features", "package-conflicts",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Would use project environment at: .venv
    error: Package `app` and package `shared` are incompatible with the declared conflicts: {app, shared}
    ");
    uv_snapshot!(context.filters(), context.export().args(["--package", "app"]).args([
        "--frozen", "--offline", "--extra", "legacy", "--no-header", "--no-hashes", "--no-annotate",
        "--preview-features", "package-conflicts",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Package `app` and package `shared` are incompatible with the declared conflicts: {app, shared}
    ");
    uv_snapshot!(context.filters(), context.export().args(["--package", "app"]).args([
        "--frozen", "--offline", "--extra", "modern", "--no-header", "--no-hashes", "--no-annotate",
        "--preview-features", "package-conflicts",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    bridge==2.0.0
    ");
    uv_snapshot!(context.filters(), context.sync().args(["--package", "app"]).args([
        "--frozen", "--offline", "--dry-run", "--extra", "modern", "--preview-features", "package-conflicts",
    ]), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Would use project environment at: .venv
    Would download 1 package
    Would install 1 package
     + bridge==2.0.0
    "#);
    uv_snapshot!(context.filters(), context.export().args([
        "--package", "app", "--frozen", "--offline", "--no-header", "--no-hashes", "--no-annotate",
        "--preview-features", "package-conflicts",
    ]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    bridge==2.0.0
    "#);
    uv_snapshot!(context.filters(), context.sync().args([
        "--package", "app", "--frozen", "--offline", "--dry-run", "--preview-features", "package-conflicts",
    ]), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Would use project environment at: .venv
    Would download 1 package
    Would install 1 package
     + bridge==2.0.0
    "#);
    Ok(())
}

/// Selected manifest groups apply global overrides before traversing their locked dependencies.
#[cfg(feature = "test-universal")]
#[test]
fn explicit_roots_project_conflicts_include_manifest_group_overrides() -> Result<()> {
    let server = PackseServer::from_scenario(&toml::from_str(indoc! {r#"
        name = "explicit-roots-manifest-group-conflict"
        [root]
        [expected]
        satisfiable = true
        [packages.bridge.versions."1.0.0"]
        sdist = false
        [packages.bridge.versions."2.0.0"]
        requires = ["shared"]
        sdist = false
    "#})?);
    let context = uv_test::test_context!("3.12");
    let index = server.index_url();
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
        [dependency-groups]
        dev = ["bridge<2"]
        [tool.uv.workspace]
        members = ["app", "shared"]
        roots = ["app"]
        [tool.uv]
        override-dependencies = ["bridge==2"]
        conflicts = [[{{ package = "app" }}, {{ package = "shared" }}]]
        [[tool.uv.index]]
        url = "{index}"
        default = true
    "#})?;
    context
        .temp_dir
        .child("app/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("shared/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "shared"
        version = "0.1.0"
        requires-python = ">=3.12"
        [tool.uv]
        package = false
    "#})?;
    context
        .lock()
        .args(["--preview-features", "package-conflicts"])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--offline", "--dry-run", "--no-default-groups", "--preview-features", "package-conflicts",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Would use project environment at: .venv
    Checked in [TIME]
    Would make no changes
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--offline", "--no-default-groups", "--no-header", "--no-hashes", "--no-annotate",
        "--preview-features", "package-conflicts",
    ]), @"exit_code: 0 (success)");
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--offline", "--dry-run", "--group", "dev", "--preview-features", "package-conflicts",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Would use project environment at: .venv
    error: Package `app` and package `shared` are incompatible with the declared conflicts: {app, shared}
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--offline", "--group", "dev", "--no-header", "--no-hashes", "--no-annotate",
        "--preview-features", "package-conflicts",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Package `app` and package `shared` are incompatible with the declared conflicts: {app, shared}
    ");
    Ok(())
}

/// Project conflict membership follows the concrete Python environment, including frozen locks.
#[cfg(feature = "test-universal")]
#[test]
fn explicit_roots_project_conflicts_respect_dependency_markers() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"]);
    context
        .venv()
        .arg(context.venv.path())
        .args(["--python", "3.12"])
        .assert()
        .success();
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv.workspace]
        members = ["app", "shared"]
        roots = ["app"]
        [tool.uv.sources]
        shared = { workspace = true }
        [tool.uv]
        conflicts = [[{ package = "app" }, { package = "shared" }]]
    "#})?;
    context
        .temp_dir
        .child("app/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        [dependency-groups]
        dev = ["shared; python_version >= '3.13'"]
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("shared/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "shared"
        version = "0.1.0"
        requires-python = ">=3.12"
        [tool.uv]
        package = false
    "#})?;
    context
        .lock()
        .args([
            "--offline",
            "--python",
            "3.12",
            "--preview-features",
            "package-conflicts",
        ])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--offline", "--dry-run", "--python", "3.12", "--preview-features", "package-conflicts",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Would use project environment at: .venv
    Checked in [TIME]
    Would make no changes
    ");
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--offline", "--dry-run", "--python", "3.13", "--preview-features", "package-conflicts",
    ]), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    Using CPython 3.13.[X] interpreter at: [PYTHON-3.13]
    Would replace project environment at: .venv
    error: Package `app` and package `shared` are incompatible with the declared conflicts: {app, shared}
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--offline", "--no-header", "--no-hashes", "--no-annotate", "--preview-features", "package-conflicts",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Package `app` and package `shared` are incompatible with the declared conflicts: {app, shared}
    ");

    fs_err::remove_file(context.temp_dir.child("uv.lock"))?;
    context
        .lock()
        .args([
            "--offline",
            "--python",
            "3.12",
            "--preview-features",
            "package-conflicts",
            "--preview-features",
            "lock-without-metadata",
        ])
        .assert()
        .success();
    assert!(
        !context
            .read("uv.lock")
            .contains("[package.metadata.requires-dev]")
    );
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--offline", "--dry-run", "--python", "3.12", "--preview-features", "package-conflicts",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Would use project environment at: .venv
    Checked in [TIME]
    Would make no changes
    ");
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--offline", "--dry-run", "--python", "3.13", "--preview-features", "package-conflicts",
    ]), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    Using CPython 3.13.[X] interpreter at: [PYTHON-3.13]
    Would replace project environment at: .venv
    error: Package `app` and package `shared` are incompatible with the declared conflicts: {app, shared}
    ");

    fs_err::remove_file(context.temp_dir.child("pyproject.toml"))?;
    fs_err::remove_file(context.temp_dir.child("app/pyproject.toml"))?;
    fs_err::remove_file(context.temp_dir.child("shared/pyproject.toml"))?;
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--offline", "--dry-run", "--python", "3.12", "--preview-features", "package-conflicts",
        "--preview-features", "frozen-lockfile",
    ]), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Would use project environment at: .venv
    Checked in [TIME]
    Would make no changes
    ");
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--offline", "--dry-run", "--python", "3.13", "--preview-features", "package-conflicts",
        "--preview-features", "frozen-lockfile",
    ]), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    Using CPython 3.13.[X] interpreter at: [PYTHON-3.13]
    Would replace project environment at: .venv
    error: Package `app` and package `shared` are incompatible with the declared conflicts: {app, shared}
    ");
    Ok(())
}

/// Invalid frozen root, extra, and group selections must not replace an existing environment.
#[cfg(feature = "test-universal")]
#[test]
fn explicit_roots_frozen_conflicts_preserve_environment() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"]);
    context
        .venv()
        .arg(context.venv.path())
        .args(["--python", "3.12"])
        .assert()
        .success();
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        [project.optional-dependencies]
        first = []
        second = []
        [dependency-groups]
        first = []
        second = []
        [tool.uv]
        package = false
        conflicts = [
            [{ extra = "first" }, { extra = "second" }],
            [{ group = "first" }, { group = "second" }],
            [{ package = "app" }, { package = "shared" }],
        ]
        [tool.uv.workspace]
        members = ["shared"]
        roots = ["app", "shared"]
    "#})?;
    context
        .temp_dir
        .child("shared/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "shared"
        version = "0.1.0"
        requires-python = ">=3.12"
        [tool.uv]
        package = false
    "#})?;
    context
        .lock()
        .args([
            "--offline",
            "--python",
            "3.12",
            "--preview-features",
            "package-conflicts",
        ])
        .assert()
        .success();
    fs_err::remove_file(context.temp_dir.child("pyproject.toml"))?;
    fs_err::remove_file(context.temp_dir.child("shared/pyproject.toml"))?;
    let marker = uv_test::site_packages_path(context.venv.path(), "python3.12").join("retained.py");
    fs_err::write(&marker, "value = 1")?;
    let configuration = context.read(".venv/pyvenv.cfg");

    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--offline", "--python", "3.13", "--extra", "first", "--extra", "second",
        "--preview-features", "frozen-lockfile", "--preview-features", "package-conflicts",
    ]), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Extras `first` and `second` are incompatible with the declared conflicts: {`app[first]`, `app[second]`}
    ");
    assert!(
        marker.exists(),
        "invalid extras must not clear the existing environment"
    );
    assert_eq!(context.read(".venv/pyvenv.cfg"), configuration);

    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--offline", "--python", "3.13", "--group", "first", "--group", "second",
        "--preview-features", "frozen-lockfile", "--preview-features", "package-conflicts",
    ]), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Groups `first` and `second` are incompatible with the conflicts: {`app:first`, `app:second`}
    ");
    assert!(
        marker.exists(),
        "invalid groups must not clear the existing environment"
    );
    assert_eq!(context.read(".venv/pyvenv.cfg"), configuration);
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--offline", "--python", "3.13", "--package", "app", "--package", "shared",
        "--preview-features", "frozen-lockfile", "--preview-features", "package-conflicts",
    ]), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Package `app` and package `shared` are incompatible with the declared conflicts: {app, shared}
    ");
    assert!(
        marker.exists(),
        "conflicting roots must not clear the existing environment"
    );
    assert_eq!(context.read(".venv/pyvenv.cfg"), configuration);
    Ok(())
}

/// Pylock conflict checks use the same Python domain as the selected member's output.
#[cfg(feature = "test-universal")]
#[test]
fn explicit_roots_pylock_conflicts_use_selected_python_domain() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"]);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["shared; python_version >= '3.13'"]
        [dependency-groups]
        dev = ["legacy; python_version < '3.13'"]
        [tool.uv]
        package = false
        conflicts = [[{ package = "shared" }, { package = "legacy" }]]
        [tool.uv.sources]
        shared = { workspace = true }
        legacy = { workspace = true }
        [tool.uv.workspace]
        members = ["shared", "legacy"]
        roots = ["app"]
    "#})?;
    context
        .temp_dir
        .child("shared/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "shared"
        version = "0.1.0"
        requires-python = ">=3.13"
        [build-system]
        requires = []
        build-backend = "uv_build"
    "#})?;
    context
        .temp_dir
        .child("shared/src/shared/__init__.py")
        .touch()?;
    context
        .temp_dir
        .child("legacy/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "legacy"
        version = "0.1.0"
        requires-python = ">=3.12"
        [build-system]
        requires = []
        build-backend = "uv_build"
    "#})?;
    context
        .temp_dir
        .child("legacy/src/legacy/__init__.py")
        .touch()?;
    context
        .lock()
        .args([
            "--offline",
            "--no-index",
            "--python",
            "3.12",
            "--preview-features",
            "package-conflicts",
        ])
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--offline", "--package", "app", "--group", "dev", "--format", "pylock.toml",
        "--no-header", "--preview-features", "package-conflicts",
    ]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    lock-version = "1.0"
    created-by = "uv"
    requires-python = ">=3.12"

    [[packages]]
    name = "legacy"
    marker = "python_full_version < '3.13'"
    directory = { path = "legacy", editable = true }

    [[packages]]
    name = "shared"
    marker = "python_full_version >= '3.13'"
    directory = { path = "shared", editable = true }
    "#);
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--offline", "--package", "shared", "--group", "dev", "--format", "pylock.toml",
        "--no-header", "--preview-features", "package-conflicts",
    ]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    lock-version = "1.0"
    created-by = "uv"
    requires-python = ">=3.13"

    [[packages]]
    name = "shared"
    directory = { path = "shared", editable = true }
    "#);
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--offline", "--package", "shared", "--group", "dev", "--format", "requirements.txt",
        "--no-header", "--preview-features", "package-conflicts",
    ]), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Package `legacy` and package `shared` are incompatible with the declared conflicts: {legacy, shared}
    ");

    context
        .lock()
        .args([
            "--offline",
            "--no-index",
            "--upgrade",
            "--python",
            "3.12",
            "--preview-features",
            "package-conflicts",
            "--preview-features",
            "lock-without-metadata",
        ])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--offline", "--package", "shared", "--group", "dev", "--format", "pylock.toml",
        "--no-header", "--preview-features", "package-conflicts",
    ]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    lock-version = "1.0"
    created-by = "uv"
    requires-python = ">=3.13"

    [[packages]]
    name = "shared"
    directory = { path = "shared", editable = true }
    "#);

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&context.read("pyproject.toml").replace(
            "legacy; python_version < '3.13'",
            "legacy; python_version >= '3.13'",
        ))?;
    context
        .lock()
        .args([
            "--offline",
            "--no-index",
            "--python",
            "3.12",
            "--preview-features",
            "package-conflicts",
        ])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--offline", "--package", "shared", "--group", "dev", "--format", "pylock.toml",
        "--no-header", "--preview-features", "package-conflicts",
    ]), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Package `legacy` and package `shared` are incompatible with the declared conflicts: {legacy, shared}
    ");
    Ok(())
}

/// Dependency-activated extras select the same locked fork for conflict checks and installation.
#[cfg(feature = "test-universal")]
#[test]
fn explicit_roots_project_conflicts_follow_transitive_extras() -> Result<()> {
    let server = PackseServer::from_scenario(&toml::from_str(indoc! {r#"
        name = "explicit-roots-transitive-extra-forks"
        [root]
        [expected]
        satisfiable = true
        [packages.bridge.versions."1.0.0"]
        requires = ["shared", "selector[legacy]"]
        sdist = false
        [packages.bridge.versions."2.0.0"]
        sdist = false
    "#})?);
    let context = uv_test::test_context!("3.12");
    let index = server.index_url();
    context.temp_dir.child("pyproject.toml").write_str(&formatdoc! {r#"
        [tool.uv.workspace]
        members = ["app", "selector", "shared"]
        roots = ["app", "selector"]
        [tool.uv.sources]
        selector = {{ workspace = true }}
        [tool.uv]
        conflicts = [
            [{{ package = "app" }}, {{ package = "shared" }}],
            [{{ package = "selector", extra = "legacy" }}, {{ package = "selector", extra = "modern" }}],
        ]
        [[tool.uv.index]]
        url = "{index}"
        default = true
    "#})?;
    context
        .temp_dir
        .child("app/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["bridge", "selector[modern]"]
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("selector/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "selector"
        version = "0.1.0"
        requires-python = ">=3.12"
        [project.optional-dependencies]
        legacy = ["bridge<2"]
        modern = ["bridge>=2"]
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("shared/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "shared"
        version = "0.1.0"
        requires-python = ">=3.12"
        [tool.uv]
        package = false
    "#})?;
    context
        .lock()
        .args(["--preview-features", "package-conflicts"])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.sync().args([
        "--package", "app", "--frozen", "--offline", "--dry-run", "--preview-features", "package-conflicts",
    ]), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Would use project environment at: .venv
    Would download 1 package
    Would install 1 package
     + bridge==2.0.0
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--package", "app", "--frozen", "--offline", "--no-header", "--no-hashes", "--no-annotate",
        "--preview-features", "package-conflicts",
    ]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    bridge==2.0.0
    ");

    uv_snapshot!(context.filters(), context.sync().args([
        "--package", "app", "--package", "selector", "--frozen", "--offline", "--dry-run",
        "--preview-features", "package-conflicts",
    ]), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Would use project environment at: .venv
    Would download 1 package
    Would install 1 package
     + bridge==2.0.0
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--package", "app", "--package", "selector", "--frozen", "--offline", "--no-header", "--no-hashes",
        "--no-annotate", "--preview-features", "package-conflicts",
    ]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    bridge==2.0.0
    ");

    context
        .lock()
        .args([
            "--upgrade",
            "--preview-features",
            "package-conflicts",
            "--preview-features",
            "lock-without-metadata",
        ])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.sync().args([
        "--package", "app", "--frozen", "--offline", "--dry-run", "--preview-features", "package-conflicts",
    ]), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Would use project environment at: .venv
    Would download 1 package
    Would install 1 package
     + bridge==2.0.0
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--package", "app", "--frozen", "--offline", "--no-header", "--no-hashes", "--no-annotate",
        "--preview-features", "package-conflicts",
    ]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    bridge==2.0.0
    ");

    context.temp_dir.child("app/pyproject.toml").write_str(
        &context
            .read("app/pyproject.toml")
            .replace("selector[modern]", "selector[legacy]"),
    )?;
    context
        .lock()
        .args(["--preview-features", "package-conflicts"])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.sync().args([
        "--package", "app", "--frozen", "--offline", "--dry-run", "--preview-features", "package-conflicts",
    ]), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    Would use project environment at: .venv
    error: Package `app` and package `shared` are incompatible with the declared conflicts: {app, shared}
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--package", "app", "--frozen", "--offline", "--no-header", "--no-hashes", "--no-annotate",
        "--preview-features", "package-conflicts",
    ]), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Package `app` and package `shared` are incompatible with the declared conflicts: {app, shared}
    ");
    Ok(())
}

/// Concrete Python facts resolve conditional extra activations through a dependency cycle.
#[cfg(feature = "test-universal")]
#[test]
fn explicit_roots_project_conflicts_follow_conditional_extras() -> Result<()> {
    let server = PackseServer::from_scenario(&toml::from_str(indoc! {r#"
        name = "explicit-roots-conditional-extra-cycle"
        [root]
        [expected]
        satisfiable = true
        [packages.bridge.versions."1.0.0"]
        requires = ["selector[modern]"]
        sdist = false
        [packages.bridge.versions."2.0.0"]
        requires = ["shared"]
        sdist = false
    "#})?);
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"]);
    context
        .venv()
        .arg(context.venv.path())
        .args(["--python", "3.12"])
        .assert()
        .success();
    let index = server.index_url();
    context.temp_dir.child("pyproject.toml").write_str(&formatdoc! {r#"
        [tool.uv.workspace]
        members = ["app", "selector", "shared"]
        roots = ["app", "selector"]
        [tool.uv.sources]
        selector = {{ workspace = true }}
        [tool.uv]
        conflicts = [
            [{{ package = "app" }}, {{ package = "shared" }}],
            [{{ package = "selector", extra = "legacy" }}, {{ package = "selector", extra = "modern" }}],
        ]
        [[tool.uv.index]]
        url = "{index}"
        default = true
    "#})?;
    context
        .temp_dir
        .child("app/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["bridge", "selector[modern]; python_version >= '3.13'"]
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("selector/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "selector"
        version = "0.1.0"
        requires-python = ">=3.12"
        [project.optional-dependencies]
        legacy = ["bridge>=2"]
        modern = ["bridge<2"]
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("shared/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "shared"
        version = "0.1.0"
        requires-python = ">=3.12"
        [tool.uv]
        package = false
    "#})?;
    context
        .lock()
        .args(["--preview-features", "package-conflicts"])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.sync().args([
        "--package", "app", "--python", "3.13", "--frozen", "--offline", "--dry-run",
        "--preview-features", "package-conflicts",
    ]), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.13.[X] interpreter at: [PYTHON-3.13]
    Would replace project environment at: .venv
    Would download 1 package
    Would install 1 package
     + bridge==1.0.0
    ");
    uv_snapshot!(context.filters(), context.sync().args([
        "--package", "app", "--python", "3.12", "--frozen", "--offline", "--dry-run",
        "--preview-features", "package-conflicts",
    ]), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    Would use project environment at: .venv
    error: Package `app` and package `shared` are incompatible with the declared conflicts: {app, shared}
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--package", "app", "--frozen", "--offline", "--no-header", "--no-hashes", "--no-annotate",
        "--preview-features", "package-conflicts",
    ]), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Package `app` and package `shared` are incompatible with the declared conflicts: {app, shared}
    ");

    context
        .lock()
        .args([
            "--upgrade",
            "--preview-features",
            "package-conflicts",
            "--preview-features",
            "lock-without-metadata",
        ])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.sync().args([
        "--package", "app", "--python", "3.13", "--frozen", "--offline", "--dry-run",
        "--preview-features", "package-conflicts",
    ]), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.13.[X] interpreter at: [PYTHON-3.13]
    Would replace project environment at: .venv
    Would download 1 package
    Would install 1 package
     + bridge==1.0.0
    ");
    uv_snapshot!(context.filters(), context.sync().args([
        "--package", "app", "--python", "3.12", "--frozen", "--offline", "--dry-run",
        "--preview-features", "package-conflicts",
    ]), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    Would use project environment at: .venv
    error: Package `app` and package `shared` are incompatible with the declared conflicts: {app, shared}
    ");
    Ok(())
}

#[test]
fn explicit_roots_excluded_by_supported_environments() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let scenario = toml::from_str::<Scenario>(indoc! {r#"
        name = "explicit-root-supported-environments"
        [root]
        [expected]
        satisfiable = true
        [packages.leaf.versions."1.0.0"]
        sdist = false
    "#})?;
    let server = PackseServer::from_scenario(&scenario);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv]
        environments = ["python_version >= '3.13'"]
        [tool.uv.workspace]
        members = ["root-a", "root-b"]
        roots = ["root-a", "root-b"]
    "#})?;
    context
        .temp_dir
        .child("root-a/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "root-a"
        version = "0.1.0"
        requires-python = "==3.12.*"
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("root-b/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "root-b"
        version = "0.1.0"
        requires-python = "==3.13.*"
        dependencies = ["leaf"]
        [tool.uv]
        package = false
    "#})?;
    context
        .lock()
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();
    let locked = context.read("uv.lock");
    uv_snapshot!(context.filters(), context.sync().args(["--frozen", "--package", "root-b"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + leaf==1.0.0
    ");
    fs_err::remove_dir_all(context.cache_dir.path())?;
    uv_snapshot!(context.filters(), context.lock().args(["--locked", "--offline", "--index-url"]).arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    assert_eq!(context.read("uv.lock"), locked);
    Ok(())
}

/// A selected non-root only needs coverage over Python versions included in the lock.
#[cfg(feature = "test-universal")]
#[test]
fn explicit_roots_export_preserves_python_gaps() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv.workspace]
        members = ["root-a", "root-b", "shared"]
        roots = ["root-a", "root-b"]
        [tool.uv.sources]
        shared = { workspace = true }
    "#})?;
    context
        .temp_dir
        .child("root-a/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "root-a"
        version = "0.1.0"
        requires-python = "==3.12.*"
        dependencies = ["shared"]
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("root-b/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "root-b"
        version = "0.1.0"
        requires-python = "==3.14.*"
        dependencies = ["shared"]
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("shared/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "shared"
        version = "0.1.0"
        requires-python = ">=3.12"
        [build-system]
        requires = []
        build-backend = "uv_build"
    "#})?;
    context
        .temp_dir
        .child("shared/src/shared/__init__.py")
        .touch()?;
    uv_snapshot!(context.filters(), context.lock().args(["--offline", "--no-index"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--offline", "--package", "shared", "--format", "pylock.toml", "--no-header",
    ]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    lock-version = "1.0"
    created-by = "uv"
    requires-python = ">=3.12, !=3.13.*, <3.15"

    [[packages]]
    name = "shared"
    directory = { path = "shared", editable = true }
    "#);
    Ok(())
}
