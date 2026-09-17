use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::fixture::{FileWriteStr, PathChild, PathCreateDir};
use indoc::{formatdoc, indoc};
use uv_test::{TestContext, uv_snapshot};

use super::workspace_metadata::write_wheel_with_metadata;

const AXES: &[&str] = &[
    "--offline",
    "--preview-features",
    "workspace-resolution-axes",
];

fn write_wheels(context: &TestContext, packages: &[(&str, &str)]) -> Result<()> {
    for &(name, version) in packages {
        write_metadata_wheel(context, name, version, "")?;
    }
    Ok(())
}

fn write_metadata_wheel(
    context: &TestContext,
    name: &str,
    version: &str,
    metadata: &str,
) -> Result<()> {
    let wheels = context.temp_dir.child("wheels");
    wheels.create_dir_all()?;
    let stem = format!("{}-{version}", name.replace('-', "_"));
    write_wheel_with_metadata(
        wheels.child(format!("{stem}-py3-none-any.whl")).path(),
        name,
        version,
        &stem,
        metadata,
        &[],
    )
}

fn write_member(context: &TestContext, name: &str, dependencies: &str) -> Result<()> {
    context
        .temp_dir
        .child(format!("members/{name}/pyproject.toml"))
        .write_str(&formatdoc! {r#"
        [project]
        name = "{name}"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [{dependencies}]

        [tool.uv]
        package = false
    "#})?;
    Ok(())
}

fn raw_viewer_workspace(context: &TestContext) -> Result<()> {
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv]
        no-index = true
        find-links = ["wheels"]
        default-groups = []

        [tool.uv.workspace]
        members = ["members/*"]

        [tool.uv.workspace.resolution-axes.runtime]
        legacy = { members = ["legacy"], constraint-dependencies = ["switch-leaf==1.0.0"] }
        modern = { members = ["modern"], constraint-dependencies = ["switch-leaf==2.0.0"] }
    "#})?;
    for name in ["legacy", "modern", "shared"] {
        write_member(context, name, "\"switch-leaf>=1\"")?;
    }
    write_wheels(
        context,
        &[("switch-leaf", "1.0.0"), ("switch-leaf", "2.0.0")],
    )
}

fn higher_order_workspace(context: &TestContext, members: &[(&str, u8)]) -> Result<()> {
    let sections = ["one", "two", "three"]
        .into_iter()
        .map(|name| {
            let members = if members.iter().any(|(member, _)| *member == name) {
                format!(r#"["{name}"]"#)
            } else {
                "[]".to_owned()
            };
            formatdoc! {r"

            [tool.uv.workspace.resolution-axes.{name}]
            off = {{}}
            on = {{ members = {members} }}
            "}
        })
        .collect::<String>();
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
        [tool.uv]
        no-index = true
        find-links = ["wheels"]

        [tool.uv.workspace]
        members = ["members/*"]
        {sections}
    "#})?;
    write_member(context, "common", "")?;
    for &(name, excluded) in members {
        write_member(context, name, &format!(r#""leaf!={excluded}.0.0""#))?;
    }
    write_wheels(
        context,
        &[("leaf", "1.0.0"), ("leaf", "2.0.0"), ("leaf", "3.0.0")],
    )
}

/// Pairwise-compatible roots can still make one complete, legal axis context unsatisfiable.
/// Each root fixes a different axis, so no member's own assignments name the failing context.
#[test]
fn workspace_resolution_axes_three_way_only_incompatibility() -> Result<()> {
    for (members, expected) in [
        ([("one", 1), ("two", 2)], "leaf==3.0.0\n"),
        ([("one", 1), ("three", 3)], "leaf==2.0.0\n"),
        ([("two", 2), ("three", 3)], "leaf==1.0.0\n"),
    ] {
        let context = uv_test::test_context!("3.12");
        higher_order_workspace(&context, &members)?;
        context.lock().args(AXES).assert().success();
        let output = context
            .export()
            .args(AXES)
            .args([
                "--frozen",
                "--all-matching-packages",
                "--resolution-axis",
                "one=on",
                "--resolution-axis",
                "two=on",
                "--resolution-axis",
                "three=on",
                "--no-header",
                "--no-hashes",
                "--no-annotate",
            ])
            .output()?;
        output.clone().assert().success();
        assert_eq!(String::from_utf8(output.stdout)?, expected);
        let output = context
            .export()
            .args(AXES)
            .args([
                "--frozen",
                "--all-matching-packages",
                "--resolution-axis",
                "one=off",
                "--resolution-axis",
                "two=off",
                "--resolution-axis",
                "three=off",
                "--no-header",
                "--no-hashes",
                "--no-annotate",
            ])
            .output()?;
        output.clone().assert().success();
        assert_eq!(String::from_utf8(output.stdout)?, "");
    }

    let context = uv_test::test_context!("3.12");
    higher_order_workspace(&context, &[("one", 1), ("two", 2), ("three", 3)])?;
    context.lock().args(AXES).assert().code(1);
    assert!(!context.temp_dir.child("uv.lock").path().exists());
    Ok(())
}

fn write_membership_config(context: &TestContext, legacy: &str, modern: &str) -> Result<()> {
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
        [tool.uv]
        no-index = true
        find-links = ["wheels"]

        [tool.uv.workspace]
        members = ["members/*"]

        [tool.uv.workspace.resolution-axes.runtime]
        legacy = {{ members = ["{legacy}"], constraint-dependencies = ["switch-leaf==1"] }}
        modern = {{ members = ["{modern}"], constraint-dependencies = ["switch-leaf==2"] }}
    "#})?;
    Ok(())
}

/// The frozen lock retains its own membership model, while a checked lock must notice and
/// relock a change to the assignments even when the member set and leaf versions are unchanged.
#[test]
fn workspace_resolution_axes_membership_relock() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    write_membership_config(&context, "one", "two")?;
    write_member(&context, "one", r#""switch-leaf""#)?;
    write_member(&context, "two", r#""switch-leaf""#)?;
    write_wheels(
        &context,
        &[("switch-leaf", "1.0.0"), ("switch-leaf", "2.0.0")],
    )?;
    context.lock().args(AXES).assert().success();
    let original = context.read("uv.lock");
    uv_snapshot!(context.filters(), context.export().args(AXES).args([
        "--frozen", "--package", "one", "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    switch-leaf==1.0.0
    ");

    write_membership_config(&context, "two", "one")?;
    context.lock().args(AXES).arg("--locked").assert().failure();
    assert_eq!(context.read("uv.lock"), original);
    uv_snapshot!(context.filters(), context.export().args(AXES).args([
        "--frozen", "--package", "one", "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    switch-leaf==1.0.0
    ");

    context.lock().args(AXES).assert().success();
    assert_ne!(context.read("uv.lock"), original);
    uv_snapshot!(context.filters(), context.export().args(AXES).args([
        "--frozen", "--package", "one", "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    switch-leaf==2.0.0
    ");
    uv_snapshot!(context.filters(), context.export().args(AXES).args([
        "--frozen", "--package", "two", "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    switch-leaf==1.0.0
    ");
    let relocked = context.read("uv.lock");
    context.lock().args(AXES).arg("--locked").assert().success();
    assert_eq!(context.read("uv.lock"), relocked);
    Ok(())
}

fn activation_workspace(context: &TestContext) -> Result<()> {
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv]
        no-index = true
        find-links = ["wheels"]
        default-groups = []

        [tool.uv.workspace]
        members = ["members/*"]

        [tool.uv.workspace.resolution-axes.runtime]
        legacy = { members = ["legacy"], constraint-dependencies = ["feature-leaf==1", "group-leaf==1"] }
        modern = { members = ["modern"], constraint-dependencies = ["feature-leaf==2", "group-leaf==2"] }
    "#})?;
    write_member(context, "legacy", "")?;
    write_member(context, "modern", "")?;
    context
        .temp_dir
        .child("members/shared/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "shared"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["common-leaf"]

        [project.optional-dependencies]
        feature = ["feature-leaf"]

        [dependency-groups]
        qa = ["group-leaf"]

        [tool.uv]
        package = false
        default-groups = []
    "#})?;
    write_wheels(
        context,
        &[
            ("common-leaf", "1.0.0"),
            ("feature-leaf", "1.0.0"),
            ("feature-leaf", "2.0.0"),
            ("group-leaf", "1.0.0"),
            ("group-leaf", "2.0.0"),
        ],
    )
}

/// Unrequested extras and groups do not make an otherwise identical installation ambiguous.
/// Enabling either one exposes the genuine version choice on the unresolved axis.
#[test]
fn workspace_resolution_axes_optional_and_group_selection() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    activation_workspace(&context)?;
    context.lock().args(AXES).assert().success();
    uv_snapshot!(context.filters(), context.export().args(AXES).args([
        "--frozen", "--package", "shared", "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    common-leaf==1.0.0
    ");
    uv_snapshot!(context.filters(), context.export().args(AXES).args([
        "--frozen", "--package", "shared", "--extra", "feature",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The selected packages have different locked resolutions on unresolved axis `runtime`; select a section with `--resolution-axis AXIS=SECTION`
    ");
    uv_snapshot!(context.filters(), context.export().args(AXES).args([
        "--frozen", "--package", "shared", "--group", "qa",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The selected packages have different locked resolutions on unresolved axis `runtime`; select a section with `--resolution-axis AXIS=SECTION`
    ");
    uv_snapshot!(context.filters(), context.export().args(AXES).args([
        "--frozen", "--package", "shared", "--extra", "feature",
        "--resolution-axis", "runtime=legacy",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    common-leaf==1.0.0
    feature-leaf==1.0.0
    ");
    uv_snapshot!(context.filters(), context.export().args(AXES).args([
        "--frozen", "--package", "shared", "--group", "qa",
        "--resolution-axis", "runtime=modern",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    common-leaf==1.0.0
    group-leaf==2.0.0
    ");
    Ok(())
}

/// A universal lock cannot erase a cross-lane dependency merely because it occurs behind an
/// optional extra or a non-default dependency group.
#[test]
fn workspace_resolution_axes_cross_lane_optional_and_group_dependencies() -> Result<()> {
    for dependencies in [
        "[project.optional-dependencies]\ncross = [\"modern\"]\n",
        "[dependency-groups]\ncross = [\"modern\"]\n",
    ] {
        let context = uv_test::test_context!("3.12");
        context
            .temp_dir
            .child("pyproject.toml")
            .write_str(indoc! {r#"
            [tool.uv.workspace]
            members = ["members/*"]

            [tool.uv.workspace.resolution-axes.runtime]
            legacy = { members = ["legacy"] }
            modern = { members = ["modern"] }

            [tool.uv.sources]
            modern = { workspace = true }
        "#})?;
        write_member(&context, "modern", "")?;
        context
            .temp_dir
            .child("members/legacy/pyproject.toml")
            .write_str(&formatdoc! {r#"
            [project]
            name = "legacy"
            version = "0.1.0"
            requires-python = ">=3.12"

            {dependencies}
            [tool.uv]
            package = false
            default-groups = []
        "#})?;
        context.lock().args(AXES).assert().failure();
        assert!(!context.temp_dir.child("uv.lock").path().exists());
    }
    Ok(())
}

/// Interpreter discovery for a selected member must not intersect group Python requirements from
/// other members that merely remain possible in an unresolved context.
#[test]
fn workspace_resolution_axes_unrelated_group_python() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv]
        no-index = true
        find-links = ["wheels"]
        default-groups = []

        [tool.uv.workspace]
        members = ["members/*"]

        [tool.uv.workspace.resolution-axes.runtime]
        legacy = { members = ["legacy"], requires-python = "==3.12.*" }
        modern = { members = ["modern"], requires-python = "==3.13.*" }
    "#})?;
    for (name, requires_python) in [("legacy", "==3.12.*"), ("modern", "==3.13.*")] {
        context
            .temp_dir
            .child(format!("members/{name}/pyproject.toml"))
            .write_str(&formatdoc! {r#"
            [project]
            name = "{name}"
            version = "0.1.0"
            requires-python = ">=3.12,<3.14"

            [dependency-groups]
            qa = []

            [tool.uv]
            package = false
            default-groups = []

            [tool.uv.dependency-groups]
            qa = {{ requires-python = "{requires_python}" }}
        "#})?;
    }
    context
        .temp_dir
        .child("members/shared/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "shared"
        version = "0.1.0"
        requires-python = ">=3.12,<3.14"

        [dependency-groups]
        qa = ["common-leaf"]

        [tool.uv]
        package = false
        default-groups = []
    "#})?;
    write_wheels(&context, &[("common-leaf", "1.0.0")])?;
    context.lock().args(AXES).assert().success();
    uv_snapshot!(context.filters(), context.export().args(AXES).args([
        "--package", "shared", "--group", "qa", "--python", "3.12",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    common-leaf==1.0.0

    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");
    Ok(())
}

/// A wheel's transitive requirement can resolve to another workspace root. If that member is not
/// available in every context, an older compatible wheel can still produce a complete lock.
#[test]
fn workspace_resolution_axes_transitive_member_candidate() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let configuration = indoc! {r#"
        [tool.uv]
        no-index = true
        find-links = ["wheels"]

        [tool.uv.workspace]
        members = ["members/*"]

        [tool.uv.sources]
        legacy = { workspace = true }
    "#};
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(configuration)?;
    write_member(&context, "legacy", "")?;
    write_member(&context, "modern", "")?;
    write_member(&context, "shared", r#""bridge>=1""#)?;
    write_metadata_wheel(&context, "bridge", "1.0.0", "")?;
    write_metadata_wheel(&context, "bridge", "2.0.0", "Requires-Dist: legacy\n")?;

    context.lock().arg("--offline").assert().success();
    let lock: toml::Value = toml::from_str(&context.read("uv.lock"))?;
    let packages = lock["package"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("lock packages missing"))?;
    let bridge = packages
        .iter()
        .find(|package| package["name"].as_str() == Some("bridge"))
        .ok_or_else(|| anyhow::anyhow!("bridge missing from lock"))?;
    assert_eq!(bridge["version"].as_str(), Some("2.0.0"));
    assert_eq!(bridge["dependencies"][0]["name"].as_str(), Some("legacy"));
    let legacy = packages
        .iter()
        .find(|package| package["name"].as_str() == Some("legacy"))
        .ok_or_else(|| anyhow::anyhow!("legacy missing from lock"))?;
    assert_eq!(legacy["source"]["virtual"].as_str(), Some("members/legacy"));

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
        {configuration}

        [tool.uv.workspace.resolution-axes.runtime]
        legacy = {{ members = ["legacy"] }}
        modern = {{ members = ["modern"] }}
    "#})?;
    context.lock().args(AXES).assert().success();
    uv_snapshot!(context.filters(), context.export().args(AXES).args([
        "--frozen", "--package", "shared", "--resolution-axis", "runtime=modern",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    bridge==1.0.0
    ");
    let original = context.read("uv.lock");
    context.lock().args(AXES).arg("--locked").assert().success();
    assert_eq!(context.read("uv.lock"), original);
    Ok(())
}

/// Removing axis definitions performs a new ordinary solve; it cannot leave a v3 lock behind or
/// make intrinsically incompatible roots installable by dropping their context metadata.
#[test]
fn workspace_resolution_axes_configuration_removal() -> Result<()> {
    for (one, two, compatible) in [
        (r#""switch-leaf>=1,<3""#, r#""switch-leaf>=1,<2""#, true),
        (r#""switch-leaf==1""#, r#""switch-leaf==2""#, false),
    ] {
        let context = uv_test::test_context!("3.12");
        let configuration = indoc! {r#"
            [tool.uv]
            no-index = true
            find-links = ["wheels"]

            [tool.uv.workspace]
            members = ["members/*"]
        "#};
        let pyproject = context.temp_dir.child("pyproject.toml");
        pyproject.write_str(&formatdoc! {r#"
            {configuration}

            [tool.uv.workspace.resolution-axes.runtime]
            legacy = {{ members = ["one"] }}
            modern = {{ members = ["two"] }}
        "#})?;
        write_member(&context, "one", one)?;
        write_member(&context, "two", two)?;
        write_wheels(
            &context,
            &[("switch-leaf", "1.0.0"), ("switch-leaf", "2.0.0")],
        )?;
        context.lock().args(AXES).assert().success();
        let original = context.read("uv.lock");
        let lock: toml::Value = toml::from_str(&original)?;
        assert_eq!(lock["version"].as_integer(), Some(3));

        pyproject.write_str(configuration)?;
        if compatible {
            context.lock().arg("--offline").assert().success();
            let ordinary = context.read("uv.lock");
            let lock: toml::Value = toml::from_str(&ordinary)?;
            assert_eq!(lock["version"].as_integer(), Some(1));
            assert!(lock.get("workspace-axes").is_none());
            context
                .lock()
                .args(["--offline", "--locked"])
                .assert()
                .success();
            assert_eq!(context.read("uv.lock"), ordinary);
            uv_snapshot!(context.filters(), context.export().args([
                "--offline", "--frozen", "--all-packages",
                "--no-header", "--no-hashes", "--no-annotate",
            ]), @"
            exit_code: 0 (success)
            ----- stdout -----
            switch-leaf==1.0.0
            ");
        } else {
            context.lock().arg("--offline").assert().code(1);
            assert_eq!(context.read("uv.lock"), original);
        }
    }
    Ok(())
}

fn write_empty_group_member(
    context: &TestContext,
    requires_python: &str,
    default_group: &str,
) -> Result<()> {
    context
        .temp_dir
        .child("members/shared/pyproject.toml")
        .write_str(&formatdoc! {r#"
        [project]
        name = "shared"
        version = "0.1.0"
        requires-python = ">=3.12,<3.14"

        [dependency-groups]
        empty = []
        combined = [{{ include-group = "empty" }}]
        dormant = []

        [tool.uv]
        package = false
        default-groups = ["{default_group}"]

        [tool.uv.dependency-groups]
        empty = {{ requires-python = "{requires_python}" }}
        combined = {{ requires-python = "<3.14" }}
    "#})?;
    Ok(())
}

fn empty_group_workspace(context: &TestContext) -> Result<()> {
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv]
        no-index = true
        default-groups = []

        [tool.uv.workspace]
        members = ["members/*"]

        [tool.uv.workspace.resolution-axes.runtime]
        legacy = { members = ["legacy"] }
        modern = { members = ["modern"] }
    "#})?;
    write_member(context, "legacy", "")?;
    write_member(context, "modern", "")?;
    write_empty_group_member(context, ">=3.13,<3.14", "combined")
}

/// Frozen commands retain the Python policy and defaults of empty dependency groups even when
/// the selected member's project metadata is absent from disk.
#[test]
fn workspace_resolution_axes_frozen_empty_group_python() -> Result<()> {
    let context =
        uv_test::test_context_with_versions!(&["3.12", "3.13"]).with_filtered_python_sources();
    empty_group_workspace(&context)?;
    context
        .lock()
        .args(AXES)
        .args(["--python", "3.12"])
        .assert()
        .success();
    fs_err::rename(
        context.temp_dir.child("members/shared"),
        context.temp_dir.child("held-shared"),
    )?;

    for group in [Some("combined"), None] {
        let mut command = context.run();
        command
            .args(AXES)
            .args(["--frozen", "--package", "shared", "--python", "3.12"]);
        if let Some(group) = group {
            command.args(["--only-group", group]);
        }
        let output = command.args(["python", "--version"]).output()?;
        output.clone().assert().code(2);
        let stderr = String::from_utf8(output.stderr)?;
        assert!(
            stderr.contains("incompatible with the project's Python requirement"),
            "unexpected error: {stderr}"
        );
        assert!(
            stderr.contains("Python requirement: `==3.13.*`"),
            "unexpected error: {stderr}"
        );
    }

    let output = context
        .run()
        .args(AXES)
        .args([
            "--frozen",
            "--package",
            "shared",
            "--python",
            "3.13",
            "python",
            "-c",
            "import sys; print('%d.%d' % sys.version_info[:2])",
        ])
        .output()?;
    output.clone().assert().success();
    assert_eq!(String::from_utf8(output.stdout)?, "3.13\n");
    Ok(())
}

/// A changed empty-group bound or default cannot pass a checked lock merely because the resolved
/// package graph is otherwise identical.
#[test]
fn workspace_resolution_axes_empty_group_metadata_freshness() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    empty_group_workspace(&context)?;
    context.lock().args(AXES).assert().success();
    let original = context.read("uv.lock");

    write_empty_group_member(&context, ">=3.12,<3.13", "combined")?;
    context.lock().args(AXES).arg("--locked").assert().code(1);
    assert_eq!(context.read("uv.lock"), original);
    context.lock().args(AXES).assert().success();
    let changed_bound = context.read("uv.lock");
    assert_ne!(changed_bound, original);
    context.lock().args(AXES).arg("--locked").assert().success();
    assert_eq!(context.read("uv.lock"), changed_bound);

    write_empty_group_member(&context, ">=3.12,<3.13", "dormant")?;
    context.lock().args(AXES).arg("--locked").assert().code(1);
    assert_eq!(context.read("uv.lock"), changed_bound);
    context.lock().args(AXES).assert().success();
    let changed_defaults = context.read("uv.lock");
    assert_ne!(changed_defaults, changed_bound);
    context.lock().args(AXES).arg("--locked").assert().success();
    assert_eq!(context.read("uv.lock"), changed_defaults);
    Ok(())
}

/// Inheriting one root group does not reactivate a different root group shadowed by the selected
/// member, including when those two groups have incompatible Python requirements.
#[test]
fn workspace_resolution_axes_group_python_shadowing() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "root"
        version = "0.1.0"
        requires-python = ">=3.12,<3.14"

        [dependency-groups]
        lint = []
        docs = ["common-leaf"]

        [tool.uv]
        package = false
        no-index = true
        find-links = ["wheels"]
        default-groups = []

        [tool.uv.dependency-groups]
        lint = { requires-python = "==3.13.*" }
        docs = { requires-python = ">=3.12,<3.14" }

        [tool.uv.workspace]
        members = ["members/member"]

        [tool.uv.workspace.resolution-axes.runtime]
        selected = { members = ["member"] }
        other = {}
    "#})?;
    context
        .temp_dir
        .child("members/member/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "member"
        version = "0.1.0"
        requires-python = ">=3.12,<3.14"

        [dependency-groups]
        lint = []

        [tool.uv]
        package = false
        default-groups = []

        [tool.uv.dependency-groups]
        lint = { requires-python = "==3.12.*" }
    "#})?;
    write_wheels(&context, &[("common-leaf", "1.0.0")])?;
    context.lock().args(AXES).assert().success();
    let output = context
        .export()
        .args(AXES)
        .args([
            "--package",
            "member",
            "--group",
            "lint",
            "--group",
            "docs",
            "--python",
            "3.12",
            "--no-header",
            "--no-hashes",
            "--no-annotate",
        ])
        .output()?;
    output.clone().assert().success();
    assert_eq!(String::from_utf8(output.stdout)?, "common-leaf==1.0.0\n");

    // The frozen command continues to use the recorded root-group owner even if its declaration
    // has been removed from the live workspace metadata.
    context.temp_dir.child("pyproject.toml").write_str(
        &context
            .read("pyproject.toml")
            .replace("docs = [\"common-leaf\"]\n", ""),
    )?;
    let output = context
        .export()
        .args(AXES)
        .args([
            "--frozen",
            "--package",
            "member",
            "--group",
            "lint",
            "--group",
            "docs",
            "--no-header",
            "--no-hashes",
            "--no-annotate",
        ])
        .output()?;
    output.clone().assert().success();
    assert_eq!(String::from_utf8(output.stdout)?, "common-leaf==1.0.0\n");
    Ok(())
}

/// Anonymous workspace-root groups remain selectable from a frozen lock after their live
/// declarations are removed, including groups whose resolved dependency list is empty.
#[test]
fn workspace_resolution_axes_frozen_anonymous_root_groups() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    empty_group_workspace(&context)?;
    let base = context.read("pyproject.toml").replace(
        "no-index = true\n",
        "no-index = true\nfind-links = [\"wheels\"]\n",
    );
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
        {base}

        [dependency-groups]
        root-empty = []
        root-docs = ["common-leaf"]

        [tool.uv.dependency-groups]
        root-empty = {{ requires-python = ">=3.12,<3.14" }}
    "#})?;
    write_wheels(&context, &[("common-leaf", "1.0.0")])?;
    context.lock().args(AXES).assert().success();
    let locked = context.read("uv.lock");
    context.temp_dir.child("pyproject.toml").write_str(&base)?;

    for (group, expected) in [("root-empty", ""), ("root-docs", "common-leaf==1.0.0\n")] {
        let output = context
            .export()
            .args(AXES)
            .args([
                "--frozen",
                "--package",
                "shared",
                "--only-group",
                group,
                "--no-header",
                "--no-hashes",
                "--no-annotate",
            ])
            .output()?;
        output.clone().assert().success();
        assert_eq!(String::from_utf8(output.stdout)?, expected);
        assert_eq!(context.read("uv.lock"), locked);
    }
    Ok(())
}

/// A root owner activated for one inherited group does not activate another group shadowed by
/// the selected member when checking an ordinary dependency-group conflict set.
#[test]
fn workspace_resolution_axes_shadowed_root_group_conflict() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "root"
        version = "0.1.0"
        requires-python = ">=3.12"

        [dependency-groups]
        lint = ["root-lint"]
        docs = ["common-leaf"]

        [tool.uv]
        package = false
        no-index = true
        find-links = ["wheels"]
        default-groups = []
        conflicts = [[{ group = "lint" }, { group = "docs" }]]

        [tool.uv.workspace]
        members = ["members/member"]

        [tool.uv.workspace.resolution-axes.runtime]
        selected = { members = ["member"] }
        other = {}
    "#})?;
    context
        .temp_dir
        .child("members/member/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "member"
        version = "0.1.0"
        requires-python = ">=3.12"

        [dependency-groups]
        lint = ["member-lint"]

        [tool.uv]
        package = false
        default-groups = []
    "#})?;
    write_wheels(
        &context,
        &[
            ("common-leaf", "1.0.0"),
            ("member-lint", "1.0.0"),
            ("root-lint", "1.0.0"),
        ],
    )?;
    context.lock().args(AXES).assert().success();

    for frozen in [false, true] {
        let mut command = context.export();
        command.args(AXES);
        if frozen {
            command.arg("--frozen");
        }
        let output = command
            .args([
                "--package",
                "member",
                "--group",
                "lint",
                "--group",
                "docs",
                "--no-header",
                "--no-hashes",
                "--no-annotate",
            ])
            .output()?;
        output.clone().assert().success();
        assert_eq!(
            String::from_utf8(output.stdout)?,
            "common-leaf==1.0.0\nmember-lint==1.0.0\n"
        );
    }

    let output = context
        .export()
        .args(AXES)
        .args([
            "--frozen",
            "--package",
            "root",
            "--group",
            "lint",
            "--group",
            "docs",
            "--no-header",
            "--no-hashes",
            "--no-annotate",
        ])
        .output()?;
    output.clone().assert().code(2);
    let stderr = String::from_utf8(output.stderr)?;
    assert!(
        stderr.contains("Groups `docs` and `lint` are incompatible"),
        "unexpected error: {stderr}"
    );
    Ok(())
}

/// Universal viewers without an axis-aware schema must fail before dropping edges, exposing
/// private selectors, performing latest-version lookups, or initializing a concrete environment.
#[test]
fn workspace_resolution_axes_unadapted_viewers_fail_closed() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    raw_viewer_workspace(&context)?;
    context.lock().args(AXES).assert().success();
    let locked = context.read("uv.lock");

    for arguments in [
        &[][..],
        &["--universal"][..],
        &["--format", "json", "--preview-features", "json-output"][..],
        &[
            "--universal",
            "--format",
            "json",
            "--preview-features",
            "json-output",
        ][..],
        &["--outdated"][..],
    ] {
        let output = context
            .tree()
            .args(AXES)
            .arg("--frozen")
            .args(arguments)
            .output()?;
        output.clone().assert().code(2);
        assert!(output.stdout.is_empty());
        assert_eq!(
            String::from_utf8(output.stderr)?,
            "error: `uv tree` does not yet support workspace resolution axes; use `uv export --resolution-axis AXIS=SECTION` to select a resolution\n"
        );
        assert_eq!(context.read("uv.lock"), locked);
    }

    for sync in [false, true] {
        let mut command = context.workspace_metadata();
        command
            .args(AXES)
            .args(["--frozen", "--preview-features", "workspace-metadata"]);
        if sync {
            command.arg("--sync");
        }
        let output = command.output()?;
        output.clone().assert().code(2);
        assert!(output.stdout.is_empty());
        assert_eq!(
            String::from_utf8(output.stderr)?,
            "error: `uv workspace metadata` does not yet support workspace resolution axes; use `uv export --resolution-axis AXIS=SECTION` to select a resolution\n"
        );
        assert!(!context.temp_dir.child(".venv").path().exists());
        assert_eq!(context.read("uv.lock"), locked);
    }

    // Non-frozen commands must also stop once their lock operation produces a v3 lock.
    for (mut command, viewer, arguments) in [
        (context.tree(), "uv tree", &[][..]),
        (
            context.workspace_metadata(),
            "uv workspace metadata",
            &["--sync", "--preview-features", "workspace-metadata"][..],
        ),
    ] {
        command.args(AXES).args(arguments);
        let output = command.output()?;
        output.clone().assert().code(2);
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8(output.stderr)?;
        assert!(
            stderr.ends_with(&format!(
                "error: `{viewer}` does not yet support workspace resolution axes; use `uv export --resolution-axis AXIS=SECTION` to select a resolution\n"
            )),
            "unexpected error: {stderr}"
        );
        assert!(!context.temp_dir.child(".venv").path().exists());
        assert_eq!(context.read("uv.lock"), locked);
    }
    Ok(())
}
