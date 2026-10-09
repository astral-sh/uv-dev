use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::fixture::{ChildPath, FileTouch, FileWriteStr, PathChild};
use indoc::{formatdoc, indoc};
use uv_test::{TestContext, uv_snapshot};

fn member(root: &ChildPath, name: &str, requires_python: Option<&str>) -> Result<()> {
    let requires_python = requires_python
        .map(|value| format!("requires-python = {value:?}"))
        .unwrap_or_default();
    root.child("pyproject.toml").write_str(&formatdoc! {r#"
        [project]
        name = "{name}"
        version = "0.1.0"
        {requires_python}

        [tool.uv]
        package = false
    "#})?;
    Ok(())
}

fn roots(context: &TestContext, first: &str, second: Option<&str>) -> Result<()> {
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv.workspace]
        members = ["root-a", "root-b"]
        roots = ["root-a", "root-b"]
    "#})?;
    member(&context.temp_dir.child("root-a"), "root-a", Some(first))?;
    member(&context.temp_dir.child("root-b"), "root-b", second)?;
    Ok(())
}

#[test]
fn explicit_roots_metadata_sync_intersects_python() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"]);
    roots(&context, ">=3.12", Some(">=3.13"))?;
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
    roots(&context, ">=3.12", Some(">=3.13"))?;
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
    roots(&context, ">=3.13", None)?;
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
    member(&root_b, "root-b", Some(">=3.12"))?;
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
    member(&context.temp_dir.child("unused"), "unused", Some(">=3.12"))?;
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
    member(&context.temp_dir.child("new"), "new", Some(">=3.12"))?;
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
