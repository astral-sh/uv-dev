use std::collections::BTreeMap;

use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::{formatdoc, indoc};
use insta::allow_duplicates;
use url::Url;

use uv_static::EnvVars;
use uv_test::packse::generate_wheel;
use uv_test::{TestContext, uv_snapshot};

fn workspace() -> Result<TestContext> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv.workspace]
        members = ["packages/*"]

        [tool.uv.sources]
        shared = { workspace = true }
        leaf = { workspace = true }
        platform-extra = { workspace = true }
        tooling = { workspace = true }
    "#})?;
    context
        .temp_dir
        .child("packages/application/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "application"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["shared"]

        [project.optional-dependencies]
        platform = ["platform-extra; sys_platform == 'win32'"]

        [dependency-groups]
        lint = ["tooling"]

        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("packages/shared/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "shared"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["leaf"]

        [tool.uv]
        package = false
    "#})?;
    for name in ["leaf", "platform-extra", "tooling", "unrelated"] {
        context
            .temp_dir
            .child(format!("packages/{name}/pyproject.toml"))
            .write_str(&formatdoc! {r#"
            [project]
            name = "{name}"
            version = "0.1.0"
            requires-python = ">=3.12"

            [tool.uv]
            package = false
        "#})?;
    }
    context.lock().arg("--offline").assert().success();
    Ok(context)
}

#[test]
fn ignores_unrelated_members() -> Result<()> {
    let context = workspace()?;
    let lock = context.read("uv.lock");
    let unrelated = context.temp_dir.child("packages/unrelated/pyproject.toml");
    unrelated.write_str(
        &context
            .read("packages/unrelated/pyproject.toml")
            .replace("0.1.0", "0.2.0"),
    )?;

    // Neither another member's metadata nor an added member changes this closure.
    context
        .temp_dir
        .child("packages/added/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "added"
        version = "0.1.0"
        requires-python = ">=3.12"
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .arg("--offline")
        .arg("--check-package").arg("application")
        .arg("--check-package").arg("leaf"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context.lock()
        .arg("--offline").arg("--check-package").arg("unrelated"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: The lockfile at `uv.lock` needs to be updated for `unrelated`, but `--check-package` was provided.

    hint: To update the lockfile, run `uv lock`.
    ");
    uv_snapshot!(context.filters(), context.lock()
        .arg("--offline").arg("--check-package").arg("added"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: The lockfile at `uv.lock` needs to be updated for `added`, but `--check-package` was provided.

    hint: To update the lockfile, run `uv lock`.
    ");
    uv_snapshot!(context.filters(), context.lock()
        .arg("--offline").arg("--check"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 7 packages in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--check` was provided.

    hint: To update the lockfile, run `uv lock`.
    ");
    assert_eq!(context.read("uv.lock"), lock);

    Ok(())
}

#[test]
fn ignores_incompatible_sibling_requirements() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv.workspace]
        members = ["packages/*"]
    "#})?;
    for name in ["first", "second"] {
        context
            .temp_dir
            .child(format!("packages/{name}/pyproject.toml"))
            .write_str(&formatdoc! {r#"
            [project]
            name = "{name}"
            version = "0.1.0"
            requires-python = ">=3.12"
            dependencies = ["iniconfig==2.0.0"]
        "#})?;
    }
    context.lock().assert().success();
    let lock = context.read("uv.lock");
    let path = "packages/second/pyproject.toml";
    context
        .temp_dir
        .child(path)
        .write_str(&context.read(path).replace("==2.0.0", "==1.1.1"))?;

    uv_snapshot!(context.filters(), context.lock()
        .arg("--offline").arg("--check-package").arg("first"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.lock()
        .arg("--offline").arg("--check-package").arg("second"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: The lockfile at `uv.lock` needs to be updated for `second`, but `--check-package` was provided.

    hint: To update the lockfile, run `uv lock`.
    ");
    uv_snapshot!(context.filters(), context.lock()
        .arg("--offline").arg("--check-package").arg("first")
        .arg("--check-package").arg("second"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: The lockfile at `uv.lock` needs to be updated for `first` and `second`, but `--check-package` was provided.

    hint: To update the lockfile, run `uv lock`.
    ");
    assert_eq!(context.read("uv.lock"), lock);
    Ok(())
}

#[test]
fn checks_transitive_dependencies_extras_and_groups() -> Result<()> {
    let context = workspace()?;
    let lock = context.read("uv.lock");
    for name in ["leaf", "platform-extra", "tooling"] {
        let path = format!("packages/{name}/pyproject.toml");
        let original = context.read(&path);
        context
            .temp_dir
            .child(&path)
            .write_str(&original.replace("0.1.0", "0.2.0"))?;

        allow_duplicates! {
            uv_snapshot!(context.filters(), context.lock()
                .arg("--offline").arg("--check-package").arg("application"), @"
            exit_code: 1 (failure)
            ----- stderr -----
            error: The lockfile at `uv.lock` needs to be updated for `application`, but `--check-package` was provided.

            hint: To update the lockfile, run `uv lock`.
            ");
            uv_snapshot!(context.filters(), context.lock()
                .arg("--offline").arg("--check-package").arg("unrelated"), @"
            exit_code: 0 (success)
            ----- stderr -----
            Resolved 6 packages in [TIME]
            ");
        }
        assert_eq!(context.read("uv.lock"), lock);
        context.temp_dir.child(&path).write_str(&original)?;
    }
    Ok(())
}

#[test]
fn checks_new_dependency_edges() -> Result<()> {
    let context = workspace()?;
    let lock = context.read("uv.lock");
    let path = "packages/application/pyproject.toml";
    // The new dependency already exists in the lock, but is absent from the selected graph.
    context
        .temp_dir
        .child(path)
        .write_str(&context.read(path).replace(
            "dependencies = [\"shared\"]",
            "dependencies = [\"shared\", \"tooling\"]",
        ))?;
    uv_snapshot!(context.filters(), context.lock()
        .arg("--offline").arg("--check-package").arg("application"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: The lockfile at `uv.lock` needs to be updated for `application`, but `--check-package` was provided.

    hint: To update the lockfile, run `uv lock`.
    ");
    assert_eq!(context.read("uv.lock"), lock);
    Ok(())
}

#[test]
fn checks_workspace_policy() -> Result<()> {
    let context = workspace()?;
    let lock = context.read("uv.lock");
    let original = context.read("pyproject.toml");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&format!(
            "{original}\n[tool.uv]\nconstraint-dependencies = [\"anyio<5\"]\n"
        ))?;
    uv_snapshot!(context.filters(), context.lock()
        .arg("--offline").arg("--check-package").arg("application"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: The lockfile at `uv.lock` needs to be updated for `application`, but `--check-package` was provided.

    hint: To update the lockfile, run `uv lock`.
    ");
    assert_eq!(context.read("uv.lock"), lock);
    Ok(())
}

#[test]
fn preserves_shared_explicit_index_assignments() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let links = context.temp_dir.child("links");
    links.create_dir_all()?;
    let (filename, wheel) = generate_wheel(
        &"shared-dep".parse()?,
        &"3.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
    );
    links.child(filename).write_binary(&wheel)?;
    let index = Url::from_directory_path(links.path())
        .map_err(|()| anyhow::anyhow!("could not convert index path to URL"))?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv.workspace]
        members = ["packages/*"]

        [[tool.uv.index]]
        name = "default"
        url = "https://pypi.org/simple"
        default = true
    "#})?;
    context
        .temp_dir
        .child("packages/web/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "web"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["shared-dep"]
    "#})?;
    let worker = formatdoc! {r#"
        [project]
        name = "worker"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["shared-dep"]

        [tool.uv.sources]
        shared-dep = {{ index = "local" }}

        [[tool.uv.index]]
        name = "local"
        url = "{index}"
        format = "flat"
        explicit = true
    "#};
    let worker_path = context.temp_dir.child("packages/worker/pyproject.toml");
    worker_path.write_str(&worker)?;
    context.lock().arg("--offline").assert().success();
    uv_snapshot!(context.filters(), context.lock()
        .arg("--offline").arg("--check-package").arg("web"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");

    // The assignment must also be recoverable from current declarations when package metadata is
    // omitted from the committed lock.
    let mut lock = context.read("uv.lock").parse::<toml_edit::DocumentMut>()?;
    let Some(packages) = lock["package"].as_array_of_tables_mut() else {
        anyhow::bail!("lockfile did not contain a package array");
    };
    for package in packages.iter_mut() {
        package.remove("metadata");
    }
    lock["revision"] = toml_edit::value(4);
    let lock = lock.to_string();
    context.temp_dir.child("uv.lock").write_str(&lock)?;
    worker_path.write_str(&worker.replace("[\"shared-dep\"]", "[\"shared-dep>=4\"]"))?;
    uv_snapshot!(context.filters(), context.lock()
        .arg("--offline").arg("--preview-features").arg("lock-without-metadata")
        .arg("--check-package").arg("web"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.lock()
        .arg("--offline").arg("--preview-features").arg("lock-without-metadata")
        .arg("--check-package").arg("worker"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: The lockfile at `uv.lock` needs to be updated for `worker`, but `--check-package` was provided.

    hint: To update the lockfile, run `uv lock`.
    ");

    // Keeping the index definition is not sufficient when the relevant assignment is removed.
    worker_path.write_str(&worker.replace(
        "shared-dep = { index = \"local\" }",
        "other-dep = { index = \"local\" }",
    ))?;
    uv_snapshot!(context.filters(), context.lock()
        .arg("--offline").arg("--preview-features").arg("lock-without-metadata")
        .arg("--check-package").arg("web"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: The lockfile at `uv.lock` needs to be updated for `web`, but `--check-package` was provided.

    hint: To update the lockfile, run `uv lock`.
    ");
    assert_eq!(context.read("uv.lock"), lock);

    // A member inside the selected closure may also provide the shared index. The result must not
    // depend on whether that member is visited before the registry package.
    worker_path.write_str(&worker)?;
    context
        .temp_dir
        .child("packages/web/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "web"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["shared-dep", "worker"]

        [tool.uv.sources]
        worker = { workspace = true }
    "#})?;
    context.lock().arg("--offline").assert().success();
    uv_snapshot!(context.filters(), context.lock()
        .arg("--offline").arg("--check-package").arg("web"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    Ok(())
}

#[test]
fn checks_lock_without_package_metadata() -> Result<()> {
    let context = workspace()?;
    let mut lock = context.read("uv.lock").parse::<toml_edit::DocumentMut>()?;
    let Some(packages) = lock["package"].as_array_of_tables_mut() else {
        anyhow::bail!("lockfile did not contain a package array");
    };
    for package in packages.iter_mut() {
        package.remove("metadata");
    }
    lock["revision"] = toml_edit::value(4);
    let lock = lock.to_string();
    context.temp_dir.child("uv.lock").write_str(&lock)?;

    let path = "packages/unrelated/pyproject.toml";
    context
        .temp_dir
        .child(path)
        .write_str(&context.read(path).replace("0.1.0", "0.2.0"))?;
    uv_snapshot!(context.filters(), context.lock()
        .arg("--offline").arg("--preview-features").arg("lock-without-metadata")
        .arg("--check-package").arg("application"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    ");

    let path = "packages/application/pyproject.toml";
    context
        .temp_dir
        .child(path)
        .write_str(&context.read(path).replace(
            "dependencies = [\"shared\"]",
            "dependencies = [\"shared\", \"tooling\"]",
        ))?;
    uv_snapshot!(context.filters(), context.lock()
        .arg("--offline").arg("--preview-features").arg("lock-without-metadata")
        .arg("--check-package").arg("application"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: The lockfile at `uv.lock` needs to be updated for `application`, but `--check-package` was provided.

    hint: To update the lockfile, run `uv lock`.
    ");
    assert_eq!(context.read("uv.lock"), lock);
    Ok(())
}

#[test]
fn rejects_missing_members_and_locks() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .arg("--offline").arg("--check-package").arg("missing"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Package `missing` not found in workspace
    ");
    uv_snapshot!(context.filters(), context.lock()
        .arg("--offline").arg("--check-package").arg("project"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Unable to find lockfile at `uv.lock`, but `--check-package` was provided. To create a lockfile, run `uv lock` or `uv sync` without the flag.
    ");
    assert!(!context.temp_dir.child("uv.lock").exists());
    Ok(())
}

#[test]
fn check_package_overrides_frozen_environment() -> Result<()> {
    let context = workspace()?;
    let lock = context.read("uv.lock");
    uv_snapshot!(context.filters(), context.lock()
        .arg("--offline").arg("--check-package").arg("application")
        .env(EnvVars::UV_FROZEN, "1"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Ignoring `UV_FROZEN` because `--check-package` was provided
    Resolved 6 packages in [TIME]
    ");
    assert_eq!(context.read("uv.lock"), lock);
    Ok(())
}

#[test]
fn rejects_update_options() -> Result<()> {
    let context = workspace()?;
    let lock = context.read("uv.lock");
    uv_snapshot!(context.filters(), context.lock()
        .arg("--check-package").arg("application")
        .arg("--upgrade-package").arg("leaf"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the argument '--check-package <PACKAGE>' cannot be used with '--upgrade-package <UPGRADE_PACKAGE>'

    Usage: uv lock --cache-dir [CACHE_DIR] --check-package <PACKAGE> --exclude-newer <EXCLUDE_NEWER>

    For more information, try '--help'.
    ");
    uv_snapshot!(context.filters(), context.lock()
        .arg("--check-package").arg("application")
        .arg("--check-exists"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the argument '--check-package <PACKAGE>' cannot be used with '--check-exists'

    Usage: uv lock --cache-dir [CACHE_DIR] --check-package <PACKAGE> --exclude-newer <EXCLUDE_NEWER>

    For more information, try '--help'.
    ");
    uv_snapshot!(context.filters(), context.lock()
        .arg("--check-package").arg("application")
        .arg("--refresh"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the argument '--check-package <PACKAGE>' cannot be used with '--refresh'

    Usage: uv lock --cache-dir [CACHE_DIR] --check-package <PACKAGE> --exclude-newer <EXCLUDE_NEWER>

    For more information, try '--help'.
    ");
    uv_snapshot!(context.filters(), context.lock()
        .arg("--check-package").arg("application")
        .arg("--script").arg("script.py"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the argument '--check-package <PACKAGE>' cannot be used with '--script <SCRIPT>'

    Usage: uv lock --cache-dir [CACHE_DIR] --check-package <PACKAGE> --exclude-newer <EXCLUDE_NEWER>

    For more information, try '--help'.
    ");
    assert_eq!(context.read("uv.lock"), lock);
    Ok(())
}
