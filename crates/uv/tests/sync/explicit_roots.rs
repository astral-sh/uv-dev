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
      cause: Because the requested Python version (>=3.12) does not satisfy Python>=3.13 and shared==0.1.0 depends on Python>=3.13, we can conclude that shared==0.1.0 cannot be used.
             And because only shared==0.1.0 is available and your project depends on shared, we can conclude that your project's requirements are unsatisfiable.
             And because only app==0.1.0 is available and your project requires app, we can conclude that your project's requirements are unsatisfiable.

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
             And because your project depends on shared{python_full_version >= '3.13'}, we can conclude that your project's requirements are unsatisfiable.
             And because only app==0.1.0 is available and your project requires app, we can conclude that your project's requirements are unsatisfiable.

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
