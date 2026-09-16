use std::str::FromStr;

use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::fixture::{FileWriteStr, PathChild, PathCreateDir};
use indoc::{formatdoc, indoc};
use uv_pep508::MarkerTree;
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
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to resolve workspace group `main`
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
    error: The selected packages are not covered by a single workspace group; add them to a group or select a narrower target
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
    for (name, excluded) in [("one", 1), ("two", 2), ("three", 3)] {
        let output = context
            .export()
            .args([
                "--offline",
                "--frozen",
                "--workspace-group",
                name,
                "--no-header",
                "--no-hashes",
            ])
            .output()?;
        output.clone().assert().success();
        let output = String::from_utf8(output.stdout)?;
        assert!(output.contains("leaf=="));
        assert!(!output.contains(&format!("leaf=={excluded}.0.0")));
    }
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
    for (name, version) in [("main", 1), ("next", 2)] {
        let output = context
            .export()
            .args([
                "--offline",
                "--frozen",
                "--workspace-group",
                name,
                "--no-header",
                "--no-hashes",
            ])
            .output()?;
        output.clone().assert().success();
        assert!(String::from_utf8(output.stdout)?.contains(&format!("common_leaf-{version}.0.0")));
    }
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
    for (name, version) in [("main", 1), ("next", 2)] {
        let output = context
            .export()
            .args([
                "--offline",
                "--frozen",
                "--workspace-group",
                name,
                "--no-header",
                "--no-hashes",
            ])
            .output()?;
        output.clone().assert().success();
        assert!(String::from_utf8(output.stdout)?.contains(&format!("common-leaf=={version}.0.0")));
    }
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
