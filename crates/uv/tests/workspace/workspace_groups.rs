use std::collections::BTreeMap;
use std::str::FromStr;

use anyhow::{Result, anyhow};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::assert::PathAssert;
use assert_fs::fixture::{FileWriteBin, FileWriteStr, PathChild, PathCreateDir};
use indoc::{formatdoc, indoc};
use predicates::prelude::predicate;
use sha2::{Digest, Sha256};
use url::Url;
use uv_fs::PythonExt;
use uv_pep508::MarkerTree;
use uv_test::packse::scenario::Scenario;
use uv_test::packse::{PackseServer, generate_wheel_with_files};
use uv_test::{TestContext, uv_snapshot};

use super::workspace_metadata::write_wheel_with_metadata;

fn workspace(context: &TestContext) -> Result<()> {
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv]
        no-index = true
        find-links = ["wheels"]

        [tool.uv.workspace]
        members = ["members/*"]

        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["common", "legacy"]
        requires-python = ">=3.12,<3.13"
        default = true

        [[tool.uv.workspace.groups]]
        name = "next"
        members = ["common", "next"]
        requires-python = ">=3.12,<3.15"

        [tool.uv.sources]
        common = { workspace = true }
    "#})?;
    for (name, requires_python, dependencies) in [
        ("common", ">=3.12,<3.15", r#""common-leaf>=1""#),
        (
            "legacy",
            ">=3.12,<3.13",
            r#""common", "branch-one", "common-leaf<2""#,
        ),
        ("next", ">=3.12,<3.15", r#""common", "branch-two""#),
    ] {
        context
            .temp_dir
            .child(format!("members/{name}/pyproject.toml"))
            .write_str(&formatdoc! {r#"
            [project]
            name = "{name}"
            version = "0.1.0"
            requires-python = "{requires_python}"
            dependencies = [{dependencies}]

            [tool.uv]
            package = false
        "#})?;
    }
    let wheels = context.temp_dir.child("wheels");
    wheels.create_dir_all()?;
    for (name, version, metadata) in [
        ("common-leaf", "1.0.0", ""),
        ("common-leaf", "2.0.0", ""),
        ("shared-leaf", "1.0.0", ""),
        ("shared-leaf", "2.0.0", ""),
        ("branch-one", "1.0.0", "Requires-Dist: shared-leaf<2\n"),
        ("branch-two", "1.0.0", "Requires-Dist: shared-leaf>=2\n"),
    ] {
        let stem = format!("{}-{version}", name.replace('-', "_"));
        write_wheel_with_metadata(
            wheels.child(format!("{stem}-py3-none-any.whl")).path(),
            name,
            version,
            &stem,
            metadata,
            &[],
        )?;
    }
    Ok(())
}

/// Dynamic group metadata verifies known build artifacts before importing them.
#[test]
fn workspace_groups_dynamic_metadata_verifies_build_artifacts() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let sentinel = context.temp_dir.child("payload-executed");
    let marker = sentinel.path().escape_for_python();
    let (filename, trusted) = generate_wheel_with_files(
        &"build-helper".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[("build_helper/payload.py", "pass\n")],
    );
    let (_, replacement) = generate_wheel_with_files(
        &"build-helper".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[(
            "build_helper/payload.py",
            &formatdoc! {r#"
            from pathlib import Path
            Path({marker}).write_text("replacement")
        "#},
        )],
    );
    let trusted_digest = hex::encode(Sha256::digest(&trusted));
    let replacement_digest = hex::encode(Sha256::digest(&replacement));
    let context = context
        .with_filter((trusted_digest.clone(), "[TRUSTED_HASH]"))
        .with_filter((replacement_digest.clone(), "[REPLACEMENT_HASH]"));
    let wheel = context.temp_dir.child("wheels").child(&filename);
    wheel.write_binary(&trusted)?;
    let wheel_url =
        Url::from_file_path(wheel.path()).map_err(|()| anyhow!("wheel path must be absolute"))?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
        [project]
        name = "app"
        version = "1.0.0"
        requires-python = ">=3.12"
        dynamic = ["dependencies"]
        [build-system]
        requires = ["build-helper @ {wheel_url}"]
        build-backend = "backend"
        backend-path = ["."]

        [tool.uv.workspace]
        members = []
        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]
        default = true
    "#})?;
    context
        .temp_dir
        .child("backend.py")
        .write_str(&formatdoc! {r#"
        from pathlib import Path

        def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
            import build_helper.payload
            dist_info = Path(metadata_directory) / "app-1.0.0.dist-info"
            dist_info.mkdir()
            (dist_info / "METADATA").write_text(
                "Metadata-Version: 2.3\n"
                "Name: app\n"
                "Version: 1.0.0\n"
                "Requires-Python: >=3.12\n"
                "Requires-Dist: build-helper @ {wheel_url}\n"
            )
            return dist_info.name

        prepare_metadata_for_build_editable = prepare_metadata_for_build_wheel
    "#})?;
    uv_snapshot!(context.filters(), context.lock().args(["--offline", "--no-cache", "--python", "3.12"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    let locked = context.read("uv.lock");
    sentinel.assert(predicate::path::missing());
    wheel.write_binary(&replacement)?;

    let unformatted = locked.replacen("version = ", "version=", 1);
    assert_ne!(unformatted, locked);
    context.temp_dir.child("uv.lock").write_str(&unformatted)?;
    uv_snapshot!(context.filters(), context.lock().args([
        "--offline", "--no-cache", "--python", "3.12", "--locked",
        "--preview-features", "lockfile-format-check",
    ]), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: The lockfile at `uv.lock` has non-canonical formatting at line 1, but `--locked` was provided.

    hint: To regenerate the lockfile, run `uv lock --refresh --preview-features lockfile-format-check`.
    ");
    sentinel.assert(predicate::path::missing());
    context.temp_dir.child("uv.lock").write_str(&locked)?;

    uv_snapshot!(context.filters(), context.lock().args(["--offline", "--no-cache", "--python", "3.12", "--locked"]), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to build `app @ file://[TEMP_DIR]/`
      cause: Failed to install requirements from `build-system.requires`
      cause: Failed to read `build-helper @ file://[TEMP_DIR]/wheels/build_helper-1.0.0-py3-none-any.whl`
      cause: Hash mismatch for `build-helper @ file://[TEMP_DIR]/wheels/build_helper-1.0.0-py3-none-any.whl`

             Expected:
               sha256:[TRUSTED_HASH]

             Computed:
               sha256:[REPLACEMENT_HASH]
    ");
    sentinel.assert(predicate::path::missing());
    assert_eq!(context.read("uv.lock"), locked);

    uv_snapshot!(context.filters(), context.lock().args(["--offline", "--no-cache", "--python", "3.12"]), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to build `app @ file://[TEMP_DIR]/`
      cause: Failed to install requirements from `build-system.requires`
      cause: Failed to read `build-helper @ file://[TEMP_DIR]/wheels/build_helper-1.0.0-py3-none-any.whl`
      cause: Hash mismatch for `build-helper @ file://[TEMP_DIR]/wheels/build_helper-1.0.0-py3-none-any.whl`

             Expected:
               sha256:[TRUSTED_HASH]

             Computed:
               sha256:[REPLACEMENT_HASH]
    ");
    sentinel.assert(predicate::path::missing());
    assert_eq!(context.read("uv.lock"), locked);

    uv_snapshot!(context.filters(), context.lock().args([
        "--offline", "--no-cache", "--python", "3.12", "--upgrade-package", "build-helper",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    assert_eq!(context.read("payload-executed"), "replacement");
    Ok(())
}

/// Metadata probes use current build constraints when the build helper is absent from the lock.
#[test]
fn workspace_groups_dynamic_metadata_uses_current_build_constraints() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let sentinel = context.temp_dir.child("payload-executed");
    let marker = sentinel.path().escape_for_python();
    let (filename, trusted) = generate_wheel_with_files(
        &"build-helper".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[("build_helper/payload.py", "pass\n")],
    );
    let (_, replacement) = generate_wheel_with_files(
        &"build-helper".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[(
            "build_helper/payload.py",
            &formatdoc! {r#"
            from pathlib import Path
            Path({marker}).write_text("replacement")
        "#},
        )],
    );
    let trusted_digest = hex::encode(Sha256::digest(&trusted));
    let replacement_digest = hex::encode(Sha256::digest(&replacement));
    let context = context
        .with_filter((trusted_digest.clone(), "[TRUSTED_HASH]"))
        .with_filter((replacement_digest.clone(), "[REPLACEMENT_HASH]"));
    let wheel = context.temp_dir.child("wheels").child(&filename);
    wheel.write_binary(&trusted)?;
    let wheel_url =
        Url::from_file_path(wheel.path()).map_err(|()| anyhow!("wheel path must be absolute"))?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
        [project]
        name = "app"
        version = "1.0.0"
        requires-python = ">=3.12"
        dynamic = ["dependencies"]
        [build-system]
        requires = ["build-helper @ {wheel_url}"]
        build-backend = "backend"
        backend-path = ["."]
        [tool.uv]
        build-constraint-dependencies = [
            {{ requirement = "build-helper==1.0.0", hashes = ["sha256:{trusted_digest}"] }},
        ]
        [tool.uv.workspace]
        members = []
        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]
        default = true
    "#})?;
    context
        .temp_dir
        .child("backend.py")
        .write_str(&formatdoc! {r#"
        from pathlib import Path

        def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
            import build_helper.payload
            dist_info = Path(metadata_directory) / "app-1.0.0.dist-info"
            dist_info.mkdir()
            (dist_info / "METADATA").write_text(
                "Metadata-Version: 2.3\n"
                "Name: app\n"
                "Version: 1.0.0\n"
                "Requires-Python: >=3.12\n"

            )
            return dist_info.name

        prepare_metadata_for_build_editable = prepare_metadata_for_build_wheel
    "#})?;
    uv_snapshot!(context.filters(), context.lock().args(["--offline", "--no-cache", "--python", "3.12"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    sentinel.assert(predicate::path::missing());
    wheel.write_binary(&replacement)?;
    let pyproject = context.read("pyproject.toml");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&pyproject.replace(&trusted_digest, &replacement_digest))?;

    uv_snapshot!(context.filters(), context.lock().args(["--offline", "--no-cache", "--python", "3.12"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    assert_eq!(context.read("payload-executed"), "replacement");
    Ok(())
}

#[test]
fn workspace_groups_inferred_extra_and_group_splits() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let scenario = toml::from_str::<Scenario>(indoc! {r#"
        name = "workspace-group-extra-group-splits"
        [root]
        [expected]
        satisfiable = true
        [packages.shared-leaf.versions."1.0.0"]
        sdist = false
        [packages.shared-leaf.versions."2.0.0"]
        sdist = false
        [packages.extra-leaf.versions."1.0.0"]
        sdist = false
        [packages.extra-leaf.versions."2.0.0"]
        sdist = false
        [packages.group-leaf.versions."1.0.0"]
        sdist = false
        [packages.group-leaf.versions."2.0.0"]
        sdist = false
    "#})?;
    let server = PackseServer::from_scenario(&scenario);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv]
        conflicts = [
            [{ package = "root-a", extra = "legacy" }, { package = "root-a", extra = "modern" }],
            [{ package = "root-a", group = "legacy" }, { package = "root-a", group = "modern" }],
        ]
        [tool.uv.workspace]
        members = ["members/*"]
        [[tool.uv.workspace.groups]]
        name = "a"
        members = ["root-a"]
        [[tool.uv.workspace.groups]]
        name = "b"
        members = ["root-b"]
    "#})?;
    context
        .temp_dir
        .child("members/root-a/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "root-a"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["shared-leaf<2"]
        [project.optional-dependencies]
        legacy = ["extra-leaf<2"]
        modern = ["extra-leaf>=2"]
        [dependency-groups]
        legacy = ["group-leaf<2"]
        modern = ["group-leaf>=2"]
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("members/root-b/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "root-b"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["shared-leaf>=2"]
        [tool.uv]
        package = false
    "#})?;
    uv_snapshot!(context.filters(), context.lock().arg("--index-url").arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    ");
    let lock = context.read("uv.lock");
    uv_snapshot!(context.filters(), context.lock().arg("--locked").arg("--index-url").arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    ");
    assert_eq!(lock, context.read("uv.lock"));
    uv_snapshot!(context.filters(), context.export()
        .args(["--frozen", "--workspace-group", "a", "--extra", "legacy", "--group", "modern",
            "--no-header", "--no-hashes", "--no-annotate"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    extra-leaf==1.0.0
    group-leaf==2.0.0
    shared-leaf==1.0.0
    ");
    uv_snapshot!(context.filters(), context.export()
        .args(["--frozen", "--workspace-group", "a", "--extra", "modern", "--group", "legacy",
            "--no-header", "--no-hashes", "--no-annotate"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    extra-leaf==2.0.0
    group-leaf==1.0.0
    shared-leaf==1.0.0
    ");
    uv_snapshot!(context.filters(), context.export()
        .args(["--frozen", "--workspace-group", "b", "--no-header", "--no-hashes", "--no-annotate"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    shared-leaf==2.0.0
    ");
    Ok(())
}

#[test]
fn workspace_groups_explicit_project_conflicts() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let scenario = toml::from_str::<Scenario>(indoc! {r#"
        name = "workspace-group-extra-group-splits"
        [root]
        [expected]
        satisfiable = true
        [packages.shared-leaf.versions."1.0.0"]
        sdist = false
        [packages.shared-leaf.versions."2.0.0"]
        sdist = false
        [packages.extra-leaf.versions."1.0.0"]
        sdist = false
        [packages.extra-leaf.versions."2.0.0"]
        sdist = false
        [packages.group-leaf.versions."1.0.0"]
        sdist = false
        [packages.group-leaf.versions."2.0.0"]
        sdist = false
    "#})?;
    let server = PackseServer::from_scenario(&scenario);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv]
        conflicts = [
            [{ package = "root-a", extra = "legacy" }, { package = "root-a", extra = "modern" }],
            [{ package = "root-a", group = "legacy" }, { package = "root-a", group = "modern" }],
            [{ package = "root-a" }, { package = "root-b" }],
        ]
        [tool.uv.workspace]
        members = ["members/*"]
        [[tool.uv.workspace.groups]]
        name = "apps"
        members = ["root-a", "root-b"]
    "#})?;
    context
        .temp_dir
        .child("members/root-a/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "root-a"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["shared-leaf<2"]
        [project.optional-dependencies]
        legacy = ["extra-leaf<2"]
        modern = ["extra-leaf>=2"]
        [dependency-groups]
        legacy = ["group-leaf<2"]
        modern = ["group-leaf>=2"]
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("members/root-b/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "root-b"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["shared-leaf>=2"]
        [tool.uv]
        package = false
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--preview-features", "package-conflicts", "--index-url"]).arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    ");
    let lock = context.read("uv.lock");
    uv_snapshot!(context.filters(), context.lock()
        .args(["--preview-features", "package-conflicts", "--locked", "--index-url"]).arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    ");
    assert_eq!(lock, context.read("uv.lock"));
    uv_snapshot!(context.filters(), context.export()
        .args(["--frozen", "--workspace-group", "apps", "--package", "root-a",
            "--extra", "legacy", "--group", "modern", "--no-header", "--no-hashes", "--no-annotate"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    extra-leaf==1.0.0
    group-leaf==2.0.0
    shared-leaf==1.0.0
    ");
    uv_snapshot!(context.filters(), context.export()
        .args(["--frozen", "--workspace-group", "apps", "--package", "root-b",
            "--no-header", "--no-hashes", "--no-annotate"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    shared-leaf==2.0.0
    ");
    // Dependency groups do not require their base project to be installed.
    uv_snapshot!(context.filters(), context.export()
        .args(["--frozen", "--workspace-group", "apps", "--only-group", "modern",
            "--no-header", "--no-hashes", "--no-annotate"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    group-leaf==2.0.0
    ");

    // Project conflict ordering must not change the set of possible forks.
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv]
        conflicts = [
            [{ package = "root-a" }, { package = "root-b" }],
            [{ package = "root-a", extra = "legacy" }, { package = "root-a", extra = "modern" }],
            [{ package = "root-a", group = "legacy" }, { package = "root-a", group = "modern" }],
        ]
        [tool.uv.workspace]
        members = ["members/*"]
        [[tool.uv.workspace.groups]]
        name = "apps"
        members = ["root-a", "root-b"]
    "#})?;
    fs_err::remove_file(context.temp_dir.child("uv.lock"))?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--preview-features", "package-conflicts", "--index-url"]).arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    ");
    Ok(())
}

#[test]
fn workspace_groups_explicit_own_extra_conflict() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let scenario = toml::from_str::<Scenario>(indoc! {r#"
        name = "workspace-group-extra-group-splits"
        [root]
        [expected]
        satisfiable = true
        [packages.shared-leaf.versions."1.0.0"]
        sdist = false
        [packages.shared-leaf.versions."2.0.0"]
        sdist = false
    "#})?;
    let server = PackseServer::from_scenario(&scenario);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
            [tool.uv]
            conflicts = [[{ package = "root-a" }, { package = "root-a", extra = "modern" }]]
            [tool.uv.workspace]
            members = ["members/*"]
            [[tool.uv.workspace.groups]]
            name = "apps"
            members = ["root-a"]
        "#})?;
    context
        .temp_dir
        .child("members/root-a/pyproject.toml")
        .write_str(indoc! {r#"
            [project]
            name = "root-a"
            version = "0.1.0"
            requires-python = ">=3.12"
            dependencies = ["shared-leaf<2"]
            [project.optional-dependencies]
            modern = ["shared-leaf>=2"]
            [tool.uv]
            package = false
        "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--preview-features", "package-conflicts", "--index-url"])
        .arg(server.index_url()), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to resolve workspace group `apps`
      cause: No solution found when resolving dependencies for split (markers: python_full_version >= '3.12'; included: root-a[modern]; excluded: root-a)
      cause: Because root-a[modern] depends on shared-leaf>=2 and your project depends on shared-leaf<2, we can conclude that your project and root-a[modern] are incompatible.
             And because your project requires root-a[modern], we can conclude that your project's requirements are unsatisfiable.
    ");

    // An explicitly conflicting extra is an alternative root, but still depends on its base.
    context
        .temp_dir
        .child("members/root-a/pyproject.toml")
        .write_str(
            &context
                .read("members/root-a/pyproject.toml")
                .replace("shared-leaf>=2", "shared-leaf<2"),
        )?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--preview-features", "package-conflicts", "--index-url"]).arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    let lock = context.read("uv.lock");
    uv_snapshot!(context.filters(), context.lock()
        .args(["--preview-features", "package-conflicts", "--locked", "--index-url"]).arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    assert_eq!(lock, context.read("uv.lock"));
    uv_snapshot!(context.filters(), context.export()
        .args(["--frozen", "--workspace-group", "apps", "--no-header", "--no-hashes", "--no-annotate"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    shared-leaf==1.0.0
    ");
    uv_snapshot!(context.filters(), context.export()
        .args(["--frozen", "--workspace-group", "apps", "--extra", "modern",
            "--no-header", "--no-hashes", "--no-annotate"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Extra `modern` and package `root-a` are incompatible with the declared conflicts: {`root-a[modern]`, root-a}
    ");
    Ok(())
}

#[test]
fn workspace_groups_lock_export_sync_run() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    workspace(&context)?;
    uv_snapshot!(context.filters(), context.lock().arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    ");
    let original = context.read("uv.lock");
    let lock: toml::Value = toml::from_str(&original)?;
    let packages = lock
        .get("package")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("lock packages missing"))?;
    let packages = packages
        .iter()
        .map(|package| (&package["name"], &package["version"]))
        .collect::<Vec<_>>();
    insta::assert_json_snapshot!(packages, @r#"
    [
      [
        "branch-one",
        "1.0.0"
      ],
      [
        "branch-two",
        "1.0.0"
      ],
      [
        "common",
        "0.1.0"
      ],
      [
        "common-leaf",
        "1.0.0"
      ],
      [
        "legacy",
        "0.1.0"
      ],
      [
        "next",
        "0.1.0"
      ],
      [
        "shared-leaf",
        "1.0.0"
      ],
      [
        "shared-leaf",
        "2.0.0"
      ]
    ]
    "#);
    uv_snapshot!(context.filters(), context.lock().arg("--offline").arg("--check"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    ");
    assert_eq!(original, context.read("uv.lock"));
    uv_snapshot!(context.filters(), context.export().args(["--offline", "--frozen", "--workspace-group", "main", "--no-hashes", "--no-header"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    branch-one==1.0.0
        # via legacy
    common-leaf==1.0.0
        # via
        #   common
        #   legacy
    shared-leaf==1.0.0
        # via branch-one
    ");
    uv_snapshot!(context.filters(), context.export().args(["--offline", "--frozen", "--workspace-group", "next", "--no-hashes", "--no-header"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    branch-two==1.0.0
        # via next
    common-leaf==1.0.0
        # via common
    shared-leaf==2.0.0
        # via branch-two
    ");
    uv_snapshot!(context.filters(), context.sync().args(["--offline", "--frozen", "--workspace-group", "next"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + branch-two==1.0.0
     + common-leaf==1.0.0
     + shared-leaf==2.0.0
    ");
    uv_snapshot!(context.filters(), context.run().args(["--offline", "--frozen", "--workspace-group", "next", "python", "-c", "import importlib.metadata as m; print(m.version('shared-leaf'))"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    2.0.0

    ----- stderr -----
    Checked 3 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.sync().args(["--offline", "--frozen"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 2 packages in [TIME]
    Uninstalled 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + branch-one==1.0.0
     - branch-two==1.0.0
     - shared-leaf==2.0.0
     + shared-leaf==1.0.0
    ");
    uv_snapshot!(context.filters(), context.run().args(["--offline", "--frozen", "python", "-c", "import importlib.metadata as m; print(m.version('shared-leaf'))"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    1.0.0

    ----- stderr -----
    Checked 3 packages in [TIME]
    ");
    Ok(())
}

#[test]
fn workspace_groups_internal_conflict() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    workspace(&context)?;
    context
        .temp_dir
        .child("members/legacy/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "legacy"
        version = "0.1.0"
        requires-python = ">=3.12,<3.13"
        dependencies = ["branch-one", "branch-two"]
        [tool.uv]
        package = false
    "#})?;
    uv_snapshot!(context.filters(), context.lock().arg("--offline"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to resolve workspace group `main`
      cause: No solution found when resolving dependencies for split (markers: python_full_version == '3.12.*')
      cause: Because all versions of branch-two depend on shared-leaf>=2 and all versions of branch-one depend on shared-leaf<2, we can conclude that all versions of branch-one and all versions of branch-two are incompatible.
             And because legacy depends on branch-one and branch-two, we can conclude that legacy's requirements are unsatisfiable.
             And because only legacy==0.1.0 is available and your workspace requires legacy, we can conclude that your workspace's requirements are unsatisfiable.
    ");
    Ok(())
}

#[test]
fn workspace_groups_configuration_errors() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    workspace(&context)?;
    let original = context.read("pyproject.toml");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&original.replace("name = \"next\"", "name = \"main\""))?;
    uv_snapshot!(context.filters(), context.lock().arg("--offline"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Workspace group `main` is defined more than once
    ");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&original.replace("name = \"next\"", "name = \"next\"\ndefault = true"))?;
    uv_snapshot!(context.filters(), context.lock().arg("--offline"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Workspace groups `main` and `next` are both marked as default
    ");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&original.replace(
            "members = [\"common\", \"next\"]",
            "members = [\"missing\"]",
        ))?;
    uv_snapshot!(context.filters(), context.lock().arg("--offline"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Workspace group `next` contains unknown member `missing`
    ");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&original.replace(
            "requires-python = \">=3.12,<3.13\"",
            "requires-python = \">=3.14\"",
        ))?;
    uv_snapshot!(context.filters(), context.lock().arg("--offline"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Workspace group `main` has incompatible `requires-python` declarations
    ");
    Ok(())
}

/// Package selection includes local members reached only through dependency groups.
#[test]
fn workspace_groups_select_dependency_group_member() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv]
        no-index = true
        find-links = ["wheels"]

        [tool.uv.workspace]
        members = ["app", "leaf"]

        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]

        [tool.uv.sources]
        leaf = { workspace = true }
    "#})?;
    context
        .temp_dir
        .child("app/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12,<3.14"

        [dependency-groups]
        test = ["leaf"]

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
        requires-python = ">=3.12,<3.14"
        dependencies = ["leaf-dep==1.0.0"]

        [tool.uv]
        package = false
    "#})?;
    context.temp_dir.child("wheels").create_dir_all()?;
    write_wheel_with_metadata(
        &context
            .temp_dir
            .child("wheels/leaf_dep-1.0.0-py3-none-any.whl"),
        "leaf-dep",
        "1.0.0",
        "leaf_dep-1.0.0",
        "",
        &[],
    )?;
    uv_snapshot!(context.filters(), context.lock().arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--package", "leaf", "--no-header", "--no-hashes",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    leaf-dep==1.0.0
        # via leaf
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--package", "leaf", "--no-header", "--no-hashes",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    leaf-dep==1.0.0
        # via leaf

    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    Ok(())
}

/// Package selection includes local members reached only through an optional extra.
#[test]
fn workspace_groups_select_optional_extra_member() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv]
        no-index = true
        find-links = ["wheels"]

        [tool.uv.workspace]
        members = ["app", "leaf"]

        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]

        [tool.uv.sources]
        leaf = { workspace = true }
    "#})?;
    context
        .temp_dir
        .child("app/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12,<3.14"

        [project.optional-dependencies]
        test = ["leaf"]

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
        requires-python = ">=3.12,<3.14"
        dependencies = ["leaf-dep==1.0.0"]

        [tool.uv]
        package = false
    "#})?;
    context.temp_dir.child("wheels").create_dir_all()?;
    write_wheel_with_metadata(
        &context
            .temp_dir
            .child("wheels/leaf_dep-1.0.0-py3-none-any.whl"),
        "leaf-dep",
        "1.0.0",
        "leaf_dep-1.0.0",
        "",
        &[],
    )?;
    uv_snapshot!(context.filters(), context.lock().arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--package", "leaf", "--no-header", "--no-hashes",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    leaf-dep==1.0.0
        # via leaf
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--package", "leaf", "--no-header", "--no-hashes",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    leaf-dep==1.0.0
        # via leaf

    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    Ok(())
}

#[test]
fn workspace_groups_ordinary_targeting() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    workspace(&context)?;
    context.temp_dir.child("pyproject.toml").write_str(
        &context
            .read("pyproject.toml")
            .replace("default = true\n", ""),
    )?;
    context.lock().arg("--offline").assert().success();
    let next = context
        .export()
        .args([
            "--offline",
            "--frozen",
            "--package",
            "next",
            "--no-header",
            "--no-hashes",
        ])
        .output()?;
    next.clone().assert().success();
    let next = String::from_utf8(next.stdout)?;
    assert!(next.contains("shared-leaf==2.0.0"));
    assert!(!next.contains("branch-one"));
    uv_snapshot!(context.filters(), context.export().args(["--offline", "--frozen", "--package", "common", "--no-header", "--no-hashes"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    common-leaf==1.0.0
        # via common
    ");
    uv_snapshot!(context.filters(), context.export().args(["--offline", "--frozen", "--workspace-group", "main", "--package", "next"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The selected packages are not all reachable in workspace group `main`
    ");

    context
        .temp_dir
        .child("members/common/pyproject.toml")
        .write_str(
            &context
                .read("members/common/pyproject.toml")
                .replace("common-leaf>=1", "common-leaf>=1\", \"shared-leaf>=1"),
        )?;
    context.lock().arg("--offline").assert().success();
    uv_snapshot!(context.filters(), context.export().args(["--offline", "--frozen", "--package", "common"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The lockfile contains multiple workspace contexts; select one with `--workspace-group`
    ");
    context
        .temp_dir
        .child("members/unused/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "unused"
        version = "0.1.0"
        requires-python = ">=4"
        [tool.uv]
        package = false
    "#})?;
    uv_snapshot!(context.filters(), context.export().args(["--offline", "--frozen", "--package", "unused"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Workspace members are not reachable from any workspace group: `unused`
    ");

    context
        .temp_dir
        .child("members/unused-two/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "unused-two"
        version = "0.1.0"
        requires-python = ">=3.12"
        [tool.uv]
        package = false
    "#})?;
    uv_snapshot!(context.filters(), context.export().args(["--offline", "--all-packages"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    error: Workspace members are not reachable from any workspace group: `unused`, `unused-two`
    ");

    // A frozen export only needs the selected graph and root lock metadata.
    fs_err::remove_file(context.temp_dir.child("members/legacy/pyproject.toml"))?;
    context
        .export()
        .args([
            "--offline",
            "--frozen",
            "--workspace-group",
            "next",
            "--no-header",
            "--no-hashes",
        ])
        .assert()
        .success();
    Ok(())
}

/// A Python installation in a patch-level gap cannot satisfy an ordinary shared-member selection.
#[test]
#[cfg(feature = "test-python-patch")]
fn workspace_groups_select_python_outside_patch_gap() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12.9", "3.13"]);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv.workspace]
        members = ["legacy", "modern", "common"]
        [[tool.uv.workspace.groups]]
        name = "legacy"
        members = ["legacy"]
        [[tool.uv.workspace.groups]]
        name = "modern"
        members = ["modern"]
        [tool.uv.sources]
        common = { workspace = true }
    "#})?;
    context
        .temp_dir
        .child("legacy/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "legacy"
        version = "1.0.0"
        requires-python = ">=3.12,<3.12.8"
        dependencies = ["common"]
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("modern/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "modern"
        version = "1.0.0"
        requires-python = ">=3.12.10,<3.14"
        dependencies = ["common"]
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("common/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "common"
        version = "1.0.0"
        requires-python = ">=3.12"
        [tool.uv]
        package = false
    "#})?;
    uv_snapshot!(context.filters(), context.sync().args(["--offline", "--package", "common"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.13.[X] interpreter at: [PYTHON-3.13]
    Creating virtual environment at: .venv
    Resolved 3 packages in [TIME]
    Checked in [TIME]
    ");
    uv_snapshot!(context.filters(), context.run().args([
        "--no-sync", "--package", "common", "python", "-c", "import sys; print(sys.version_info[:2])",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    (3, 13)
    ");
    Ok(())
}

/// A union that cannot fit in the lock's Python declaration fails before interpreter selection.
#[test]
fn workspace_groups_reject_unrepresentable_python_union() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv.workspace]
        members = ["legacy", "modern"]
        [[tool.uv.workspace.groups]]
        name = "legacy"
        members = ["legacy"]
        [[tool.uv.workspace.groups]]
        name = "modern"
        members = ["modern"]
    "#})?;
    context
        .temp_dir
        .child("legacy/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "legacy"
        version = "1.0.0"
        requires-python = ">=3.12,<3.12.3"
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("modern/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "modern"
        version = "1.0.0"
        requires-python = ">=3.13,<3.14"
        [tool.uv]
        package = false
    "#})?;
    uv_snapshot!(context.filters(), context.lock().arg("--offline"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The combined Python domain of workspace groups `legacy`, `modern` cannot be represented by `requires-python`
    ");
    context
        .temp_dir
        .child("uv.lock")
        .assert(predicate::path::missing());
    context
        .temp_dir
        .child(".venv")
        .assert(predicate::path::missing());
    Ok(())
}

#[test]
fn workspace_groups_shared_target_is_unambiguous() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    workspace(&context)?;
    for path in ["pyproject.toml", "members/legacy/pyproject.toml"] {
        context.temp_dir.child(path).write_str(
            &context
                .read(path)
                .replace("default = true\n", "")
                .replace(">=3.12,<3.13", ">=3.12,<3.15"),
        )?;
    }
    context.lock().arg("--offline").assert().success();
    uv_snapshot!(context.filters(), context.export().args(["--offline", "--frozen", "--package", "common", "--no-header", "--no-hashes"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    common-leaf==1.0.0
        # via common
    ");
    Ok(())
}

#[test]
fn workspace_groups_disjoint_target_versions() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    workspace(&context)?;
    context.temp_dir.child("pyproject.toml").write_str(
        &context
            .read("pyproject.toml")
            .replace("default = true\n", "")
            .replace(
                "requires-python = \">=3.12,<3.15\"",
                "requires-python = \">=3.13,<3.15\"",
            ),
    )?;
    context
        .temp_dir
        .child("members/common/pyproject.toml")
        .write_str(
            &context
                .read("members/common/pyproject.toml")
                .replace("common-leaf>=1", "common-leaf>=1\", \"shared-leaf>=1"),
        )?;
    context.lock().arg("--offline").assert().success();
    context
        .lock()
        .args(["--offline", "--check"])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.export().args(["--offline", "--frozen", "--package", "common", "--no-header", "--no-hashes"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    common-leaf==1.0.0 ; python_full_version < '3.13'
        # via common
    common-leaf==2.0.0 ; python_full_version >= '3.13'
        # via common
    shared-leaf==1.0.0 ; python_full_version < '3.13'
        # via common
    shared-leaf==2.0.0 ; python_full_version >= '3.13'
        # via common
    ");
    Ok(())
}

#[test]
fn workspace_groups_compatible_all_packages() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    workspace(&context)?;
    context.temp_dir.child("pyproject.toml").write_str(
        &context
            .read("pyproject.toml")
            .replace("default = true\n", ""),
    )?;
    context
        .temp_dir
        .child("members/next/pyproject.toml")
        .write_str(
            &context
                .read("members/next/pyproject.toml")
                .replace("branch-two", "branch-one"),
        )?;
    context.lock().arg("--offline").assert().success();
    context
        .lock()
        .args(["--offline", "--check"])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.export().args(["--offline", "--frozen", "--all-packages", "--no-header", "--no-hashes"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    branch-one==1.0.0
        # via
        #   legacy
        #   next
    common-leaf==1.0.0
        # via
        #   common
        #   legacy
    shared-leaf==1.0.0
        # via branch-one
    ");
    Ok(())
}

#[test]
fn workspace_groups_inferred_python() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        [tool.uv]
        package = false
        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]
        default = true
    "#})?;
    uv_snapshot!(context.filters(), context.lock().arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: No `requires-python` value found in workspace group `main`. Defaulting to `>=3.12`.
    Resolved 1 package in [TIME]
    ");
    context
        .lock()
        .args(["--offline", "--check"])
        .assert()
        .success();
    let lock: toml::Value = toml::from_str(&context.read("uv.lock"))?;
    assert_eq!(
        lock["workspace-group"][0]["effective-requires-python"].as_str(),
        Some(">=3.12")
    );
    context
        .export()
        .args(["--offline", "--frozen"])
        .assert()
        .success();
    Ok(())
}

#[test]
fn workspace_groups_workspace_dependency_groups() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    workspace(&context)?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [dependency-groups]
        dev = ["common-leaf==1.0.0"]
        [tool.uv]
        no-index = true
        find-links = ["wheels"]
        [tool.uv.workspace]
        members = ["members/*"]
        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]
        default = true
    "#})?;
    context
        .temp_dir
        .child("members/app/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        [tool.uv]
        package = false
    "#})?;
    context.lock().arg("--offline").assert().success();
    context
        .lock()
        .args(["--offline", "--check"])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.export().args(["--offline", "--frozen", "--group", "dev", "--no-header", "--no-hashes"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    common-leaf==1.0.0
    ");
    Ok(())
}

#[test]
fn workspace_groups_no_sources() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    workspace(&context)?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv]
        no-index = true
        find-links = ["wheels"]
        [tool.uv.workspace]
        members = ["members/*"]
        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]
        requires-python = ">=3.12,<3.15"
        default = true
        [tool.uv.sources]
        common-leaf = { workspace = true }
    "#})?;
    for (name, requires_python, dependencies) in [
        ("app", ">=3.12,<3.15", "\"common-leaf==1.0.0\""),
        ("common-leaf", ">=3.12,<3.13", ""),
    ] {
        context
            .temp_dir
            .child(format!("members/{name}/pyproject.toml"))
            .write_str(&formatdoc! {r#"
            [project]
            name = "{name}"
            version = "1.0.0"
            requires-python = "{requires_python}"
            dependencies = [{dependencies}]
            [tool.uv]
            package = false
        "#})?;
    }
    context.lock().arg("--offline").assert().success();
    let lock: toml::Value = toml::from_str(&context.read("uv.lock"))?;
    assert_eq!(
        lock["workspace-group"][0]["effective-requires-python"].as_str(),
        Some("==3.12.*")
    );
    context
        .lock()
        .args(["--offline", "--no-sources"])
        .assert()
        .success();
    context
        .lock()
        .args(["--offline", "--no-sources", "--check"])
        .assert()
        .success();
    let lock: toml::Value = toml::from_str(&context.read("uv.lock"))?;
    assert_eq!(
        lock["workspace-group"][0]["effective-requires-python"].as_str(),
        Some(">=3.12, <3.15")
    );
    uv_snapshot!(context.filters(), context.export().args(["--offline", "--frozen", "--no-sources", "--no-header", "--no-hashes"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the argument '--frozen' cannot be used with '--no-sources'

    Usage: uv export --cache-dir [CACHE_DIR] --offline --frozen --no-header --no-hashes --exclude-newer <EXCLUDE_NEWER>

    For more information, try '--help'.
    ");
    Ok(())
}

#[test]
fn workspace_groups_higher_order_conflict() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv]
        no-index = true
        find-links = ["wheels"]
        [tool.uv.workspace]
        members = ["members/*"]
        [[tool.uv.workspace.groups]]
        name = "one"
        members = ["one"]
        [[tool.uv.workspace.groups]]
        name = "two"
        members = ["two"]
        [[tool.uv.workspace.groups]]
        name = "three"
        members = ["three"]
    "#})?;
    context.temp_dir.child("wheels").create_dir_all()?;
    for (name, excluded) in [("one", 1), ("two", 2), ("three", 3)] {
        context
            .temp_dir
            .child(format!("members/{name}/pyproject.toml"))
            .write_str(&formatdoc! {r#"
            [project]
            name = "{name}"
            version = "0.1.0"
            requires-python = ">=3.12"
            dependencies = ["leaf!={excluded}.0.0"]
            [tool.uv]
            package = false
        "#})?;
        let version = format!("{excluded}.0.0");
        let stem = format!("leaf-{version}");
        write_wheel_with_metadata(
            context
                .temp_dir
                .child(format!("wheels/{stem}-py3-none-any.whl"))
                .path(),
            "leaf",
            &version,
            &stem,
            "",
            &[],
        )?;
    }
    context.lock().arg("--offline").assert().success();
    let original = context.read("uv.lock");
    context
        .lock()
        .args(["--offline", "--check"])
        .assert()
        .success();
    assert_eq!(original, context.read("uv.lock"));
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--workspace-group", "one", "--no-header", "--no-hashes",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    leaf==3.0.0
        # via one
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--workspace-group", "two", "--no-header", "--no-hashes",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    leaf==1.0.0
        # via two
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--workspace-group", "three", "--no-header", "--no-hashes",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    leaf==1.0.0
        # via three
    ");
    Ok(())
}

#[test]
fn workspace_groups_conflicting_sources() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    workspace(&context)?;
    context.temp_dir.child("pyproject.toml").write_str(
        &context
            .read("pyproject.toml")
            .replace(
                "members = [\"common\", \"legacy\"]",
                "members = [\"legacy\"]",
            )
            .replace("members = [\"common\", \"next\"]", "members = [\"next\"]"),
    )?;
    for (name, version) in [("legacy", 1), ("next", 2)] {
        context
            .temp_dir
            .child(format!("members/{name}/pyproject.toml"))
            .write_str(&formatdoc! {r#"
            [project]
            name = "{name}"
            version = "0.1.0"
            requires-python = ">=3.12,<3.15"
            dependencies = ["common-leaf"]
            [tool.uv]
            package = false
            [tool.uv.sources]
            common-leaf = {{ path = "../../wheels/common_leaf-{version}.0.0-py3-none-any.whl" }}
        "#})?;
    }
    context.lock().arg("--offline").assert().success();
    context
        .lock()
        .args(["--offline", "--check"])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--workspace-group", "main", "--no-header", "--no-hashes",
    ]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    ./wheels/common_leaf-1.0.0-py3-none-any.whl
        # via legacy
    "#);
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--workspace-group", "next", "--no-header", "--no-hashes",
    ]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    ./wheels/common_leaf-2.0.0-py3-none-any.whl
        # via next
    "#);
    Ok(())
}

#[test]
fn workspace_groups_conflicting_indexes() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    workspace(&context)?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&format!(
            "{}\n{}",
            context
                .read("pyproject.toml")
                .replace("no-index = true\n", "")
                .replace("find-links = [\"wheels\"]\n", "")
                .replace(
                    "members = [\"common\", \"legacy\"]",
                    "members = [\"legacy\"]"
                )
                .replace("members = [\"common\", \"next\"]", "members = [\"next\"]"),
            indoc! {r#"
        [[tool.uv.index]]
        name = "one"
        url = "indexes/one"
        format = "flat"
        explicit = true
        [[tool.uv.index]]
        name = "two"
        url = "indexes/two"
        format = "flat"
        explicit = true
    "#}
        ))?;
    for (name, index, version) in [("legacy", "one", 1), ("next", "two", 2)] {
        context
            .temp_dir
            .child(format!("indexes/{index}"))
            .create_dir_all()?;
        let wheel = format!("common_leaf-{version}.0.0-py3-none-any.whl");
        fs_err::copy(
            context.temp_dir.child(format!("wheels/{wheel}")),
            context.temp_dir.child(format!("indexes/{index}/{wheel}")),
        )?;
        context
            .temp_dir
            .child(format!("members/{name}/pyproject.toml"))
            .write_str(&formatdoc! {r#"
            [project]
            name = "{name}"
            version = "0.1.0"
            requires-python = ">=3.12,<3.15"
            dependencies = ["common-leaf"]
            [tool.uv]
            package = false
            [tool.uv.sources]
            common-leaf = {{ index = "{index}" }}
        "#})?;
    }
    context.lock().arg("--offline").assert().success();
    context
        .lock()
        .args(["--offline", "--check"])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--workspace-group", "main", "--no-header", "--no-hashes",
    ]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    common-leaf==1.0.0
        # via legacy
    "#);
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--workspace-group", "next", "--no-header", "--no-hashes",
    ]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    common-leaf==2.0.0
        # via next
    "#);
    Ok(())
}

#[test]
fn workspace_groups_conditional_member_python() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    workspace(&context)?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv]
        no-index = true
        find-links = ["wheels"]
        [tool.uv.workspace]
        members = ["members/*"]
        [[tool.uv.workspace.groups]]
        name = "conditional"
        members = ["app"]
        requires-python = ">=3.12,<3.15"
        default = true
        [tool.uv.sources]
        middle = { workspace = true }
        legacy = { workspace = true }
    "#})?;
    for (name, requires_python, dependencies) in [
        (
            "app",
            ">=3.12,<3.15",
            "\"middle; sys_platform == 'win32' or python_version < '3.13'\"",
        ),
        ("middle", ">=3.12,<3.14", "\"legacy\""),
        ("legacy", ">=3.12,<3.13", "\"common-leaf==1.0.0\""),
        ("next", ">=4", ""),
    ] {
        context
            .temp_dir
            .child(format!("members/{name}/pyproject.toml"))
            .write_str(&formatdoc! {r#"
            [project]
            name = "{name}"
            version = "0.1.0"
            requires-python = "{requires_python}"
            dependencies = [{dependencies}]
            [tool.uv]
            package = false
        "#})?;
    }
    context.lock().arg("--offline").assert().success();
    context
        .lock()
        .args(["--offline", "--check"])
        .assert()
        .success();
    let lock: toml::Value = toml::from_str(&context.read("uv.lock"))?;
    assert_eq!(
        lock["workspace-group"][0]["effective-requires-python"].as_str(),
        Some(">=3.12, <3.15")
    );
    let environment = lock["workspace-group"][0]["environment"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing environment"))?;
    assert_eq!(
        MarkerTree::from_str(environment)?,
        MarkerTree::from_str(
            "python_full_version >= '3.12' and python_full_version < '3.15' and (sys_platform != 'win32' or python_full_version < '3.13')"
        )?
    );
    uv_snapshot!(context.filters(), context.export().args(["--offline", "--frozen", "--no-header", "--no-hashes"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    common-leaf==1.0.0 ; python_full_version < '3.13'
        # via legacy
    ");
    context
        .temp_dir
        .child("members/app/pyproject.toml")
        .write_str(
            &context
                .read("members/app/pyproject.toml")
                .replace(
                    "middle; sys_platform == 'win32' or python_version < '3.13'",
                    "middle",
                )
                .replace(">=3.12,<3.15", ">=3.13,<3.15"),
        )?;
    uv_snapshot!(context.filters(), context.lock().arg("--offline"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Workspace group `conditional` has incompatible `requires-python` declarations
    ");
    Ok(())
}

#[test]
fn workspace_groups_url_source_override_member_python() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv.workspace]
        members = ["app", "leaf"]

        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]
        default = true

        [tool.uv.sources]
        leaf = { workspace = true }
    "#})?;
    context
        .temp_dir
        .child("app/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["leaf @ https://example.com/leaf-0.1.0-py3-none-any.whl"]

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
        requires-python = ">=3.13"

        [tool.uv]
        package = false
    "#})?;
    uv_snapshot!(context.filters(), context.lock().args(["--offline", "--no-index"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    let lock: toml::Value = toml::from_str(&context.read("uv.lock"))?;
    assert_eq!(
        lock["workspace-group"][0]["effective-requires-python"].as_str(),
        Some(">=3.13")
    );
    uv_snapshot!(context.filters(), context.lock().args(["--offline", "--no-index", "--check"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    Ok(())
}

#[test]
fn workspace_groups_removed_regenerates_ordinary_lock() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv.workspace]
        members = ["app"]
        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]
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
        .lock()
        .args(["--offline", "--no-index"])
        .assert()
        .success();
    let grouped: toml::Value = toml::from_str(&context.read("uv.lock"))?;
    assert_eq!(grouped["version"].as_integer(), Some(2));
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv.workspace]
        members = ["app"]
    "#})?;
    uv_snapshot!(context.filters(), context.lock().args(["--offline", "--no-index", "--check"]), @r#"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--check` was provided.

    hint: To update the lockfile, run `uv lock`.
    "#);
    uv_snapshot!(context.filters(), context.lock().args(["--offline", "--no-index"]), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    "#);
    let ordinary: toml::Value = toml::from_str(&context.read("uv.lock"))?;
    assert_eq!(ordinary["version"].as_integer(), Some(1));
    assert!(ordinary.get("workspace-group").is_none());
    context
        .lock()
        .args(["--offline", "--no-index", "--locked"])
        .assert()
        .success();
    Ok(())
}

#[test]
fn workspace_groups_lenient_dependencies() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["leaf>=1.9.*"]
        [tool.uv]
        package = false
        no-index = true
        find-links = ["wheels"]
        [tool.uv.workspace]
        members = []
        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]
        default = true
    "#})?;
    context.temp_dir.child("wheels").create_dir_all()?;
    write_wheel_with_metadata(
        context
            .temp_dir
            .child("wheels/leaf-2.0.0-py3-none-any.whl")
            .path(),
        "leaf",
        "2.0.0",
        "leaf-2.0.0",
        "",
        &[],
    )?;
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--no-header", "--no-hashes",
    ]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    leaf==2.0.0
        # via app

    ----- stderr -----
    Resolved 2 packages in [TIME]
    "#);
    context.temp_dir.child("pyproject.toml").write_str(
        &context
            .read("pyproject.toml")
            .replace("leaf>=1.9.*", "leaf>=?"),
    )?;
    uv_snapshot!(context.filters(), context.lock().arg("--offline"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Invalid dependency in workspace group `main` member `app`
      cause: expected version to start with a number, but no leading ASCII digits were found
             leaf>=?
                 ^^^
    ");
    Ok(())
}

#[test]
fn workspace_groups_effective_local_dependencies() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.13,<3.14"
        dependencies = ["excluded", "overridden"]
        [tool.uv]
        package = false
        exclude-dependencies = ["excluded"]
        override-dependencies = ["overridden; python_version < '3.13'"]
        [tool.uv.sources]
        excluded = { workspace = true }
        overridden = { workspace = true }
        [tool.uv.workspace]
        members = ["excluded", "overridden"]
        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]
        default = true
    "#})?;
    context
        .temp_dir
        .child("excluded/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "excluded"
        version = "0.1.0"
        requires-python = ">=3.12,<3.13"
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("overridden/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "overridden"
        version = "0.1.0"
        requires-python = ">=3.12,<3.13"
        [tool.uv]
        package = false
    "#})?;
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--no-index", "--no-header", "--no-hashes",
    ]), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    "#);
    Ok(())
}

#[test]
fn workspace_groups_selected_member_default_groups() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv]
        no-index = true
        find-links = ["wheels"]
        [tool.uv.workspace]
        members = ["app"]
        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]
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
        [dependency-groups]
        test = ["leaf"]
        [tool.uv]
        package = false
        default-groups = ["test"]
    "#})?;
    context.temp_dir.child("wheels").create_dir_all()?;
    write_wheel_with_metadata(
        context
            .temp_dir
            .child("wheels/leaf-1.0.0-py3-none-any.whl")
            .path(),
        "leaf",
        "1.0.0",
        "leaf-1.0.0",
        "",
        &[],
    )?;
    uv_snapshot!(context.filters(), context.sync().arg("--offline"), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + leaf==1.0.0
    "#);
    uv_snapshot!(context.filters(), context.sync().args(["--offline", "--frozen", "--dry-run"]), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Would use project environment at: .venv
    Checked 1 package in [TIME]
    Would make no changes
    "#);
    uv_snapshot!(context.filters(), context.export().args(["--offline", "--no-header", "--no-hashes"]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    leaf==1.0.0

    ----- stderr -----
    Resolved 2 packages in [TIME]
    "#);
    uv_snapshot!(context.filters(), context.export().args(["--offline", "--frozen", "--no-header", "--no-hashes"]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    leaf==1.0.0
    "#);
    uv_snapshot!(context.filters(), context.run().args([
        "--offline", "python", "-c", "from importlib.metadata import version; print(version('leaf'))",
    ]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    1.0.0

    ----- stderr -----
    Resolved 2 packages in [TIME]
    Checked 1 package in [TIME]
    "#);
    Ok(())
}

#[test]
fn workspace_groups_batch_selects_each_context() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    workspace(&context)?;
    context.temp_dir.child("pyproject.toml").write_str(
        &context
            .read("pyproject.toml")
            .replace("default = true", "default = false"),
    )?;
    context.temp_dir.child("batch.toml").write_str(indoc! {r#"
        [[export]]
        output-file = "legacy.txt"
        package = ["legacy"]
        [[export]]
        output-file = "next.txt"
        package = ["next"]
    "#})?;
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--no-header", "--no-hashes", "--batch", "batch.toml",
        "--preview-features", "batch-export",
    ]), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    "#);
    insta::assert_snapshot!(context.read("legacy.txt"), @r#"
    branch-one==1.0.0
        # via legacy
    common-leaf==1.0.0
        # via
        #   common
        #   legacy
    shared-leaf==1.0.0
        # via branch-one
    "#);
    insta::assert_snapshot!(context.read("next.txt"), @r#"
    branch-two==1.0.0
        # via next
    common-leaf==1.0.0
        # via common
    shared-leaf==2.0.0
        # via branch-two
    "#);
    fs_err::remove_file(context.temp_dir.child("pyproject.toml"))?;
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--no-header", "--no-hashes", "--batch", "batch.toml",
        "--preview-features", "batch-export,frozen-lockfile",
    ]), @"exit_code: 0 (success)");
    insta::assert_snapshot!(context.read("legacy.txt"), @r#"
    branch-one==1.0.0
        # via legacy
    common-leaf==1.0.0
        # via
        #   common
        #   legacy
    shared-leaf==1.0.0
        # via branch-one
    "#);
    insta::assert_snapshot!(context.read("next.txt"), @r#"
    branch-two==1.0.0
        # via next
    common-leaf==1.0.0
        # via common
    shared-leaf==2.0.0
        # via branch-two
    "#);
    Ok(())
}

#[test]
fn workspace_groups_batch_all_packages_uses_selected_roots() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    workspace(&context)?;
    context.temp_dir.child("batch.toml").write_str(indoc! {r#"
        [[export]]
        output-file = "all.txt"
        all-packages = true
    "#})?;
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--no-header", "--no-hashes", "--no-annotate", "--batch", "batch.toml",
        "--preview-features", "batch-export",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    ");
    insta::assert_snapshot!(context.read("all.txt"), @"
    branch-one==1.0.0
    common-leaf==1.0.0
    shared-leaf==1.0.0
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--no-header", "--no-hashes", "--no-annotate", "--batch", "batch.toml",
        "--workspace-group", "next", "--preview-features", "batch-export",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    ");
    insta::assert_snapshot!(context.read("all.txt"), @"
    branch-two==1.0.0
    common-leaf==1.0.0
    shared-leaf==2.0.0
    ");
    fs_err::remove_file(context.temp_dir.child("pyproject.toml"))?;
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--no-header", "--no-hashes", "--no-annotate", "--batch", "batch.toml",
        "--preview-features", "batch-export,frozen-lockfile",
    ]), @"exit_code: 0 (success)");
    insta::assert_snapshot!(context.read("all.txt"), @"
    branch-one==1.0.0
    common-leaf==1.0.0
    shared-leaf==1.0.0
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--no-header", "--no-hashes", "--no-annotate", "--batch", "batch.toml",
        "--workspace-group", "next", "--preview-features", "batch-export,frozen-lockfile",
    ]), @"exit_code: 0 (success)");
    insta::assert_snapshot!(context.read("all.txt"), @"
    branch-two==1.0.0
    common-leaf==1.0.0
    shared-leaf==2.0.0
    ");
    Ok(())
}

#[test]
fn workspace_groups_ordinary_python_intersection() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"]);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv.workspace]
        members = ["a", "b"]
        [[tool.uv.workspace.groups]]
        name = "first"
        members = ["a"]
        [[tool.uv.workspace.groups]]
        name = "second"
        members = ["b"]
    "#})?;
    context
        .temp_dir
        .child("a/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "a"
        version = "0.1.0"
        requires-python = ">=3.12,<3.14"
        dependencies = []

        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("b/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "b"
        version = "0.1.0"
        requires-python = ">=3.13,<3.15"
        dependencies = []

        [tool.uv]
        package = false
    "#})?;
    context
        .venv()
        .args(["--clear", "--python", "3.12"])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.sync().args(["--all-packages", "--offline", "--no-index"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.13.[X] interpreter at: [PYTHON-3.13]
    Removed virtual environment at: .venv
    Creating virtual environment at: .venv
    Resolved 2 packages in [TIME]
    Checked in [TIME]
    ");
    uv_snapshot!(context.python_command().arg("-c").arg("import sys; print(sys.version_info[:2])"), @"
    exit_code: 0 (success)
    ----- stdout -----
    (3, 13)
    ");
    Ok(())
}

/// A registry release does not replace the identity of a same-name local workspace member.
#[test]
fn workspace_groups_frozen_local_member_shares_registry_name() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context.temp_dir.child("wheels").create_dir_all()?;
    write_wheel_with_metadata(
        &context.temp_dir.child("wheels/leaf-1.0.0-py3-none-any.whl"),
        "leaf",
        "1.0.0",
        "leaf-1.0.0",
        "",
        &[],
    )?;
    write_wheel_with_metadata(
        &context
            .temp_dir
            .child("wheels/leaf_dep-1.0.0-py3-none-any.whl"),
        "leaf-dep",
        "1.0.0",
        "leaf_dep-1.0.0",
        "",
        &[],
    )?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv.workspace]
        members = ["app", "leaf"]
        [[tool.uv.workspace.groups]]
        name = "legacy"
        members = ["app"]
        requires-python = ">=3.12,<3.13"
        [[tool.uv.workspace.groups]]
        name = "modern"
        members = ["app"]
        requires-python = ">=3.13,<3.14"
        [tool.uv.sources]
        leaf = { workspace = true, marker = "python_version >= '3.13'" }
    "#})?;
    context
        .temp_dir
        .child("app/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["leaf"]
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
        requires-python = ">=3.13"
        dependencies = ["leaf-dep"]
        [tool.uv]
        package = false
    "#})?;
    context
        .lock()
        .args(["--offline", "--no-index", "--find-links", "wheels"])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--workspace-group", "modern", "--package", "leaf",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    leaf-dep==1.0.0
    ");
    fs_err::remove_file(context.temp_dir.child("pyproject.toml"))?;
    fs_err::remove_file(context.temp_dir.child("app/pyproject.toml"))?;
    fs_err::remove_file(context.temp_dir.child("leaf/pyproject.toml"))?;
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--workspace-group", "modern", "--package", "leaf",
        "--preview-features", "frozen-lockfile", "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    leaf-dep==1.0.0
    ");
    Ok(())
}

#[test]
fn workspace_groups_frozen_transitive_member() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv.workspace]
        members = ["app", "common"]
        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]
        [tool.uv.sources]
        common = { workspace = true }
    "#})?;
    context
        .temp_dir
        .child("app/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["common"]

        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("common/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "common"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [tool.uv]
        package = false
    "#})?;
    context
        .lock()
        .args(["--offline", "--no-index"])
        .assert()
        .success();
    fs_err::remove_file(context.temp_dir.child("pyproject.toml"))?;
    fs_err::remove_file(context.temp_dir.child("app/pyproject.toml"))?;
    fs_err::remove_file(context.temp_dir.child("common/pyproject.toml"))?;
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--workspace-group", "main", "--package", "common", "--preview-features", "frozen-lockfile",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"exit_code: 0 (success)");
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--workspace-group", "main", "--package", "common", "--preview-features", "frozen-lockfile", "--offline",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked in [TIME]
    ");
    Ok(())
}

#[test]
fn workspace_groups_retain_later_member_groups() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"]);
    let scenario = toml::from_str::<Scenario>(indoc! {r#"
        name = "workspace-group-member-metadata"
        [root]
        [expected]
        satisfiable = true
        [packages.shared-leaf.versions."1.0.0"]
        sdist = false
        [packages.shared-leaf.versions."2.0.0"]
        sdist = false
        [packages.test-leaf.versions."1.0.0"]
        sdist = false
    "#})?;
    let server = PackseServer::from_scenario(&scenario);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv.workspace]
        members = ["app", "common", "other"]
        [[tool.uv.workspace.groups]]
        name = "first"
        members = ["app"]
        [[tool.uv.workspace.groups]]
        name = "second"
        members = ["common", "other"]
        [tool.uv.sources]
        common = { workspace = true }
    "#})?;
    context
        .temp_dir
        .child("app/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["common", "shared-leaf<2"]

        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("common/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "common"
        version = "0.1.0"
        requires-python = ">=3.12"
        [dependency-groups]
        test = ["test-leaf"]
        [tool.uv]
        package = false
        default-groups = ["test"]
        [tool.uv.dependency-groups]
        test = { requires-python = ">=3.13" }
    "#})?;
    context
        .temp_dir
        .child("other/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "other"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["shared-leaf>=2"]

        [tool.uv]
        package = false
    "#})?;
    context
        .lock()
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();
    context
        .venv()
        .args(["--clear", "--python", "3.12"])
        .assert()
        .success();
    fs_err::remove_file(context.temp_dir.child("pyproject.toml"))?;
    fs_err::remove_file(context.temp_dir.child("app/pyproject.toml"))?;
    fs_err::remove_file(context.temp_dir.child("common/pyproject.toml"))?;
    fs_err::remove_file(context.temp_dir.child("other/pyproject.toml"))?;
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--workspace-group", "second", "--package", "common", "--preview-features", "frozen-lockfile",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.13.[X] interpreter at: [PYTHON-3.13]
    Removed virtual environment at: .venv
    Creating virtual environment at: .venv
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + test-leaf==1.0.0
    ");
    uv_snapshot!(context.python_command().arg("-c").arg("import sys; from importlib.metadata import version; print(sys.version_info[:2]); print(version('test-leaf'))"), @"
    exit_code: 0 (success)
    ----- stdout -----
    (3, 13)
    1.0.0
    ");
    Ok(())
}

#[test]
fn workspace_groups_retain_context_resolution_inputs() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let scenario = toml::from_str::<Scenario>(indoc! {r#"
        name = "workspace-group-retained-inputs"
        [root]
        [expected]
        satisfiable = true
        [packages.shared-leaf.versions."1.0.0"]
        sdist = false
        [packages.shared-leaf.versions."2.0.0"]
        sdist = false
        [packages.only-later.versions."1.0.0"]
        sdist = false
        [packages.later-leaf.versions."1.0.0"]
        sdist = false
    "#})?;
    let server = PackseServer::from_scenario(&scenario);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv]
        exclude-dependencies = ["only-later"]
        [tool.uv.exclude-newer-package]
        later-leaf = "2100-01-01T00:00:00Z"
        [tool.uv.workspace]
        members = ["a", "b"]
        [[tool.uv.workspace.groups]]
        name = "first"
        members = ["a"]
        [[tool.uv.workspace.groups]]
        name = "second"
        members = ["b"]
    "#})?;
    context
        .temp_dir
        .child("a/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "a"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["shared-leaf<2"]

        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("b/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "b"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["shared-leaf>=2", "only-later", "later-leaf"]

        [tool.uv]
        package = false
    "#})?;
    context
        .lock()
        .args(["--preview-features", "resolution-inputs", "--index-url"])
        .arg(server.index_url())
        .assert()
        .success();
    let lock: toml::Value = toml::from_str(&context.read("uv.lock"))?;
    assert_eq!(
        lock.get("options")
            .and_then(|options| options.get("exclude-newer-package"))
            .and_then(|packages| packages.get("later-leaf"))
            .and_then(toml::Value::as_str),
        Some("2100-01-01T00:00:00Z"),
    );
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&context.read("pyproject.toml").replace(
            "exclude-dependencies = [\"only-later\"]",
            "exclude-dependencies = []",
        ))?;
    uv_snapshot!(context.filters(), context.lock().args([
        "--locked", "--preview-features", "resolution-inputs", "--index-url",
    ]).arg(server.index_url()), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

    hint: To update the lockfile, run `uv lock`.
    ");
    Ok(())
}

/// Optional dependencies reached through a member extra constrain the group's Python domain.
#[test]
fn workspace_groups_include_transitive_extra_python() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv.workspace]
        members = ["app", "common", "leaf"]
        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]
        default = true
        [tool.uv.sources]
        common = { workspace = true }
        leaf = { workspace = true }
    "#})?;
    context
        .temp_dir
        .child("app/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["common[feature]"]
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("common/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "common"
        version = "0.1.0"
        requires-python = ">=3.12"
        [project.optional-dependencies]
        feature = ["leaf; extra == 'feature'"]
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
        requires-python = ">=3.13"
        [tool.uv]
        package = false
    "#})?;
    uv_snapshot!(context.filters(), context.lock().args(["--offline", "--no-index"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    let lock: toml::Value = toml::from_str(&context.read("uv.lock"))?;
    insta::assert_snapshot!(lock["workspace-group"][0]["effective-requires-python"].as_str().expect("group records its Python domain"), @">=3.13");
    Ok(())
}

/// A later extra request revisits a member whose production dependencies were already reached.
#[test]
fn workspace_groups_revisit_member_for_requested_extra() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv.workspace]
        members = ["app", "common", "leaf", "bridge"]
        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]
        default = true
        [tool.uv.sources]
        common = { workspace = true }
        leaf = { workspace = true }
        bridge = { workspace = true }
    "#})?;
    context
        .temp_dir
        .child("app/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["bridge", "common"]
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("common/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "common"
        version = "0.1.0"
        requires-python = ">=3.12"
        [project.optional-dependencies]
        feature = ["leaf"]
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
        requires-python = ">=3.13"
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("bridge/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "bridge"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["common[feature]"]
        [tool.uv]
        package = false
    "#})?;
    uv_snapshot!(context.filters(), context.lock().args(["--offline", "--no-index"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");
    let lock: toml::Value = toml::from_str(&context.read("uv.lock"))?;
    insta::assert_snapshot!(lock["workspace-group"][0]["effective-requires-python"].as_str().expect("group records its Python domain"), @">=3.13");
    Ok(())
}

/// Conditional extra activation narrows member reachability only where that extra is requested.
#[test]
fn workspace_groups_preserve_extra_activation_markers() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv.workspace]
        members = ["app", "common", "leaf"]
        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]
        default = true
        [tool.uv.sources]
        common = { workspace = true }
        leaf = { workspace = true }
    "#})?;
    context
        .temp_dir
        .child("app/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["common[feature]; python_version >= '3.13'", "common"]
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("common/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "common"
        version = "0.1.0"
        requires-python = ">=3.12"
        [project.optional-dependencies]
        feature = ["leaf"]
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
        requires-python = ">=3.13"
        [tool.uv]
        package = false
    "#})?;
    uv_snapshot!(context.filters(), context.lock().args(["--offline", "--no-index"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    let lock: toml::Value = toml::from_str(&context.read("uv.lock"))?;
    insta::assert_snapshot!(lock["workspace-group"][0]["effective-requires-python"].as_str().expect("group records its Python domain"), @">=3.12");
    Ok(())
}

/// Recursive self-extras constrain the group's Python domain.
#[test]
fn workspace_groups_include_recursive_self_extra_python() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv.workspace]
        members = ["app", "leaf"]
        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]
        default = true
        [tool.uv.sources]
        leaf = { workspace = true }
    "#})?;
    context
        .temp_dir
        .child("app/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["app[start]"]
        [project.optional-dependencies]
        start = ["app[feature]; extra == 'start'"]
        feature = ["leaf"]
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
        requires-python = ">=3.13"
        [tool.uv]
        package = false
    "#})?;
    uv_snapshot!(context.filters(), context.lock().args(["--offline", "--no-index"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    let lock: toml::Value = toml::from_str(&context.read("uv.lock"))?;
    insta::assert_snapshot!(lock["workspace-group"][0]["effective-requires-python"].as_str().expect("group records its Python domain"), @">=3.13");
    Ok(())
}

/// Recursive self-extra activation retains its conditional Python domain.
#[test]
fn workspace_groups_preserve_recursive_self_extra_marker() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv.workspace]
        members = ["app", "common", "leaf"]
        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]
        default = true
        [tool.uv.sources]
        common = { workspace = true }
        leaf = { workspace = true }
    "#})?;
    context
        .temp_dir
        .child("app/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["common"]
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("common/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "common"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["common[feature]; python_version >= '3.13'"]
        [project.optional-dependencies]
        feature = ["leaf"]
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
        requires-python = ">=3.13"
        [tool.uv]
        package = false
    "#})?;
    uv_snapshot!(context.filters(), context.lock().args(["--offline", "--no-index"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    let lock: toml::Value = toml::from_str(&context.read("uv.lock"))?;
    insta::assert_snapshot!(lock["workspace-group"][0]["effective-requires-python"].as_str().expect("group records its Python domain"), @">=3.12");
    Ok(())
}

/// A missing manifest must not bypass the lockfile's workspace-group consistency checks.
#[test]
fn workspace_groups_frozen_rejects_missing_root() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context.temp_dir.child("uv.lock").write_str(indoc! {r#"
        version = 2
        revision = 5
        requires-python = ">=3.12"
        resolution-markers = ["extra == 'workspace-main'", "extra == 'workspace-next'"]

        [[workspace-group]]
        name = "main"
        members = ["app"]
        effective-requires-python = ">=3.12"
        default = true

        [[workspace-group]]
        name = "next"
        members = ["app"]
        effective-requires-python = ">=3.12"

        [manifest]
        members = ["app"]

        [[package]]
        name = "app"
        version = "0.1.0"
        source = { virtual = "." }
        resolution-markers = ["extra == 'workspace-next'"]
    "#})?;

    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--offline", "--workspace-group", "next", "--preview-features", "frozen-lockfile",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to parse lockfile `[TEMP_DIR]/uv.lock`
      cause: Workspace group `main` contains member `app` with no locked package
    ");
    Ok(())
}

/// Transitive workspace members may build metadata even when only their parent is a group root.
#[test]
fn workspace_groups_no_build_transitive_dynamic_metadata() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["child"]

        [tool.uv.workspace]
        members = ["child"]

        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]
        default = true

        [tool.uv.sources]
        child = { workspace = true, editable = false }
    "#})?;
    context
        .temp_dir
        .child("child/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "child"
        requires-python = ">=3.12"
        dynamic = ["version", "dependencies"]

        [build-system]
        requires = []
        backend-path = ["."]
        build-backend = "build_backend"
    "#})?;
    context
        .temp_dir
        .child("child/build_backend.py")
        .write_str(indoc! {r#"
        import pathlib
        from textwrap import dedent

        def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
            pathlib.Path("metadata-hook-called").write_text("called")
            dist_info = pathlib.Path(metadata_directory, "child-0.1.0.dist-info")
            dist_info.mkdir()
            dist_info.joinpath("METADATA").write_text(dedent("""
                Metadata-Version: 2.1
                Name: child
                Version: 0.1.0
            """).lstrip())
            return dist_info.name
    "#})?;

    uv_snapshot!(context.filters(), context.lock().args(["--no-build", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    insta::assert_snapshot!(context.read("child/metadata-hook-called"), @"called");
    Ok(())
}

/// Editing and tree discovery honor disabled workspace source overrides before choosing Python.
#[test]
fn workspace_groups_no_sources_editing_and_tree() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = PackseServer::from_scenario(&toml::from_str::<Scenario>(indoc! {r#"
        name = "workspace-group-no-sources-discovery"
        [root]
        [expected]
        satisfiable = true
        [packages.leaf.versions."1.0.0"]
        sdist = false
        [packages.extra-leaf.versions."1.0.0"]
        sdist = false
    "#})?);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv.workspace]
        members = ["app", "leaf"]
        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]
        default = true
        [tool.uv.sources]
        leaf = { workspace = true }
    "#})?;
    context
        .temp_dir
        .child("app/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.13"
        dependencies = ["leaf"]
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
        requires-python = "<3.13"
        [tool.uv]
        package = false
    "#})?;

    uv_snapshot!(context.filters(), context.add().args([
        "--package", "app", "--no-sources", "--no-sync", "extra-leaf",
    ]).arg("--default-index").arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.remove().args([
        "--package", "app", "--no-sources", "--no-sync", "extra-leaf",
    ]).arg("--default-index").arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.tree().args([
        "--no-sources", "--locked", "--package", "app",
    ]).arg("--default-index").arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stdout -----
    app v0.1.0
    └── leaf v1.0.0

    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    Ok(())
}

/// A selected transitive member's default groups narrow the named context's interpreter domain.
#[test]
fn workspace_groups_selected_transitive_member_group_python() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"]);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv]
        no-index = true
        find-links = ["wheels"]
        [tool.uv.workspace]
        members = ["app", "common"]
        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]
        default = true
        [[tool.uv.workspace.groups]]
        name = "common"
        members = ["common"]
        [tool.uv.sources]
        common = { workspace = true }
    "#})?;
    context
        .temp_dir
        .child("app/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["common"]
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("common/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "common"
        version = "0.1.0"
        requires-python = ">=3.12"
        [dependency-groups]
        test = ["leaf; python_version >= '3.13'"]
        [tool.uv]
        package = false
        default-groups = ["test"]
        [tool.uv.dependency-groups]
        test = { requires-python = ">=3.13" }
    "#})?;
    context.temp_dir.child("wheels").create_dir_all()?;
    write_wheel_with_metadata(
        context
            .temp_dir
            .child("wheels/leaf-1.0.0-py3-none-any.whl")
            .path(),
        "leaf",
        "1.0.0",
        "leaf-1.0.0",
        "",
        &[],
    )?;
    uv_snapshot!(context.filters(), context.sync().args([
        "--offline", "--workspace-group", "main", "--package", "common",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.13.[X] interpreter at: [PYTHON-3.13]
    Creating virtual environment at: .venv
    Resolved 3 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + leaf==1.0.0
    ");
    uv_snapshot!(context.filters(), context.sync().args([
        "--offline", "--frozen", "--workspace-group", "main", "--package", "common",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 1 package in [TIME]
    ");
    Ok(())
}

/// Root-authored dependency overrides use root sources before replacing member dependencies.
#[test]
fn workspace_groups_overrides_use_workspace_root_sources() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv]
        override-dependencies = ["leaf"]
        [tool.uv.workspace]
        members = ["app", "leaf"]
        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]
        default = true
        [tool.uv.sources]
        leaf = { workspace = true }
    "#})?;
    context
        .temp_dir
        .child("app/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["leaf"]
        [tool.uv]
        package = false
        [tool.uv.sources]
        leaf = { workspace = true, marker = "python_version >= '3.13'" }
    "#})?;
    context
        .temp_dir
        .child("leaf/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "leaf"
        version = "0.1.0"
        requires-python = ">=3.13"
        [tool.uv]
        package = false
    "#})?;
    uv_snapshot!(context.filters(), context.lock().args(["--offline", "--no-index"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    let lock: toml::Value = toml::from_str(&context.read("uv.lock"))?;
    insta::assert_snapshot!(lock["workspace-group"][0]["effective-requires-python"].as_str().expect("group records its Python domain"), @">=3.13");
    Ok(())
}

/// A tree observes conflicting workspace contexts while retaining concrete Python guards.
#[test]
fn workspace_groups_tree_includes_each_context() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context.temp_dir.child("wheels").create_dir_all()?;
    write_wheel_with_metadata(
        &context.temp_dir.child("wheels/leaf-1.0.0-py3-none-any.whl"),
        "leaf",
        "1.0.0",
        "leaf-1.0.0",
        "",
        &[],
    )?;
    write_wheel_with_metadata(
        &context.temp_dir.child("wheels/leaf-2.0.0-py3-none-any.whl"),
        "leaf",
        "2.0.0",
        "leaf-2.0.0",
        "",
        &[],
    )?;
    write_wheel_with_metadata(
        &context
            .temp_dir
            .child("wheels/future-1.0.0-py3-none-any.whl"),
        "future",
        "1.0.0",
        "future-1.0.0",
        "",
        &[],
    )?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv]
        no-index = true
        find-links = ["wheels"]
        [tool.uv.workspace]
        members = ["a", "b"]
        [[tool.uv.workspace.groups]]
        name = "a"
        members = ["a"]
        default = true
        [[tool.uv.workspace.groups]]
        name = "b"
        members = ["b"]
    "#})?;
    context
        .temp_dir
        .child("a/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "a"
        version = "0.1.0"
        requires-python = ">=3.12,<3.14"
        dependencies = [
            "leaf==1.0.0; python_version < '3.13'",
            "future==1.0.0; python_version >= '3.13'",
        ]
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("b/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "b"
        version = "0.1.0"
        requires-python = ">=3.12,<3.14"
        dependencies = ["leaf==2.0.0"]
        [tool.uv]
        package = false
    "#})?;
    uv_snapshot!(context.filters(), context.tree().arg("--offline"), @"
    exit_code: 0 (success)
    ----- stdout -----
    b v0.1.0
    └── leaf v2.0.0
    a v0.1.0
    └── leaf v1.0.0

    ----- stderr -----
    Resolved 5 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.tree().args(["--offline", "--frozen", "--universal"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    b v0.1.0
    └── leaf v2.0.0
    a v0.1.0
    ├── future v1.0.0
    └── leaf v1.0.0
    ");
    Ok(())
}

/// A registry dependency with a member's name does not make that local member reachable.
#[test]
fn workspace_groups_reject_registry_package_as_member() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::from_scenario(&toml::from_str::<Scenario>(indoc! {r#"
        name = "workspace-group-registry-member"
        [root]
        [expected]
        satisfiable = true
        [packages.leaf.versions."1.0.0"]
        sdist = false
    "#})?);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv.workspace]
        members = ["app", "leaf"]
        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]
        [tool.uv.sources]
        leaf = { workspace = true }
    "#})?;
    context
        .temp_dir
        .child("app/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["leaf"]
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
        [tool.uv]
        package = false
    "#})?;
    uv_snapshot!(context.filters(), context.lock().args(["--no-sources", "--index-url"]).arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.sync().args([
        "--frozen", "--dry-run", "--workspace-group", "main", "--package", "leaf",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The selected packages are not all reachable in workspace group `main`
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--frozen", "--package", "leaf", "--no-header", "--no-hashes",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Workspace members are not reachable from any workspace group: `leaf`
    ");
    Ok(())
}

/// A stale-lock preview uses the explicitly selected context before reporting the mismatch.
#[test]
fn workspace_groups_locked_dry_run_uses_selected_context() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context.temp_dir.child("wheels").create_dir_all()?;
    write_wheel_with_metadata(
        &context.temp_dir.child("wheels/leaf-1.0.0-py3-none-any.whl"),
        "leaf",
        "1.0.0",
        "leaf-1.0.0",
        "",
        &[],
    )?;
    write_wheel_with_metadata(
        &context.temp_dir.child("wheels/leaf-2.0.0-py3-none-any.whl"),
        "leaf",
        "2.0.0",
        "leaf-2.0.0",
        "",
        &[],
    )?;
    write_wheel_with_metadata(
        &context
            .temp_dir
            .child("wheels/fresh-1.0.0-py3-none-any.whl"),
        "fresh",
        "1.0.0",
        "fresh-1.0.0",
        "",
        &[],
    )?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv]
        no-index = true
        find-links = ["wheels"]
        [tool.uv.workspace]
        members = ["main", "next"]
        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["main"]
        default = true
        [[tool.uv.workspace.groups]]
        name = "next"
        members = ["next"]
    "#})?;
    context
        .temp_dir
        .child("main/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "main"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["leaf==1.0.0"]
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("next/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "next"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["leaf==2.0.0"]
        [tool.uv]
        package = false
    "#})?;
    uv_snapshot!(context.filters(), context.lock().arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");
    let existing = context.read("uv.lock");
    context
        .temp_dir
        .child("next/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "next"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["leaf==2.0.0", "fresh"]
        [tool.uv]
        package = false
    "#})?;
    uv_snapshot!(context.filters(), context.sync().args([
        "--offline", "--locked", "--dry-run", "--workspace-group", "next",
    ]), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Would use project environment at: .venv
    Resolved 5 packages in [TIME]
    Would download 2 packages
    Would install 2 packages
     + fresh==1.0.0
     + leaf==2.0.0
    error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

    hint: To update the lockfile, run `uv lock`.
    ");
    assert_eq!(existing, context.read("uv.lock"));
    Ok(())
}

/// A dynamic version does not remove the package scope of a version-independent exclusion.
#[test]
fn workspace_groups_dynamic_version_name_scoped_exclusion() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        requires-python = ">=3.12,<3.13"
        dependencies = ["leaf"]
        dynamic = ["version"]
        [build-system]
        requires = []
        build-backend = "backend"
        backend-path = ["."]
        [tool.uv]
        exclude-dependencies = [{ package = { name = "app" }, dependencies = ["leaf"] }]
        [tool.uv.workspace]
        members = ["leaf"]
        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]
        default = true
        [tool.uv.sources]
        leaf = { workspace = true }
    "#})?;
    context.temp_dir.child("backend.py").write_str(indoc! {r#"
        import pathlib

        def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
            dist_info = pathlib.Path(metadata_directory, "app-1.0.0.dist-info")
            dist_info.mkdir()
            dist_info.joinpath("METADATA").write_text(
                "Metadata-Version: 2.3\n"
                "Name: app\n"
                "Version: 1.0.0\n"
                "Requires-Python: >=3.12,<3.13\n"
                "Requires-Dist: leaf\n"
            )
            return dist_info.name

        prepare_metadata_for_build_editable = prepare_metadata_for_build_wheel
    "#})?;
    context
        .temp_dir
        .child("leaf/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "leaf"
        version = "0.1.0"
        requires-python = ">=3.13"
        [tool.uv]
        package = false
    "#})?;
    uv_snapshot!(context.filters(), context.lock().args(["--offline", "--no-index"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    let lock: toml::Value = toml::from_str(&context.read("uv.lock"))?;
    insta::assert_snapshot!(lock["workspace-group"][0]["effective-requires-python"].as_str().expect("group records its Python domain"), @"==3.12.*");
    Ok(())
}

/// A version-specific exclusion is selected from actual backend metadata before validating the domain.
#[test]
fn workspace_groups_dynamic_version_exact_scoped_exclusion() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        requires-python = ">=3.12,<3.13"
        dependencies = ["leaf"]
        dynamic = ["version"]
        [build-system]
        requires = []
        build-backend = "backend"
        backend-path = ["."]
        [tool.uv]
        exclude-dependencies = [{ package = { name = "app", version = "1.0.0" }, dependencies = ["leaf"] }]
        [tool.uv.workspace]
        members = ["leaf"]
        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]
        default = true
        [tool.uv.sources]
        leaf = { workspace = true }
    "#})?;
    context.temp_dir.child("backend.py").write_str(indoc! {r#"
        import pathlib

        def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
            dist_info = pathlib.Path(metadata_directory, "app-1.0.0.dist-info")
            dist_info.mkdir()
            dist_info.joinpath("METADATA").write_text(
                "Metadata-Version: 2.3\n"
                "Name: app\n"
                "Version: 1.0.0\n"
                "Requires-Python: >=3.12,<3.13\n"
                "Requires-Dist: leaf\n"
            )
            return dist_info.name

        prepare_metadata_for_build_editable = prepare_metadata_for_build_wheel
    "#})?;
    context
        .temp_dir
        .child("leaf/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "leaf"
        version = "0.1.0"
        requires-python = ">=3.13"
        [tool.uv]
        package = false
    "#})?;
    uv_snapshot!(context.filters(), context.lock().args(["--offline", "--no-index"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    let lock: toml::Value = toml::from_str(&context.read("uv.lock"))?;
    insta::assert_snapshot!(lock["workspace-group"][0]["effective-requires-python"].as_str().expect("group records its Python domain"), @"==3.12.*");
    Ok(())
}

/// An exact version scope can restore an edge hidden by a versionless exclusion before environment creation.
#[test]
fn workspace_groups_dynamic_version_reselects_python() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"]);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        requires-python = ">=3.12"
        dependencies = ["leaf"]
        dynamic = ["version"]
        [build-system]
        requires = []
        build-backend = "backend"
        backend-path = ["."]
        [tool.uv]
        exclude-dependencies = [
            { package = { name = "app" }, dependencies = ["leaf"] },
            { package = { name = "app", version = "2.0.0" }, dependencies = [] },
        ]
        [tool.uv.workspace]
        members = ["leaf"]
        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]
        default = true
        [tool.uv.sources]
        leaf = { workspace = true }
    "#})?;
    context.temp_dir.child("backend.py").write_str(indoc! {r#"
        import pathlib

        def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
            dist_info = pathlib.Path(metadata_directory, "app-2.0.0.dist-info")
            dist_info.mkdir()
            dist_info.joinpath("METADATA").write_text(
                "Metadata-Version: 2.3\n"
                "Name: app\n"
                "Version: 2.0.0\n"
                "Requires-Python: >=3.12\n"
                "Requires-Dist: leaf\n"
            )
            return dist_info.name

        prepare_metadata_for_build_editable = prepare_metadata_for_build_wheel
    "#})?;
    context
        .temp_dir
        .child("leaf/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "leaf"
        version = "0.1.0"
        requires-python = ">=3.13"
        [tool.uv]
        package = false
    "#})?;
    uv_snapshot!(context.filters(), context.sync().args([
        "--offline", "--no-index", "--no-install-workspace",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.13.[X] interpreter at: [PYTHON-3.13]
    Creating virtual environment at: .venv
    Resolved 2 packages in [TIME]
    Checked in [TIME]
    ");
    let lock: toml::Value = toml::from_str(&context.read("uv.lock"))?;
    insta::assert_snapshot!(lock["workspace-group"][0]["effective-requires-python"].as_str().expect("group records its Python domain"), @">=3.13");
    Ok(())
}

/// Interpreter-only commands use provisional group bounds without executing project backends.
#[test]
fn workspace_groups_interpreter_commands_do_not_build_metadata() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "1.0.0"
        requires-python = ">=3.12"
        dynamic = ["dependencies"]
        [build-system]
        requires = []
        build-backend = "backend"
        backend-path = ["."]
        [tool.uv.workspace]
        members = []
        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]
        default = true
    "#})?;
    context.temp_dir.child("backend.py").write_str(indoc! {r#"
        raise RuntimeError("interpreter discovery must not execute this backend")
    "#})?;
    uv_snapshot!(context.filters(), context.python_find(), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.12]
    ");
    uv_snapshot!(context.filters(), context.venv(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    ");
    uv_snapshot!(context.filters(), context.run().args([
        "--no-sync", "python", "-c", "import sys; print(sys.version_info[:2])",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    (3, 12)
    ");
    context
        .temp_dir
        .child("uv.lock")
        .assert(predicate::path::missing());
    Ok(())
}

/// Pin compatibility uses provisional bounds without executing project backends.
#[test]
fn workspace_groups_pin_does_not_build_metadata() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "1.0.0"
        requires-python = ">=3.12"
        dynamic = ["dependencies"]
        [build-system]
        requires = []
        build-backend = "backend"
        backend-path = ["."]
        [tool.uv.workspace]
        members = []
        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]
        default = true
    "#})?;
    context.temp_dir.child("backend.py").write_str(indoc! {r#"
        raise RuntimeError("interpreter discovery must not execute this backend")
    "#})?;
    uv_snapshot!(context.filters(), context.python_pin().arg("3.12"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Pinned `.python-version` to `3.12`
    ");
    insta::assert_snapshot!(context.read(".python-version"), @"3.12");
    context
        .temp_dir
        .child("uv.lock")
        .assert(predicate::path::missing());
    Ok(())
}

/// Initializing a member can inherit provisional workspace bounds without building metadata.
#[test]
fn workspace_groups_init_does_not_build_metadata() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "1.0.0"
        requires-python = ">=3.12"
        dynamic = ["dependencies"]
        [build-system]
        requires = []
        build-backend = "backend"
        backend-path = ["."]
        [tool.uv.workspace]
        members = []
        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]
        default = true
    "#})?;
    context.temp_dir.child("backend.py").write_str(indoc! {r#"
        raise RuntimeError("interpreter discovery must not execute this backend")
    "#})?;
    uv_snapshot!(context.filters(), context.init().args(["member", "--no-readme", "--vcs", "none"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Adding `member` as member of workspace `[TEMP_DIR]/`
    Initialized project `member` at `[TEMP_DIR]/member`
    ");
    context
        .temp_dir
        .child("uv.lock")
        .assert(predicate::path::missing());
    context
        .temp_dir
        .child(".venv")
        .assert(predicate::path::missing());
    Ok(())
}

/// Removing from a modern group replaces an incompatible environment accepted by another group.
#[test]
fn workspace_groups_remove_selects_target_python() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"]);
    context.temp_dir.child("wheels").create_dir_all()?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv]
        no-index = true
        find-links = ["wheels"]
        [tool.uv.workspace]
        members = ["legacy", "modern"]
        [[tool.uv.workspace.groups]]
        name = "legacy"
        members = ["legacy"]
        [[tool.uv.workspace.groups]]
        name = "modern"
        members = ["modern"]
    "#})?;
    context
        .temp_dir
        .child("legacy/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "legacy"
        version = "0.1.0"
        requires-python = "==3.12.*"
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("modern/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "modern"
        version = "0.1.0"
        requires-python = ">=3.13"
        dependencies = ["old"]
        [tool.uv]
        package = false

    "#})?;
    context.venv().args(["--python", "3.12"]).assert().success();
    uv_snapshot!(context.filters(), context.remove().args([
        "--package", "modern", "old", "--offline",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.13.[X] interpreter at: [PYTHON-3.13]
    Removed virtual environment at: .venv
    Creating virtual environment at: .venv
    Resolved 2 packages in [TIME]
    Checked in [TIME]
    ");
    context
        .assert_command("import sys; assert sys.version_info[:2] == (3, 13)")
        .success();
    assert!(!context.read("modern/pyproject.toml").contains("old"));
    Ok(())
}

/// Adding to a modern group uses the same Python context for discovery and synchronization.
#[test]
fn workspace_groups_add_selects_target_python() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"]);
    context.temp_dir.child("wheels").create_dir_all()?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv]
        no-index = true
        find-links = ["wheels"]
        [tool.uv.workspace]
        members = ["legacy", "modern"]
        [[tool.uv.workspace.groups]]
        name = "legacy"
        members = ["legacy"]
        [[tool.uv.workspace.groups]]
        name = "modern"
        members = ["modern"]
    "#})?;
    context
        .temp_dir
        .child("legacy/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "legacy"
        version = "0.1.0"
        requires-python = "==3.12.*"
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("modern/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "modern"
        version = "0.1.0"
        requires-python = ">=3.13"
        dependencies = []
        [tool.uv]
        package = false

    "#})?;
    context.venv().args(["--python", "3.12"]).assert().success();
    write_wheel_with_metadata(
        &context
            .temp_dir
            .child("wheels/fresh-1.0.0-py3-none-any.whl"),
        "fresh",
        "1.0.0",
        "fresh-1.0.0",
        "",
        &[],
    )?;
    uv_snapshot!(context.filters(), context.add().args([
        "--package", "modern", "fresh", "--offline",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.13.[X] interpreter at: [PYTHON-3.13]
    Removed virtual environment at: .venv
    Creating virtual environment at: .venv
    Resolved 3 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + fresh==1.0.0
    ");
    context
        .assert_command("import sys; assert sys.version_info[:2] == (3, 13)")
        .success();
    Ok(())
}

/// Version edits synchronize using the edited member's Python domain.
#[test]
fn workspace_groups_version_selects_target_python() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"]);
    context.temp_dir.child("wheels").create_dir_all()?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv]
        no-index = true
        find-links = ["wheels"]
        [tool.uv.workspace]
        members = ["legacy", "modern"]
        [[tool.uv.workspace.groups]]
        name = "legacy"
        members = ["legacy"]
        [[tool.uv.workspace.groups]]
        name = "modern"
        members = ["modern"]
    "#})?;
    context
        .temp_dir
        .child("legacy/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "legacy"
        version = "0.1.0"
        requires-python = "==3.12.*"
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("modern/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "modern"
        version = "0.1.0"
        requires-python = ">=3.13"
        dependencies = []
        [tool.uv]
        package = false

    "#})?;
    context.venv().args(["--python", "3.12"]).assert().success();
    uv_snapshot!(context.filters(), context.version()
        .current_dir(context.temp_dir.child("modern"))
        .args(["0.2.0", "--offline"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    modern 0.1.0 => 0.2.0

    ----- stderr -----
    Using CPython 3.13.[X] interpreter at: [PYTHON-3.13]
    Removed virtual environment at: [VENV]/
    Creating virtual environment at: [VENV]/
    Resolved 2 packages in [TIME]
    Checked in [TIME]
    ");
    context
        .assert_command("import sys; assert sys.version_info[:2] == (3, 13)")
        .success();
    assert!(
        context
            .read("modern/pyproject.toml")
            .contains("version = \"0.2.0\"")
    );
    Ok(())
}

/// A lock-only edit can use any interpreter in the combined workspace domain.
#[test]
fn workspace_groups_remove_no_sync_keeps_union_python() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"]);
    context.temp_dir.child("wheels").create_dir_all()?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv]
        no-index = true
        find-links = ["wheels"]
        [tool.uv.workspace]
        members = ["legacy", "modern"]
        [[tool.uv.workspace.groups]]
        name = "legacy"
        members = ["legacy"]
        [[tool.uv.workspace.groups]]
        name = "modern"
        members = ["modern"]
    "#})?;
    context
        .temp_dir
        .child("legacy/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "legacy"
        version = "0.1.0"
        requires-python = "==3.12.*"
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("modern/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "modern"
        version = "0.1.0"
        requires-python = ">=3.13"
        dependencies = ["old"]
        [tool.uv]
        package = false

    "#})?;
    context.venv().args(["--python", "3.12"]).assert().success();
    uv_snapshot!(context.filters(), context.remove().args([
        "--package", "modern", "old", "--offline", "--no-sync", "--python", "3.12",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    context
        .assert_command("import sys; assert sys.version_info[:2] == (3, 12)")
        .success();
    Ok(())
}

/// Frozen tool lookup projects the selected group without changing an incompatible environment.
#[test]
fn workspace_groups_check_frozen_no_sync_selects_locked_tool() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"]);
    context.temp_dir.child("wheels").create_dir_all()?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv]
        no-index = true
        find-links = ["wheels"]
        [tool.uv.workspace]
        members = ["legacy", "modern"]
        [[tool.uv.workspace.groups]]
        name = "legacy"
        members = ["legacy"]
        [[tool.uv.workspace.groups]]
        name = "modern"
        members = ["modern"]
    "#})?;
    context
        .temp_dir
        .child("legacy/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "legacy"
        version = "0.1.0"
        requires-python = "==3.12.*"
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("modern/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "modern"
        version = "0.1.0"
        requires-python = ">=3.13"
        dependencies = []
        [tool.uv]
        package = false
        [dependency-groups]
        dev = ["ty==1.2.3"]
    "#})?;
    context.venv().args(["--python", "3.12"]).assert().success();
    let (filename, wheel) = generate_wheel_with_files(
        &"ty".parse()?,
        &"1.2.3".parse()?,
        &[],
        &BTreeMap::new(),
        Some(&">=3.13".parse()?),
        "py3-none-any",
        &[
            (
                "ty/cli.py",
                indoc! {r#"
                import sys

                def main():
                    assert sys.version_info[:2] == (3, 13)
                    if "--version" in sys.argv:
                        print("ty 1.2.3")
                    else:
                        print("All checks passed!")
            "#},
            ),
            (
                "ty-1.2.3.dist-info/entry_points.txt",
                "[console_scripts]\nty=ty.cli:main\n",
            ),
        ],
    );
    context
        .temp_dir
        .child("wheels")
        .child(filename)
        .write_binary(&wheel)?;
    context.lock().arg("--offline").assert().success();
    uv_snapshot!(context.filters(), context.check().args([
        "--package", "modern", "--frozen", "--no-sync", "--offline", "--show-version",
        "--preview-features", "check-command",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    All checks passed!

    ----- stderr -----
    warning: Using incompatible environment (`.venv`) due to `--no-sync` (The project environment's Python version does not satisfy the request: `Python >=3.13`)
    Using CPython 3.13.[X] interpreter at: [PYTHON-3.13]
    Installed 1 package in [TIME]
    Using ty 1.2.3
    ");
    context
        .assert_command("import sys; assert sys.version_info[:2] == (3, 12)")
        .success();
    uv_test::assert_path_missing(context.site_packages().join("ty"));
    Ok(())
}

/// Tool lookup and synchronization traverse the same selected workspace context.
#[test]
fn workspace_groups_check_selects_locked_tool_and_python() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"]);
    context.temp_dir.child("wheels").create_dir_all()?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv]
        no-index = true
        find-links = ["wheels"]
        [tool.uv.workspace]
        members = ["legacy", "modern"]
        [[tool.uv.workspace.groups]]
        name = "legacy"
        members = ["legacy"]
        [[tool.uv.workspace.groups]]
        name = "modern"
        members = ["modern"]
    "#})?;
    context
        .temp_dir
        .child("legacy/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "legacy"
        version = "0.1.0"
        requires-python = "==3.12.*"
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("modern/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "modern"
        version = "0.1.0"
        requires-python = ">=3.13"
        dependencies = []
        [tool.uv]
        package = false
        [dependency-groups]
        dev = ["ty==1.2.3"]
    "#})?;
    context.venv().args(["--python", "3.12"]).assert().success();
    let (filename, wheel) = generate_wheel_with_files(
        &"ty".parse()?,
        &"1.2.3".parse()?,
        &[],
        &BTreeMap::new(),
        Some(&">=3.13".parse()?),
        "py3-none-any",
        &[
            (
                "ty/cli.py",
                indoc! {r#"
                import sys

                def main():
                    assert sys.version_info[:2] == (3, 13)
                    if "--version" in sys.argv:
                        print("ty 1.2.3")
                    else:
                        print("All checks passed!")
            "#},
            ),
            (
                "ty-1.2.3.dist-info/entry_points.txt",
                "[console_scripts]\nty=ty.cli:main\n",
            ),
        ],
    );
    context
        .temp_dir
        .child("wheels")
        .child(filename)
        .write_binary(&wheel)?;
    uv_snapshot!(context.filters(), context.check().args([
        "--package", "modern", "--offline", "--show-version",
        "--preview-features", "check-command",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    All checks passed!

    ----- stderr -----
    Using CPython 3.13.[X] interpreter at: [PYTHON-3.13]
    Removed virtual environment at: .venv
    Creating virtual environment at: .venv
    Installed 1 package in [TIME]
    Using ty 1.2.3
    ");
    context
        .assert_command("import sys; assert sys.version_info[:2] == (3, 13)")
        .success();
    Ok(())
}

/// Adding a dependency completes dynamic group discovery before creating the project environment.
#[test]
fn workspace_groups_add_reselects_python_after_metadata() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"]);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        requires-python = ">=3.12"
        dependencies = ["leaf"]
        dynamic = ["version"]
        [build-system]
        requires = []
        build-backend = "backend"
        backend-path = ["."]
        [tool.uv]
        exclude-dependencies = [
            { package = { name = "app" }, dependencies = ["leaf"] },
            { package = { name = "app", version = "2.0.0" }, dependencies = [] },
        ]
        [tool.uv.workspace]
        members = ["leaf"]
        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]
        default = true
        [tool.uv.sources]
        leaf = { workspace = true }
    "#})?;
    context.temp_dir.child("backend.py").write_str(indoc! {r#"
        import pathlib
        import tomllib

        def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
            project = tomllib.loads(pathlib.Path("pyproject.toml").read_text())["project"]
            dist_info = pathlib.Path(metadata_directory, "app-2.0.0.dist-info")
            dist_info.mkdir()
            dist_info.joinpath("METADATA").write_text(
                "Metadata-Version: 2.3\n"
                "Name: app\n"
                "Version: 2.0.0\n"
                "Requires-Python: >=3.12\n"
                + "".join(f"Requires-Dist: {dependency}\n" for dependency in project["dependencies"])
            )
            return dist_info.name

        prepare_metadata_for_build_editable = prepare_metadata_for_build_wheel
    "#})?;
    context
        .temp_dir
        .child("leaf/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "leaf"
        version = "0.1.0"
        requires-python = ">=3.13"
        [tool.uv]
        package = false
    "#})?;
    context.temp_dir.child("wheels").create_dir_all()?;
    write_wheel_with_metadata(
        &context
            .temp_dir
            .child("wheels/fresh-1.0.0-py3-none-any.whl"),
        "fresh",
        "1.0.0",
        "fresh-1.0.0",
        "",
        &[],
    )?;
    uv_snapshot!(context.filters(), context.add().args([
        "fresh", "--offline", "--no-index", "--find-links", "wheels", "--no-install-workspace",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.13.[X] interpreter at: [PYTHON-3.13]
    Creating virtual environment at: .venv
    Resolved 3 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + fresh==1.0.0
    ");
    uv_snapshot!(context.filters(), context.run().args([
        "--no-sync", "python", "-c", "import sys; print(sys.version_info[:2])",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    (3, 13)
    ");
    Ok(())
}

/// Removing a dependency still validates an explicit Python request against completed metadata.
#[test]
fn workspace_groups_remove_checks_final_python_domain() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"]);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        requires-python = ">=3.12"
        dependencies = ["leaf", "fresh"]
        dynamic = ["version"]
        [build-system]
        requires = []
        build-backend = "backend"
        backend-path = ["."]
        [tool.uv]
        exclude-dependencies = [
            { package = { name = "app" }, dependencies = ["leaf"] },
            { package = { name = "app", version = "2.0.0" }, dependencies = [] },
        ]
        [tool.uv.workspace]
        members = ["leaf"]
        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]
        default = true
        [tool.uv.sources]
        leaf = { workspace = true }
    "#})?;
    context.temp_dir.child("backend.py").write_str(indoc! {r#"
        import pathlib
        import tomllib

        def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
            project = tomllib.loads(pathlib.Path("pyproject.toml").read_text())["project"]
            dist_info = pathlib.Path(metadata_directory, "app-2.0.0.dist-info")
            dist_info.mkdir()
            dist_info.joinpath("METADATA").write_text(
                "Metadata-Version: 2.3\n"
                "Name: app\n"
                "Version: 2.0.0\n"
                "Requires-Python: >=3.12\n"
                + "".join(f"Requires-Dist: {dependency}\n" for dependency in project["dependencies"])
            )
            return dist_info.name

        prepare_metadata_for_build_editable = prepare_metadata_for_build_wheel
    "#})?;
    context
        .temp_dir
        .child("leaf/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "leaf"
        version = "0.1.0"
        requires-python = ">=3.13"
        [tool.uv]
        package = false
    "#})?;
    let original = context.read("pyproject.toml");
    uv_snapshot!(context.filters(), context.remove().args([
        "fresh", "--offline", "--no-index", "--no-sync", "--python", "3.12",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    error: The requested interpreter resolved to Python 3.12.[X], which is incompatible with the project's Python requirement: `>=3.13`
    ");
    assert_eq!(original, context.read("pyproject.toml"));
    context
        .temp_dir
        .child(".venv")
        .assert(predicate::path::missing());
    context
        .temp_dir
        .child("uv.lock")
        .assert(predicate::path::missing());
    Ok(())
}

/// Dynamic dependency metadata can raise the inferred domain before a project environment is created.
#[test]
fn workspace_groups_dynamic_dependencies_reselect_python() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"]);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        requires-python = ">=3.12"
        version = "2.0.0"
        dynamic = ["dependencies"]
        [build-system]
        requires = []
        build-backend = "backend"
        backend-path = ["."]
        [tool.uv.workspace]
        members = ["leaf"]
        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]
        default = true
        [tool.uv.sources]
        leaf = { workspace = true }
    "#})?;
    context.temp_dir.child("backend.py").write_str(indoc! {r#"
        import pathlib

        def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
            dist_info = pathlib.Path(metadata_directory, "app-2.0.0.dist-info")
            dist_info.mkdir()
            dist_info.joinpath("METADATA").write_text(
                "Metadata-Version: 2.3\n"
                "Name: app\n"
                "Version: 2.0.0\n"
                "Requires-Python: >=3.12\n"
                "Requires-Dist: leaf\n"
            )
            return dist_info.name

        prepare_metadata_for_build_editable = prepare_metadata_for_build_wheel
    "#})?;
    context
        .temp_dir
        .child("leaf/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "leaf"
        version = "0.1.0"
        requires-python = ">=3.13"
        [tool.uv]
        package = false
    "#})?;
    uv_snapshot!(context.filters(), context.sync().args([
        "--offline", "--no-index", "--no-install-workspace",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.13.[X] interpreter at: [PYTHON-3.13]
    Creating virtual environment at: .venv
    Resolved 2 packages in [TIME]
    Checked in [TIME]
    ");
    uv_snapshot!(context.filters(), context.tree().args(["--offline", "--no-index"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    app v2.0.0
    └── leaf v0.1.0

    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    let lock: toml::Value = toml::from_str(&context.read("uv.lock"))?;
    insta::assert_snapshot!(lock["workspace-group"][0]["effective-requires-python"].as_str().expect("group records its Python domain"), @">=3.13");
    Ok(())
}

/// Metadata can reveal another dynamic member whose backend needs a different interpreter.
#[test]
fn workspace_groups_dynamic_dependencies_refine_new_members() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"]);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "1.0.0"
        requires-python = ">=3.12"
        dynamic = ["dependencies"]
        [build-system]
        requires = []
        build-backend = "backend"
        backend-path = ["."]
        [tool.uv.workspace]
        members = ["child", "leaf"]
        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]
        default = true
        [tool.uv.sources]
        child = { workspace = true }
        leaf = { workspace = true }
    "#})?;
    context.temp_dir.child("backend.py").write_str(indoc! {r#"
        import pathlib

        def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
            dist_info = pathlib.Path(metadata_directory, "app-1.0.0.dist-info")
            dist_info.mkdir()
            dist_info.joinpath("METADATA").write_text(
                "Metadata-Version: 2.3\n"
                "Name: app\n"
                "Version: 1.0.0\n"
                "Requires-Python: >=3.12\n"
                "Requires-Dist: child\n"
            )
            return dist_info.name

        prepare_metadata_for_build_editable = prepare_metadata_for_build_wheel
    "#})?;
    context
        .temp_dir
        .child("child/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "child"
        version = "1.0.0"
        requires-python = ">=3.13"
        dynamic = ["dependencies"]
        [build-system]
        requires = []
        build-backend = "backend"
        backend-path = ["."]
    "#})?;
    context
        .temp_dir
        .child("child/backend.py")
        .write_str(indoc! {r#"
        import pathlib
        import sys

        def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
            assert sys.version_info[:2] == (3, 13)
            dist_info = pathlib.Path(metadata_directory, "child-1.0.0.dist-info")
            dist_info.mkdir()
            dist_info.joinpath("METADATA").write_text(
                "Metadata-Version: 2.3\n"
                "Name: child\n"
                "Version: 1.0.0\n"
                "Requires-Python: >=3.13\n"
                "Requires-Dist: leaf\n"
            )
            return dist_info.name

        prepare_metadata_for_build_editable = prepare_metadata_for_build_wheel
    "#})?;
    context
        .temp_dir
        .child("leaf/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "leaf"
        version = "1.0.0"
        requires-python = ">=3.13"
        [tool.uv]
        package = false
    "#})?;
    uv_snapshot!(context.filters(), context.sync().args([
        "--offline", "--no-index", "--no-install-workspace", "--no-install-package", "child",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.13.[X] interpreter at: [PYTHON-3.13]
    Creating virtual environment at: .venv
    Resolved 3 packages in [TIME]
    Checked in [TIME]
    ");
    let lock: toml::Value = toml::from_str(&context.read("uv.lock"))?;
    insta::assert_snapshot!(lock["workspace-group"][0]["effective-requires-python"].as_str().expect("group records its Python domain"), @">=3.13");
    Ok(())
}

/// An explicit interpreter selects the command's group, while other groups can build metadata.
#[test]
fn workspace_groups_dynamic_metadata_uses_group_python() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"]);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv.workspace]
        members = ["main", "future"]
        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["main"]
        default = true
        [[tool.uv.workspace.groups]]
        name = "future"
        members = ["future"]
    "#})?;
    context
        .temp_dir
        .child("main/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "main"
        version = "1.0.0"
        requires-python = ">=3.12,<3.13"
        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child("future/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "future"
        version = "1.0.0"
        requires-python = ">=3.13"
        dynamic = ["dependencies"]
        [build-system]
        requires = []
        build-backend = "backend"
        backend-path = ["."]
    "#})?;
    context
        .temp_dir
        .child("future/backend.py")
        .write_str(indoc! {r#"
        import pathlib
        import sys

        def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
            assert sys.version_info[:2] == (3, 13)
            dist_info = pathlib.Path(metadata_directory, "future-1.0.0.dist-info")
            dist_info.mkdir()
            dist_info.joinpath("METADATA").write_text(
                "Metadata-Version: 2.3\n"
                "Name: future\n"
                "Version: 1.0.0\n"
                "Requires-Python: >=3.13\n"
            )
            return dist_info.name

        prepare_metadata_for_build_editable = prepare_metadata_for_build_wheel
    "#})?;
    uv_snapshot!(context.filters(), context.sync().args([
        "--offline", "--no-index", "--python", "3.12", "--no-install-workspace",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Resolved 2 packages in [TIME]
    Checked in [TIME]
    ");
    uv_snapshot!(context.filters(), context.sync().args([
        "--offline", "--no-index", "--python", "3.12", "--workspace-group", "future",
        "--no-install-workspace",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    error: The requested interpreter resolved to Python 3.12.[X], which is incompatible with the project's Python requirement: `>=3.13` (from workspace member `future`'s `project.requires-python`).
    ");
    context
        .assert_command("import sys; assert sys.version_info[:2] == (3, 12)")
        .success();
    Ok(())
}

/// A conditional dynamic member builds metadata within its own reachable Python domain.
#[test]
fn workspace_groups_dynamic_metadata_uses_member_python() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"]);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "app"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["child; python_version >= '3.13'"]
        [tool.uv]
        package = false
        [tool.uv.workspace]
        members = ["child"]
        [[tool.uv.workspace.groups]]
        name = "main"
        members = ["app"]
        default = true
        [tool.uv.sources]
        child = { workspace = true }
    "#})?;
    context
        .temp_dir
        .child("child/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "child"
        version = "1.0.0"
        requires-python = ">=3.13"
        dynamic = ["dependencies"]
        [build-system]
        requires = []
        build-backend = "backend"
        backend-path = ["."]
    "#})?;
    context
        .temp_dir
        .child("child/backend.py")
        .write_str(indoc! {r#"
        import pathlib
        import sys

        def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
            assert sys.version_info[:2] == (3, 13)
            dist_info = pathlib.Path(metadata_directory, "child-1.0.0.dist-info")
            dist_info.mkdir()
            dist_info.joinpath("METADATA").write_text(
                "Metadata-Version: 2.3\n"
                "Name: child\n"
                "Version: 1.0.0\n"
                "Requires-Python: >=3.13\n"
            )
            return dist_info.name

        prepare_metadata_for_build_editable = prepare_metadata_for_build_wheel
    "#})?;
    uv_snapshot!(context.filters(), context.sync().args([
        "--offline", "--no-index", "--python", "3.12",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Resolved 2 packages in [TIME]
    Checked in [TIME]
    ");
    let lock: toml::Value = toml::from_str(&context.read("uv.lock"))?;
    insta::assert_snapshot!(lock["workspace-group"][0]["effective-requires-python"].as_str().expect("group records its Python domain"), @">=3.12");
    Ok(())
}
