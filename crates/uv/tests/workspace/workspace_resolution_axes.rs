use anyhow::{Context, Result};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::fixture::{FileWriteStr, PathChild, PathCreateDir};
use indoc::{formatdoc, indoc};
use url::Url;
use uv_fs::normalize_path;
use uv_test::archive::write_tar_gz;
use uv_test::{TestContext, uv_snapshot};

use super::workspace_metadata::write_wheel_with_metadata;

fn write_wheels(context: &TestContext, packages: &[(&str, &str, &str)]) -> Result<()> {
    let wheels = context.temp_dir.child("wheels");
    wheels.create_dir_all()?;
    for &(name, version, metadata) in packages {
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

        [tool.uv.workspace.resolution-axes.python]
        py312 = { members = ["legacy-worker"], requires-python = "==3.12.*" }
        py313 = { members = ["hybrid-worker", "modern-worker", "modern-peer"], requires-python = "==3.13.*" }

        [tool.uv.workspace.resolution-axes.sqlalchemy]
        v1 = { members = ["legacy-worker", "hybrid-worker"], constraint-dependencies = ["sqlalchemy>=1,<2", "policy-only==1"] }
        v2 = { members = ["modern-worker", "modern-peer"], constraint-dependencies = ["sqlalchemy>=2,<3"] }

        [tool.uv.workspace.resolution-axes.lib]
        v1 = { members = ["legacy-worker"], constraint-dependencies = ["axis-lib>=1,<2"] }
        v2 = { members = ["hybrid-worker"], constraint-dependencies = ["axis-lib>=2,<3"] }
        v3 = { members = ["modern-worker", "modern-peer"], constraint-dependencies = ["axis-lib>=3,<4"] }

        [tool.uv.sources]
        shared = { workspace = true }
    "#})?;
    for (name, requires_python, dependencies) in [
        ("shared", ">=3.12,<3.14", r#""common-leaf>=1""#),
        (
            "legacy-worker",
            "==3.12.*",
            r#""shared", "python-leaf", "sqlalchemy", "axis-lib""#,
        ),
        (
            "hybrid-worker",
            "==3.13.*",
            r#""shared", "python-leaf", "sqlalchemy", "axis-lib""#,
        ),
        (
            "modern-worker",
            "==3.13.*",
            r#""shared", "python-leaf", "sqlalchemy", "axis-lib""#,
        ),
        ("modern-peer", "==3.13.*", r#""shared""#),
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
    write_wheels(
        context,
        &[
            ("common-leaf", "1.0.0", ""),
            ("common-leaf", "2.0.0", ""),
            ("python-leaf", "1.0.0", "Requires-Python: >=3.12\n"),
            ("python-leaf", "2.0.0", "Requires-Python: >=3.13\n"),
            ("sqlalchemy", "1.0.0", ""),
            ("sqlalchemy", "2.0.0", ""),
            ("axis-lib", "1.0.0", ""),
            ("axis-lib", "2.0.0", ""),
            ("axis-lib", "3.0.0", ""),
        ],
    )
}

fn alignment_workspace(context: &TestContext, force_split: bool) -> Result<()> {
    let (wide_policy, narrow_policy, wide_dependencies, narrow_dependencies) = if force_split {
        (
            r#", constraint-dependencies = ["branch-leaf==2"]"#,
            r#", constraint-dependencies = ["branch-leaf==1"]"#,
            r#""branch-leaf>=1", "common-leaf>=1,<4,!=2""#,
            r#""branch-leaf>=1", "common-leaf>=1,<3""#,
        )
    } else {
        ("", "", r#""common-leaf>=1,<3""#, r#""common-leaf>=1,<2""#)
    };
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
        a-wide = {{ members = ["wide"]{wide_policy} }}
        z-narrow = {{ members = ["narrow"]{narrow_policy} }}
    "#})?;
    for (name, dependencies) in [("wide", wide_dependencies), ("narrow", narrow_dependencies)] {
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
    }
    write_wheels(
        context,
        &[
            ("common-leaf", "1.0.0", ""),
            ("common-leaf", "2.0.0", ""),
            ("common-leaf", "3.0.0", ""),
            ("branch-leaf", "1.0.0", ""),
            ("branch-leaf", "2.0.0", ""),
        ],
    )
}

fn python_build_workspace(context: &TestContext) -> Result<()> {
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv]
        no-index = true

        [tool.uv.workspace]
        members = ["members/*"]

        [tool.uv.workspace.resolution-axes.python]
        py312 = { members = ["legacy"], requires-python = "==3.12.*" }
        py313 = { members = ["modern"], requires-python = "==3.13.*" }
    "#})?;
    for (name, minor) in [("legacy", 12), ("modern", 13)] {
        let member = context.temp_dir.child(format!("members/{name}"));
        member.child("pyproject.toml").write_str(&formatdoc! {r#"
            [project]
            name = "{name}"
            version = "0.1.0"
            requires-python = "==3.{minor}.*"

            [build-system]
            requires = []
            build-backend = "backend"
            backend-path = ["."]
        "#})?;
        write_wheel_with_metadata(
            member
                .child(format!("{name}-0.1.0-py3-none-any.whl"))
                .path(),
            name,
            "0.1.0",
            &format!("{name}-0.1.0"),
            &format!("Requires-Python: ==3.{minor}.*\n"),
            &[],
        )?;
        member.child("backend.py").write_str(&formatdoc! {r#"
            import shutil
            import sys
            from pathlib import Path


            def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
                if sys.version_info[:2] != (3, {minor}):
                    raise RuntimeError("Expected Python 3.{minor}, got " + sys.version.split()[0])
                Path(__file__).with_name("build-python").write_text(
                    ".".join(map(str, sys.version_info[:2])), encoding="utf-8"
                )
                source = Path(__file__).with_name("{name}-0.1.0-py3-none-any.whl")
                shutil.copyfile(source, Path(wheel_directory) / source.name)
                return source.name
        "#})?;
    }
    Ok(())
}

fn locked_versions(context: &TestContext, name: &str) -> Result<Vec<String>> {
    let lock: toml::Value = toml::from_str(&context.read("uv.lock"))?;
    let packages = lock
        .get("package")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("lock packages missing"))?;
    packages
        .iter()
        .filter(|package| package.get("name").and_then(toml::Value::as_str) == Some(name))
        .map(|package| {
            package
                .get("version")
                .and_then(toml::Value::as_str)
                .map(ToOwned::to_owned)
                .ok_or_else(|| anyhow::anyhow!("lock package `{name}` has no version"))
        })
        .collect()
}

/// Compatible axis products stay symbolic, even when their concrete root sets differ. This
/// workspace has more than sixteen million complete selections, but needs only one shared solve.
#[test]
fn workspace_resolution_axes_compatible_product_stays_symbolic() -> Result<()> {
    const AXES: usize = 24;

    let context = uv_test::test_context!("3.12");
    let mut pyproject = indoc! {r#"
        [tool.uv]
        no-index = true
        find-links = ["wheels"]

        [tool.uv.workspace]
        members = ["members/*"]
    "#}
    .to_owned();
    for axis in 0..AXES {
        pyproject.push_str(&formatdoc! {r#"

            [tool.uv.workspace.resolution-axes.axis-{axis:02}]
            left = {{ members = ["axis-{axis:02}-left"] }}
            right = {{ members = ["axis-{axis:02}-right"] }}
        "#});
        for (section, dependency) in [
            ("left", "common-leaf>=1,<3"),
            ("right", "common-leaf>=1,<2"),
        ] {
            let name = format!("axis-{axis:02}-{section}");
            context
                .temp_dir
                .child(format!("members/{name}/pyproject.toml"))
                .write_str(&formatdoc! {r#"
                [project]
                name = "{name}"
                version = "0.1.0"
                requires-python = ">=3.12"
                dependencies = ["{dependency}"]

                [tool.uv]
                package = false
            "#})?;
        }
    }
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&pyproject)?;
    context
        .temp_dir
        .child("members/shared/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "shared"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["common-leaf>=1"]

        [tool.uv]
        package = false
    "#})?;
    write_wheels(
        &context,
        &[("common-leaf", "1.0.0", ""), ("common-leaf", "2.0.0", "")],
    )?;

    uv_snapshot!(context.filters(), context.lock().args(["--offline", "--preview-features", "workspace-resolution-axes"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 50 packages in [TIME]
    ");
    assert_eq!(locked_versions(&context, "common-leaf")?, ["1.0.0"]);

    let serialized = context.read("uv.lock");
    let lock: toml::Value = toml::from_str(&serialized)?;
    let contexts = lock
        .get("workspace-axes")
        .and_then(|axes| axes.get("context"))
        .and_then(toml::Value::as_array)
        .context("locked workspace contexts")?;
    assert_eq!(contexts.len(), 1);
    let domain = contexts
        .first()
        .and_then(|context| context.get("domain"))
        .and_then(toml::Value::as_table)
        .context("symbolic axis domain")?;
    assert_eq!(domain.len(), AXES);
    for axis in 0..AXES {
        assert_eq!(
            domain.get(&format!("axis-{axis:02}")),
            Some(&toml::Value::Array(vec!["left".into(), "right".into()])),
        );
    }

    context
        .lock()
        .args([
            "--offline",
            "--locked",
            "--preview-features",
            "workspace-resolution-axes",
        ])
        .assert()
        .success();
    assert_eq!(context.read("uv.lock"), serialized);

    let selection = (0..AXES)
        .flat_map(|axis| {
            let section = if axis % 2 == 0 { "left" } else { "right" };
            [
                "--resolution-axis".to_owned(),
                format!("axis-{axis:02}={section}"),
            ]
        })
        .collect::<Vec<_>>();
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--all-matching-packages", "--no-header", "--no-hashes", "--no-annotate",
    ]).args(selection), @"
    exit_code: 0 (success)
    ----- stdout -----
    common-leaf==1.0.0
    ");
    Ok(())
}

/// A conflict on a late axis must not expand earlier, irrelevant axes into concrete selections.
#[test]
fn workspace_resolution_axes_conflict_directed_split_stays_symbolic() -> Result<()> {
    const IRRELEVANT_AXES: usize = 16;

    let context = uv_test::test_context!("3.12");
    alignment_workspace(&context, true)?;
    let mut pyproject = context
        .read("pyproject.toml")
        .replace("resolution-axes.runtime", "resolution-axes.z-divergent");
    for axis in 0..IRRELEVANT_AXES {
        pyproject.push_str(&formatdoc! {r"

            [tool.uv.workspace.resolution-axes.axis-{axis:02}]
            left = {{}}
            right = {{}}
        "});
    }
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&pyproject)?;

    uv_snapshot!(context.filters(), context.lock().args(["--offline", "--preview-features", "workspace-resolution-axes"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    ");
    assert_eq!(locked_versions(&context, "common-leaf")?, ["1.0.0"]);
    assert_eq!(
        locked_versions(&context, "branch-leaf")?,
        ["1.0.0", "2.0.0"]
    );

    let lock: toml::Value = toml::from_str(&context.read("uv.lock"))?;
    let contexts = lock
        .get("workspace-axes")
        .and_then(|axes| axes.get("context"))
        .and_then(toml::Value::as_array)
        .context("locked workspace contexts")?;
    assert_eq!(contexts.len(), 2);
    for (context, section) in contexts.iter().zip(["a-wide", "z-narrow"]) {
        let domain = context
            .get("domain")
            .and_then(toml::Value::as_table)
            .context("symbolic axis domain")?;
        assert_eq!(domain.len(), IRRELEVANT_AXES + 1);
        assert_eq!(
            domain.get("z-divergent"),
            Some(&toml::Value::Array(vec![section.into()])),
        );
        for axis in 0..IRRELEVANT_AXES {
            assert_eq!(
                domain.get(&format!("axis-{axis:02}")),
                Some(&toml::Value::Array(vec!["left".into(), "right".into()])),
            );
        }
    }

    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--all-matching-packages", "--resolution-axis", "z-divergent=z-narrow",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    branch-leaf==1.0.0
    common-leaf==1.0.0
    ");
    Ok(())
}

/// Compatible sections are solved together so a restrictive root can select an older common
/// dependency without creating unnecessary versions in the other section.
#[test]
fn workspace_resolution_axes_shared_first_alignment() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    alignment_workspace(&context, false)?;
    uv_snapshot!(context.filters(), context.lock().args(["--offline", "--preview-features", "workspace-resolution-axes"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    assert_eq!(locked_versions(&context, "common-leaf")?, ["1.0.0"]);

    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--all-matching-packages", "--resolution-axis", "runtime=a-wide",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    common-leaf==1.0.0
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--all-matching-packages", "--resolution-axis", "runtime=z-narrow",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    common-leaf==1.0.0
    ");
    Ok(())
}

/// A required split on one package does not require an unrelated package to diverge. The common
/// version is not the highest version in either section, so one-way sibling preferences cannot
/// find it regardless of the order in which the sections are solved.
#[test]
fn workspace_resolution_axes_reconcile_split_versions() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    alignment_workspace(&context, true)?;
    uv_snapshot!(context.filters(), context.lock().args(["--offline", "--preview-features", "workspace-resolution-axes"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    ");
    assert_eq!(locked_versions(&context, "common-leaf")?, ["1.0.0"]);
    assert_eq!(
        locked_versions(&context, "branch-leaf")?,
        ["1.0.0", "2.0.0"]
    );

    let original = context.read("uv.lock");
    context
        .lock()
        .args([
            "--offline",
            "--locked",
            "--preview-features",
            "workspace-resolution-axes",
        ])
        .assert()
        .success();
    assert_eq!(original, context.read("uv.lock"));
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--all-matching-packages", "--resolution-axis", "runtime=a-wide",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    branch-leaf==2.0.0
    common-leaf==1.0.0
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--all-matching-packages", "--resolution-axis", "runtime=z-narrow",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    branch-leaf==1.0.0
    common-leaf==1.0.0
    ");

    context
        .lock()
        .args([
            "--offline",
            "--upgrade",
            "--preview-features",
            "workspace-resolution-axes",
        ])
        .assert()
        .success();
    assert_eq!(locked_versions(&context, "common-leaf")?, ["1.0.0"]);
    assert_eq!(
        locked_versions(&context, "branch-leaf")?,
        ["1.0.0", "2.0.0"]
    );
    Ok(())
}

/// `lowest-direct` chooses versions independently for each context's direct dependency set.
#[test]
fn workspace_resolution_axes_lowest_direct() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    alignment_workspace(&context, true)?;
    context
        .temp_dir
        .child("members/narrow/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "narrow"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["branch-leaf>=1", "wrapper>=1"]

        [tool.uv]
        package = false
    "#})?;
    write_wheels(
        &context,
        &[("wrapper", "1.0.0", "Requires-Dist: common-leaf>=1,<3\n")],
    )?;

    uv_snapshot!(context.filters(), context.lock().args([
        "--offline", "--preview-features", "workspace-resolution-axes",
        "--resolution", "lowest-direct",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 7 packages in [TIME]
    ");
    assert_eq!(
        locked_versions(&context, "common-leaf")?,
        ["1.0.0", "2.0.0"]
    );
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--package", "wide", "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    branch-leaf==2.0.0
    common-leaf==1.0.0
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--package", "narrow", "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    branch-leaf==1.0.0
    common-leaf==2.0.0
    wrapper==1.0.0
    ");
    Ok(())
}

/// A shared solve must not classify a dependency as direct in a lane that only reaches it
/// transitively, even when the lanes have no incompatible version constraints.
#[test]
fn workspace_resolution_axes_lowest_direct_shared_roots() -> Result<()> {
    const IRRELEVANT_AXES: usize = 3;

    let context = uv_test::test_context!("3.12");
    alignment_workspace(&context, false)?;
    let mut pyproject = context.read("pyproject.toml");
    for axis in 0..IRRELEVANT_AXES {
        pyproject.push_str(&formatdoc! {r"

            [tool.uv.workspace.resolution-axes.aa-irrelevant-{axis}]
            left = {{}}
            right = {{}}
        "});
    }
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&pyproject)?;
    context
        .temp_dir
        .child("members/narrow/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "narrow"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["wrapper>=1"]

        [tool.uv]
        package = false
    "#})?;
    write_wheels(
        &context,
        &[("wrapper", "1.0.0", "Requires-Dist: common-leaf>=1,<3\n")],
    )?;

    uv_snapshot!(context.filters(), context.lock().args([
        "--offline", "--preview-features", "workspace-resolution-axes",
        "--resolution", "lowest-direct",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    ");
    assert_eq!(
        locked_versions(&context, "common-leaf")?,
        ["1.0.0", "2.0.0"]
    );
    let lock: toml::Value = toml::from_str(&context.read("uv.lock"))?;
    let contexts = lock
        .get("workspace-axes")
        .and_then(|axes| axes.get("context"))
        .and_then(toml::Value::as_array)
        .context("locked workspace contexts")?;
    assert_eq!(contexts.len(), 2);
    for context in contexts {
        let domain = context
            .get("domain")
            .and_then(toml::Value::as_table)
            .context("symbolic axis domain")?;
        for axis in 0..IRRELEVANT_AXES {
            assert_eq!(
                domain.get(&format!("aa-irrelevant-{axis}")),
                Some(&toml::Value::Array(vec!["left".into(), "right".into()])),
            );
        }
    }
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--package", "wide", "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    common-leaf==1.0.0
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--package", "narrow", "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    common-leaf==2.0.0
    wrapper==1.0.0
    ");
    Ok(())
}

/// Axis definitions reject ambiguous membership and invalid references before resolving packages.
#[test]
fn workspace_resolution_axes_configuration_errors() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    alignment_workspace(&context, false)?;
    let original = context.read("pyproject.toml");
    let pyproject = context.temp_dir.child("pyproject.toml");

    pyproject.write_str(
        &original.replace(r#"members = ["narrow"]"#, r#"members = ["narrow", "wide"]"#),
    )?;
    uv_snapshot!(context.filters(), context.lock().args(["--offline", "--preview-features", "workspace-resolution-axes"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Workspace member `wide` belongs to both `a-wide` and `z-narrow` on resolution axis `runtime`
    ");

    pyproject
        .write_str(&original.replace(r#"members = ["narrow"]"#, r#"members = ["missing"]"#))?;
    uv_snapshot!(context.filters(), context.lock().args(["--offline", "--preview-features", "workspace-resolution-axes"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Resolution axis `runtime` section `z-narrow` contains unknown workspace member `missing`
    ");

    pyproject.write_str(&formatdoc! {r#"
        {original}

        [[tool.uv.workspace.groups]]
        name = "legacy"
        members = ["wide"]
    "#})?;
    uv_snapshot!(context.filters(), context.lock().args(["--offline", "--preview-features", "workspace-resolution-axes"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `tool.uv.workspace.groups` and `tool.uv.workspace.resolution-axes` cannot be combined
    ");
    Ok(())
}

/// Explicit selectors are validated against the locked axis definitions, including contradictory
/// repeated assignments to the same axis.
#[test]
fn workspace_resolution_axes_selector_errors() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    alignment_workspace(&context, false)?;
    context
        .lock()
        .args([
            "--offline",
            "--preview-features",
            "workspace-resolution-axes",
        ])
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--all-matching-packages", "--resolution-axis", "missing=v1",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Resolution axis `missing` is not defined
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--all-matching-packages", "--resolution-axis", "runtime=missing",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Resolution axis `runtime` has no section `missing`
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--all-matching-packages", "--resolution-axis", "runtime=a-wide",
        "--resolution-axis", "runtime=z-narrow",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Resolution axis `runtime` selects both `a-wide` and `z-narrow`
    ");
    Ok(())
}

/// An explicit target must remain present even when its requirements happen to be compatible with
/// a member assigned to a different section.
#[test]
fn workspace_resolution_axes_strict_targets() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    alignment_workspace(&context, false)?;
    context
        .lock()
        .args([
            "--offline",
            "--preview-features",
            "workspace-resolution-axes",
        ])
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--package", "wide", "--package", "narrow",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Resolution axis `runtime` selects both `z-narrow` and `a-wide`
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--all-packages", "--resolution-axis", "runtime=a-wide",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Resolution axis `runtime` selects both `z-narrow` and `a-wide`
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--all-matching-packages",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The matching workspace members depend on unresolved axis `runtime`; select a section with `--resolution-axis AXIS=SECTION`
    ");
    Ok(())
}

/// Enabling the preview without declaring axes leaves ordinary workspace locking and targeting
/// unchanged.
#[test]
fn workspace_resolution_axes_unconfigured_workspace() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    alignment_workspace(&context, false)?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv]
        no-index = true
        find-links = ["wheels"]

        [tool.uv.workspace]
        members = ["members/*"]
    "#})?;

    uv_snapshot!(context.filters(), context.lock().arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    let serialized = context.read("uv.lock");
    let lock: toml::Value = toml::from_str(&serialized)?;
    assert_eq!(
        lock.get("version").and_then(toml::Value::as_integer),
        Some(1)
    );
    assert!(lock.get("workspace-axes").is_none());
    context
        .lock()
        .args([
            "--offline",
            "--locked",
            "--preview-features",
            "workspace-resolution-axes",
        ])
        .assert()
        .success();
    assert_eq!(context.read("uv.lock"), serialized);

    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--all-packages", "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    common-leaf==1.0.0
    ");
    Ok(())
}

/// Bulk path selectors assign every matching workspace member to the same section.
#[test]
fn workspace_resolution_axes_member_paths() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    alignment_workspace(&context, false)?;
    context.temp_dir.child("pyproject.toml").write_str(
        &context
            .read("pyproject.toml")
            .replace(r#"members = ["wide"]"#, r#"member-paths = ["members/w*"]"#),
    )?;
    context
        .temp_dir
        .child("members/wide-peer/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "wide-peer"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["common-leaf>=1,<3"]

        [tool.uv]
        package = false
    "#})?;
    uv_snapshot!(context.filters(), context.lock().args(["--offline", "--preview-features", "workspace-resolution-axes"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--package", "wide-peer",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    common-leaf==1.0.0
    ");
    Ok(())
}

/// A workspace dependency cannot bring a member from an incompatible section into a context.
#[test]
fn workspace_resolution_axes_cross_lane_dependency() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv.workspace]
        members = ["members/*"]

        [tool.uv.workspace.resolution-axes.runtime]
        old = { members = ["old"] }
        new = { members = ["new"] }

        [tool.uv.sources]
        new = { workspace = true }
    "#})?;
    for (name, dependencies) in [("old", r#""new""#), ("new", "")] {
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
    }
    uv_snapshot!(context.filters(), context.lock().args(["--offline", "--preview-features", "workspace-resolution-axes"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Workspace member `new` is required outside its resolution-axis assignments in context `runtime=old`
    ");
    Ok(())
}

/// A member's unavailable local identity does not exclude a registry distribution or a different
/// source directory with the same normalized package name and version. Its local version also does
/// not become a sibling preference for the unrelated registry distribution.
#[test]
fn workspace_resolution_axes_member_source_identity() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let workspace = context.temp_dir.child("workspace");
    workspace.child("pyproject.toml").write_str(indoc! {r#"
        [tool.uv]
        no-index = true
        find-links = ["../wheels"]

        [tool.uv.workspace]
        members = ["members/*"]

        [tool.uv.workspace.resolution-axes.runtime]
        workspace = { members = ["legacy"] }
        registry = { members = ["registry-consumer"] }
        directory = { members = ["directory-consumer"] }
    "#})?;
    for (name, dependencies, sources) in [
        ("legacy", "", ""),
        ("registry-consumer", r#""registry-bridge==1""#, ""),
        (
            "directory-consumer",
            r#""directory-bridge""#,
            indoc! {r#"
                [tool.uv.sources]
                directory-bridge = { path = "../../../external/bridge" }
            "#},
        ),
    ] {
        workspace
            .child(format!("members/{name}/pyproject.toml"))
            .write_str(&formatdoc! {r#"
            [project]
            name = "{name}"
            version = "0.1.0"
            requires-python = ">=3.12"
            dependencies = [{dependencies}]

            [tool.uv]
            package = false

            {sources}
        "#})?;
    }
    context
        .temp_dir
        .child("external/bridge/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "directory-bridge"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["legacy==0.1.0"]

        [tool.uv]
        package = false

        [tool.uv.sources]
        legacy = { path = "../legacy" }
    "#})?;
    context
        .temp_dir
        .child("external/legacy/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "legacy"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["path-leaf==1"]

        [tool.uv]
        package = false
    "#})?;
    write_wheels(
        &context,
        &[
            ("legacy", "0.1.0", ""),
            ("legacy", "1.0.0", ""),
            ("path-leaf", "1.0.0", ""),
            ("registry-bridge", "1.0.0", "Requires-Dist: legacy>=0.1.0\n"),
        ],
    )?;

    uv_snapshot!(context.filters(), context.lock().current_dir(&workspace).args([
        "--offline", "--preview-features", "workspace-resolution-axes",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Resolved 8 packages in [TIME]
    ");
    let lock: toml::Value = toml::from_str(&context.read("workspace/uv.lock"))?;
    let mut sources = lock
        .get("package")
        .and_then(toml::Value::as_array)
        .context("lock packages")?
        .iter()
        .filter(|package| package.get("name").and_then(toml::Value::as_str) == Some("legacy"))
        .map(|package| {
            package
                .get("source")
                .cloned()
                .context("source of legacy package")
        })
        .collect::<Result<Vec<_>>>()?;
    sources.sort_by_key(toml::Value::to_string);
    insta::assert_json_snapshot!(sources, @r#"
    [
      {
        "registry": "../wheels"
      },
      {
        "virtual": "../external/legacy"
      },
      {
        "virtual": "members/legacy"
      }
    ]
    "#);
    uv_snapshot!(context.filters(), context.export().current_dir(&workspace).args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--package", "registry-consumer", "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    legacy==1.0.0
    registry-bridge==1.0.0
    ");
    uv_snapshot!(context.filters(), context.export().current_dir(&workspace).args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--package", "directory-consumer", "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    path-leaf==1.0.0
    ");
    uv_snapshot!(context.filters(), context.export().current_dir(&workspace).args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--package", "legacy", "--no-header", "--no-hashes", "--no-annotate",
    ]), @"exit_code: 0 (success)");
    Ok(())
}

/// A member's assignments are intersected across independent axes, while unassigned members
/// remain available in every context. Section constraints narrow dependencies without adding them.
#[test]
fn workspace_resolution_axes_lock_and_export() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    workspace(&context)?;
    uv_snapshot!(context.filters(), context.lock().args(["--offline", "--preview-features", "workspace-resolution-axes"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 13 packages in [TIME]
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
        "axis-lib",
        "1.0.0"
      ],
      [
        "axis-lib",
        "2.0.0"
      ],
      [
        "axis-lib",
        "3.0.0"
      ],
      [
        "common-leaf",
        "2.0.0"
      ],
      [
        "hybrid-worker",
        "0.1.0"
      ],
      [
        "legacy-worker",
        "0.1.0"
      ],
      [
        "modern-peer",
        "0.1.0"
      ],
      [
        "modern-worker",
        "0.1.0"
      ],
      [
        "python-leaf",
        "1.0.0"
      ],
      [
        "python-leaf",
        "2.0.0"
      ],
      [
        "shared",
        "0.1.0"
      ],
      [
        "sqlalchemy",
        "1.0.0"
      ],
      [
        "sqlalchemy",
        "2.0.0"
      ]
    ]
    "#);
    uv_snapshot!(context.filters(), context.lock().args(["--offline", "--locked", "--preview-features", "workspace-resolution-axes"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 13 packages in [TIME]
    ");
    assert_eq!(original, context.read("uv.lock"));

    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--all-matching-packages",
        "--resolution-axis", "python=py312",
        "--resolution-axis", "sqlalchemy=v1",
        "--resolution-axis", "lib=v1",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    axis-lib==1.0.0
    common-leaf==2.0.0
    python-leaf==1.0.0
    sqlalchemy==1.0.0
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--all-matching-packages",
        "--resolution-axis", "python=py313",
        "--resolution-axis", "sqlalchemy=v1",
        "--resolution-axis", "lib=v2",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    axis-lib==2.0.0
    common-leaf==2.0.0
    python-leaf==2.0.0
    sqlalchemy==1.0.0
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--all-matching-packages",
        "--resolution-axis", "python=py313",
        "--resolution-axis", "sqlalchemy=v2",
        "--resolution-axis", "lib=v3",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    axis-lib==3.0.0
    common-leaf==2.0.0
    python-leaf==2.0.0
    sqlalchemy==2.0.0
    ");

    // No worker matches this complete selection; only the shared root remains.
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--all-matching-packages",
        "--resolution-axis", "python=py312",
        "--resolution-axis", "sqlalchemy=v2",
        "--resolution-axis", "lib=v3",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    common-leaf==2.0.0
    ");
    Ok(())
}

/// The `fewest` strategy can align a compatible older version across different Python sections.
#[test]
fn workspace_resolution_axes_fewest() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    workspace(&context)?;
    uv_snapshot!(context.filters(), context.lock().args([
        "--offline", "--preview-features", "workspace-resolution-axes",
        "--fork-strategy", "fewest",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 12 packages in [TIME]
    ");
    assert_eq!(locked_versions(&context, "python-leaf")?, ["1.0.0"]);
    assert_eq!(locked_versions(&context, "common-leaf")?, ["2.0.0"]);
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--package", "modern-worker", "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    axis-lib==3.0.0
    common-leaf==2.0.0
    python-leaf==1.0.0
    sqlalchemy==2.0.0
    ");
    Ok(())
}

/// Changing fork strategy invalidates the old strategy's choices even when the newer Python lane
/// sorts first and selects a version that cannot be reused by the older lane.
#[test]
fn workspace_resolution_axes_changed_fork_strategy() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    workspace(&context)?;
    context.temp_dir.child("pyproject.toml").write_str(
        &context
            .read("pyproject.toml")
            .replace("py312 =", "z-old =")
            .replace("py313 =", "a-new ="),
    )?;
    context
        .lock()
        .args([
            "--offline",
            "--preview-features",
            "workspace-resolution-axes",
        ])
        .assert()
        .success();
    assert_eq!(
        locked_versions(&context, "python-leaf")?,
        ["1.0.0", "2.0.0"]
    );

    context
        .lock()
        .args([
            "--offline",
            "--preview-features",
            "workspace-resolution-axes",
            "--fork-strategy",
            "fewest",
        ])
        .assert()
        .success();
    assert_eq!(locked_versions(&context, "python-leaf")?, ["1.0.0"]);
    let fewest = context.read("uv.lock");
    context
        .lock()
        .args([
            "--offline",
            "--locked",
            "--preview-features",
            "workspace-resolution-axes",
            "--fork-strategy",
            "fewest",
        ])
        .assert()
        .success();
    assert_eq!(context.read("uv.lock"), fewest);

    context
        .lock()
        .args([
            "--offline",
            "--preview-features",
            "workspace-resolution-axes",
            "--fork-strategy",
            "requires-python",
        ])
        .assert()
        .success();
    assert_eq!(
        locked_versions(&context, "python-leaf")?,
        ["1.0.0", "2.0.0"]
    );
    Ok(())
}

/// Batch entries combine their own axis assignments with the command's shared assignments.
#[test]
fn workspace_resolution_axes_batch_export() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    workspace(&context)?;
    context
        .lock()
        .args([
            "--offline",
            "--preview-features",
            "workspace-resolution-axes",
        ])
        .assert()
        .success();
    context
        .temp_dir
        .child("exports/batch.toml")
        .write_str(indoc! {r#"
        [[export]]
        output-file = "hybrid.txt"
        all-matching-packages = true
        resolution-axes = { sqlalchemy = "v1", lib = "v2" }

        [[export]]
        output-file = "modern.txt"
        all-matching-packages = true
        resolution-axes = { sqlalchemy = "v2", lib = "v3" }

        [[export]]
        output-file = "shared.txt"
        package = ["shared"]
    "#})?;

    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes,batch-export",
        "--resolution-axis", "python=py313", "--batch", "exports/batch.toml",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"exit_code: 0 (success)");
    insta::assert_snapshot!(context.read("exports/hybrid.txt"), @"
    axis-lib==2.0.0
    common-leaf==2.0.0
    python-leaf==2.0.0
    sqlalchemy==1.0.0
    ");
    insta::assert_snapshot!(context.read("exports/modern.txt"), @"
    axis-lib==3.0.0
    common-leaf==2.0.0
    python-leaf==2.0.0
    sqlalchemy==2.0.0
    ");
    insta::assert_snapshot!(context.read("exports/shared.txt"), @"common-leaf==2.0.0");

    let hybrid = context.read("exports/hybrid.txt");
    let modern = context.read("exports/modern.txt");
    context
        .temp_dir
        .child("exports/batch.toml")
        .write_str(indoc! {r#"
        [[export]]
        output-file = "hybrid.txt"
        all-matching-packages = true
        resolution-axes = { sqlalchemy = "v1", lib = "v2" }

        [[export]]
        output-file = "modern.txt"
        all-matching-packages = true
        resolution-axes = { python = "py312", sqlalchemy = "v1", lib = "v1" }
    "#})?;
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes,batch-export",
        "--resolution-axis", "python=py313", "--batch", "exports/batch.toml",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Resolution axis `python` selects both `py313` and `py312`
    ");
    assert_eq!(context.read("exports/hybrid.txt"), hybrid);
    assert_eq!(context.read("exports/modern.txt"), modern);
    Ok(())
}

/// Compatible roots can be requested together and infer their assignments. A shared target does
/// not need a selector when its projected dependency graph is the same in every context.
#[test]
fn workspace_resolution_axes_package_targeting() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    workspace(&context)?;
    context
        .lock()
        .args([
            "--offline",
            "--preview-features",
            "workspace-resolution-axes",
        ])
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--package", "modern-worker", "--package", "modern-peer",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    axis-lib==3.0.0
    common-leaf==2.0.0
    python-leaf==2.0.0
    sqlalchemy==2.0.0
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--package", "shared",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    common-leaf==2.0.0
    ");
    Ok(())
}

/// The selected locked context reaches installation and `uv run` without selecting an incompatible
/// version from another section of an axis.
#[test]
fn workspace_resolution_axes_sync_and_run() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    workspace(&context)?;
    context
        .lock()
        .args([
            "--offline",
            "--preview-features",
            "workspace-resolution-axes",
        ])
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.sync().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--all-matching-packages",
        "--resolution-axis", "python=py312",
        "--resolution-axis", "sqlalchemy=v1",
        "--resolution-axis", "lib=v1",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + axis-lib==1.0.0
     + common-leaf==2.0.0
     + python-leaf==1.0.0
     + sqlalchemy==1.0.0
    ");
    uv_snapshot!(context.filters(), context.run().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--all-matching-packages",
        "--resolution-axis", "python=py312",
        "--resolution-axis", "sqlalchemy=v1",
        "--resolution-axis", "lib=v1",
        "python", "-c", "import importlib.metadata as m; print(m.version('axis-lib'), m.version('common-leaf'), m.version('python-leaf'), m.version('sqlalchemy'))",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    1.0.0 2.0.0 1.0.0 1.0.0

    ----- stderr -----
    Checked 4 packages in [TIME]
    ");
    Ok(())
}

/// Once the axis configuration is removed, an unsynchronized overlay uses the installed lane's
/// versions instead of flattening incompatible alternatives from the stale universal lock.
#[test]
fn workspace_resolution_axes_removed_no_sync_overlay() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    alignment_workspace(&context, true)?;
    write_wheels(
        &context,
        &[("overlay-leaf", "1.0.0", "Requires-Dist: branch-leaf>=1\n")],
    )?;
    context
        .sync()
        .args([
            "--offline",
            "--preview-features",
            "workspace-resolution-axes",
            "--package",
            "narrow",
        ])
        .assert()
        .success();
    assert_eq!(
        locked_versions(&context, "branch-leaf")?,
        ["1.0.0", "2.0.0"]
    );
    let locked = context.read("uv.lock");

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [tool.uv]
        no-index = true
        find-links = ["wheels"]

        [tool.uv.workspace]
        members = ["members/*"]
    "#})?;

    uv_snapshot!(context.filters(), context.run().args([
        "--offline", "--no-sync", "--package", "narrow", "--with", "overlay-leaf",
        "python", "-c", "import importlib.metadata as m; print(m.version('overlay-leaf'), m.version('branch-leaf'))",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    1.0.0 1.0.0

    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 2 packages in [TIME]
     + branch-leaf==1.0.0
     + overlay-leaf==1.0.0
    ");
    assert_eq!(context.read("uv.lock"), locked);
    Ok(())
}

/// An unrelated selective upgrade cannot replace compatible pins in established contexts merely
/// because a newly available older version would reduce the number of distinct candidates.
#[test]
fn workspace_resolution_axes_selective_upgrade_preserves_pins() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    alignment_workspace(&context, true)?;
    fs_err::remove_file(
        context
            .temp_dir
            .child("wheels/common_leaf-1.0.0-py3-none-any.whl")
            .path(),
    )?;
    context
        .temp_dir
        .child("members/wide/pyproject.toml")
        .write_str(&context.read("members/wide/pyproject.toml").replace(
            r#""branch-leaf>=1""#,
            r#""branch-leaf>=1", "update-leaf>=1""#,
        ))?;
    write_wheels(&context, &[("update-leaf", "1.0.0", "")])?;
    context
        .lock()
        .args([
            "--offline",
            "--preview-features",
            "workspace-resolution-axes",
        ])
        .assert()
        .success();
    assert_eq!(
        locked_versions(&context, "common-leaf")?,
        ["2.0.0", "3.0.0"]
    );
    assert_eq!(locked_versions(&context, "update-leaf")?, ["1.0.0"]);

    write_wheels(
        &context,
        &[("common-leaf", "1.0.0", ""), ("update-leaf", "2.0.0", "")],
    )?;
    context
        .lock()
        .args([
            "--offline",
            "--preview-features",
            "workspace-resolution-axes",
            "--upgrade-package",
            "update-leaf",
        ])
        .assert()
        .success();
    assert_eq!(
        locked_versions(&context, "common-leaf")?,
        ["2.0.0", "3.0.0"]
    );
    assert_eq!(locked_versions(&context, "update-leaf")?, ["2.0.0"]);

    context
        .lock()
        .args([
            "--offline",
            "--preview-features",
            "workspace-resolution-axes",
            "--upgrade-package",
            "common-leaf",
        ])
        .assert()
        .success();
    assert_eq!(locked_versions(&context, "common-leaf")?, ["1.0.0"]);
    assert_eq!(locked_versions(&context, "update-leaf")?, ["2.0.0"]);
    Ok(())
}

/// Keeping both versions of a dependency is insufficient if optional alignment swaps the Python
/// environments in which those previously locked versions are selected.
#[test]
fn workspace_resolution_axes_selective_upgrade_preserves_marker_pins() -> Result<()> {
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

        [tool.uv.workspace.resolution-axes.runtime]
        a-wide = { members = ["wide"], constraint-dependencies = ["branch-leaf==2"] }
        z-narrow = { members = ["narrow"], constraint-dependencies = ["branch-leaf==1"] }
    "#})?;
    for (name, requirement) in [
        ("wide", "version-switch>=1,<4,!=2"),
        ("narrow", "version-switch>=1,<3"),
    ] {
        context
            .temp_dir
            .child(format!("members/{name}/pyproject.toml"))
            .write_str(&formatdoc! {r#"
            [project]
            name = "{name}"
            version = "0.1.0"
            requires-python = ">=3.12,<3.14"
            dependencies = ["branch-leaf", "{requirement}"]

            [tool.uv]
            package = false
        "#})?;
    }
    let original_requirements = indoc! {r"
        Requires-Dist: protected-leaf==1; python_version < '3.13'
        Requires-Dist: protected-leaf==2; python_version >= '3.13'
    "};
    write_wheels(
        &context,
        &[
            ("branch-leaf", "1.0.0", ""),
            ("branch-leaf", "2.0.0", ""),
            ("protected-leaf", "1.0.0", ""),
            ("protected-leaf", "2.0.0", ""),
            ("version-switch", "2.0.0", original_requirements),
            ("version-switch", "3.0.0", original_requirements),
        ],
    )?;
    context
        .lock()
        .args([
            "--offline",
            "--preview-features",
            "workspace-resolution-axes",
        ])
        .assert()
        .success();
    let original = context.read("uv.lock");

    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--resolution-axis", "runtime=a-wide", "--all-matching-packages",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    branch-leaf==2.0.0
    protected-leaf==1.0.0 ; python_full_version < '3.13'
    protected-leaf==2.0.0 ; python_full_version >= '3.13'
    version-switch==3.0.0
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--resolution-axis", "runtime=z-narrow", "--all-matching-packages",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    branch-leaf==1.0.0
    protected-leaf==1.0.0 ; python_full_version < '3.13'
    protected-leaf==2.0.0 ; python_full_version >= '3.13'
    version-switch==2.0.0
    ");

    write_wheels(
        &context,
        &[(
            "version-switch",
            "1.0.0",
            indoc! {r"
                Requires-Dist: protected-leaf==2; python_version < '3.13'
                Requires-Dist: protected-leaf==1; python_version >= '3.13'
            "},
        )],
    )?;
    context
        .lock()
        .args([
            "--offline",
            "--preview-features",
            "workspace-resolution-axes",
            "--upgrade-package",
            "version-switch",
        ])
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--resolution-axis", "runtime=a-wide", "--all-matching-packages",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    branch-leaf==2.0.0
    protected-leaf==1.0.0 ; python_full_version < '3.13'
    protected-leaf==2.0.0 ; python_full_version >= '3.13'
    version-switch==3.0.0
    ");
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--resolution-axis", "runtime=z-narrow", "--all-matching-packages",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    branch-leaf==1.0.0
    protected-leaf==1.0.0 ; python_full_version < '3.13'
    protected-leaf==2.0.0 ; python_full_version >= '3.13'
    version-switch==2.0.0
    ");
    assert_eq!(context.read("uv.lock"), original);
    context
        .lock()
        .args([
            "--offline",
            "--locked",
            "--preview-features",
            "workspace-resolution-axes",
        ])
        .assert()
        .success();
    assert_eq!(context.read("uv.lock"), original);
    Ok(())
}

/// Dynamically versioned local sources still have an exact locked identity that optional
/// alignment must retain unless that package is selected for upgrade.
#[test]
fn workspace_resolution_axes_selective_upgrade_preserves_dynamic_sources() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let workspace = context.temp_dir.child("workspace");
    let source_a = context.temp_dir.child("sources/a");
    let source_b = context.temp_dir.child("sources/b");
    let source_a_url = Url::from_file_path(source_a.path())
        .map_err(|()| anyhow::anyhow!("failed to convert source path to file URL"))?;
    let source_b_url = Url::from_file_path(source_b.path())
        .map_err(|()| anyhow::anyhow!("failed to convert source path to file URL"))?;

    workspace
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
        [tool.uv]
        no-index = true
        find-links = ["../wheels"]
        constraint-dependencies = [
            "dynamic-leaf @ {source_a_url} ; python_version < '3.12'",
            "dynamic-leaf @ {source_b_url} ; python_version < '3.12'",
        ]

        [tool.uv.workspace]
        members = ["members/*"]

        [tool.uv.workspace.resolution-axes.runtime]
        a-wide = {{ members = ["wide"], constraint-dependencies = ["branch-leaf==2"] }}
        z-narrow = {{ members = ["narrow"], constraint-dependencies = ["branch-leaf==1"] }}
    "#})?;
    for (name, requirement) in [
        ("wide", "version-switch>=1,<4,!=2"),
        ("narrow", "version-switch>=1,<3"),
    ] {
        workspace
            .child(format!("members/{name}/pyproject.toml"))
            .write_str(&formatdoc! {r#"
            [project]
            name = "{name}"
            version = "0.1.0"
            requires-python = ">=3.12"
            dependencies = ["branch-leaf", "{requirement}"]

            [tool.uv]
            package = false
        "#})?;
    }
    for source in [&source_a, &source_b] {
        source.child("pyproject.toml").write_str(indoc! {r#"
            [project]
            name = "dynamic-leaf"
            requires-python = ">=3.12"
            dependencies = []
            dynamic = ["version"]

            [build-system]
            requires = []
            build-backend = "backend"
            backend-path = ["."]
        "#})?;
        source.child("backend.py").write_str(indoc! {r#"
            from pathlib import Path


            def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
                directory = Path(metadata_directory) / "dynamic_leaf-1.0.0.dist-info"
                directory.mkdir()
                (directory / "METADATA").write_text(
                    "Metadata-Version: 2.3\nName: dynamic-leaf\nVersion: 1.0.0\n"
                    "Requires-Python: >=3.12\n",
                    encoding="utf-8",
                )
                return directory.name
        "#})?;
    }
    let source_a_requirement = format!("Requires-Dist: dynamic-leaf @ {source_a_url}\n");
    let source_b_requirement = format!("Requires-Dist: dynamic-leaf @ {source_b_url}\n");
    write_wheels(
        &context,
        &[
            ("branch-leaf", "1.0.0", ""),
            ("branch-leaf", "2.0.0", ""),
            ("version-switch", "2.0.0", &source_b_requirement),
            ("version-switch", "3.0.0", &source_a_requirement),
        ],
    )?;
    context
        .lock()
        .current_dir(&workspace)
        .args([
            "--offline",
            "--preview-features",
            "workspace-resolution-axes",
        ])
        .assert()
        .success();
    let original = context.read("workspace/uv.lock");
    let lock: toml::Value = toml::from_str(&original)?;
    let dynamic = lock
        .get("package")
        .and_then(toml::Value::as_array)
        .context("lock packages missing")?
        .iter()
        .filter(|package| package.get("name").and_then(toml::Value::as_str) == Some("dynamic-leaf"))
        .collect::<Vec<_>>();
    assert_eq!(dynamic.len(), 2);
    assert!(
        dynamic
            .iter()
            .all(|package| package.get("version").is_none())
    );
    let mut directories = dynamic
        .iter()
        .map(|package| {
            package
                .get("source")
                .and_then(|source| source.get("directory"))
                .and_then(toml::Value::as_str)
                .map(|directory| normalize_path(workspace.path().join(directory)).into_owned())
                .context("dynamic source directory missing")
        })
        .collect::<Result<Vec<_>>>()?;
    directories.sort();
    assert_eq!(
        directories,
        [
            normalize_path(source_a.path()).into_owned(),
            normalize_path(source_b.path()).into_owned(),
        ]
    );

    uv_snapshot!(context.filters(), context.export().current_dir(&workspace).args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--resolution-axis", "runtime=z-narrow", "--all-matching-packages",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    file://[TEMP_DIR]/sources/b
    branch-leaf==1.0.0
    version-switch==2.0.0
    ");

    write_wheels(
        &context,
        &[("version-switch", "1.0.0", &source_a_requirement)],
    )?;
    context
        .lock()
        .current_dir(&workspace)
        .args([
            "--offline",
            "--preview-features",
            "workspace-resolution-axes",
            "--upgrade-package",
            "version-switch",
        ])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.export().current_dir(&workspace).args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--resolution-axis", "runtime=z-narrow", "--all-matching-packages",
        "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    file://[TEMP_DIR]/sources/b
    branch-leaf==1.0.0
    version-switch==2.0.0
    ");
    assert_eq!(context.read("workspace/uv.lock"), original);
    context
        .lock()
        .current_dir(&workspace)
        .args([
            "--offline",
            "--locked",
            "--preview-features",
            "workspace-resolution-axes",
        ])
        .assert()
        .success();
    assert_eq!(context.read("workspace/uv.lock"), original);
    Ok(())
}

/// A candidate considered only for optional alignment can have an unusable build backend without
/// invalidating the already successful resolutions.
#[test]
fn workspace_resolution_axes_alignment_build_failure() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    alignment_workspace(&context, true)?;
    fs_err::remove_file(
        context
            .temp_dir
            .child("wheels/common_leaf-1.0.0-py3-none-any.whl")
            .path(),
    )?;
    write_tar_gz(
        fs_err::File::create(
            context
                .temp_dir
                .child("wheels/common_leaf-1.0.0.tar.gz")
                .path(),
        )?,
        &[
            (
                "common_leaf-1.0.0/pyproject.toml",
                indoc! {r#"
                    [build-system]
                    requires = []
                    build-backend = "backend"
                    backend-path = ["."]

                    [project]
                    name = "common-leaf"
                    version = "1.0.0"
                    requires-python = ">=3.12"
                    dynamic = ["dependencies"]
                "#},
            ),
            (
                "common_leaf-1.0.0/backend.py",
                indoc! {r#"
                    import os
                    from pathlib import Path

                    Path(os.environ["UV_TEST_ALIGNMENT_BUILD_MARKER"]).write_text("attempted")
                    raise RuntimeError("The optional alignment candidate cannot be built")
                "#},
            ),
        ],
    )?;

    context
        .lock()
        .args([
            "--offline",
            "--preview-features",
            "workspace-resolution-axes",
        ])
        .env(
            "UV_TEST_ALIGNMENT_BUILD_MARKER",
            context.temp_dir.child("alignment-build-attempt").path(),
        )
        .assert()
        .success();
    assert_eq!(context.read("alignment-build-attempt"), "attempted");
    assert_eq!(
        locked_versions(&context, "common-leaf")?,
        ["2.0.0", "3.0.0"]
    );
    context
        .lock()
        .args([
            "--offline",
            "--locked",
            "--preview-features",
            "workspace-resolution-axes",
        ])
        .assert()
        .success();
    Ok(())
}

/// Retargeting a member to another directory invalidates its old source even if the old directory
/// still exists and contains identical package metadata.
#[test]
fn workspace_resolution_axes_moved_member_source() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let pyproject = indoc! {r#"
        [tool.uv]
        no-index = true
        find-links = ["wheels"]

        [tool.uv.workspace]
        members = ["old/moved", "peer"]

        [tool.uv.workspace.resolution-axes.runtime]
        moved = { members = ["moved"] }
        peer = { members = ["peer"] }
    "#};
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(pyproject)?;
    for path in ["old/moved", "new/moved"] {
        context
            .temp_dir
            .child(format!("{path}/pyproject.toml"))
            .write_str(indoc! {r#"
            [project]
            name = "moved"
            version = "0.1.0"
            requires-python = ">=3.12"
            dependencies = ["common-leaf==1"]

            [tool.uv]
            package = true
        "#})?;
    }
    context
        .temp_dir
        .child("peer/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "peer"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [tool.uv]
        package = false
    "#})?;
    write_wheels(&context, &[("common-leaf", "1.0.0", "")])?;
    context
        .lock()
        .args([
            "--offline",
            "--preview-features",
            "workspace-resolution-axes",
        ])
        .assert()
        .success();
    let original = context.read("uv.lock");

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&pyproject.replace("old/moved", "new/moved"))?;
    context
        .lock()
        .args([
            "--offline",
            "--locked",
            "--preview-features",
            "workspace-resolution-axes",
        ])
        .assert()
        .failure();
    assert_eq!(context.read("uv.lock"), original);
    context
        .lock()
        .args([
            "--offline",
            "--preview-features",
            "workspace-resolution-axes",
        ])
        .assert()
        .success();

    let lock: toml::Value = toml::from_str(&context.read("uv.lock"))?;
    assert_eq!(
        lock.get("workspace-axes")
            .and_then(|axes| axes.get("member-paths"))
            .and_then(|paths| paths.get("moved"))
            .and_then(toml::Value::as_str),
        Some("new/moved")
    );
    uv_snapshot!(context.filters(), context.export().args([
        "--offline", "--frozen", "--preview-features", "workspace-resolution-axes",
        "--package", "moved", "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    -e ./new/moved
    common-leaf==1.0.0
    ");
    Ok(())
}

/// Named and path-based builds infer the source member's Python lane before interpreter discovery.
#[test]
fn workspace_resolution_axes_build_python() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"]);
    python_build_workspace(&context)?;

    uv_snapshot!(context.filters(), context.build().args([
        "--offline", "--preview-features", "workspace-resolution-axes",
        "--package", "modern", "--wheel",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Building wheel...
    Successfully built dist/modern-0.1.0-py3-none-any.whl
    ");
    assert_eq!(context.read("members/modern/build-python"), "3.13");
    fs_err::remove_file(context.temp_dir.child("members/modern/build-python").path())?;

    uv_snapshot!(context.filters(), context.build().args([
        "--offline", "--preview-features", "workspace-resolution-axes",
        "members/modern/../modern", "--wheel",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Building wheel...
    Successfully built dist/modern-0.1.0-py3-none-any.whl
    ");
    assert_eq!(context.read("members/modern/build-python"), "3.13");
    Ok(())
}

/// Building all packages does not combine their mutually exclusive Python requirements.
#[test]
fn workspace_resolution_axes_build_all_packages_python() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"])
        .with_filter((r"\[(legacy|modern)\]", "[PKG]"));
    python_build_workspace(&context)?;

    uv_snapshot!(context.filters(), context.build().args([
        "--offline", "--preview-features", "workspace-resolution-axes",
        "--all-packages", "--wheel", "--no-build-logs",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    [PKG] Building wheel...
    [PKG] Building wheel...
    Successfully built dist/legacy-0.1.0-py3-none-any.whl
    Successfully built dist/modern-0.1.0-py3-none-any.whl
    ");
    assert_eq!(context.read("members/legacy/build-python"), "3.12");
    assert_eq!(context.read("members/modern/build-python"), "3.13");
    Ok(())
}

/// Project pins are checked against that member's assignments, while a virtual-root pin can use
/// any Python version supported by the workspace's physical union.
#[test]
fn workspace_resolution_axes_python_pin() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"]);
    python_build_workspace(&context)?;
    let modern = context.temp_dir.child("members/modern");

    uv_snapshot!(context.filters(), context.python_pin().current_dir(&modern).args([
        "--offline", "--preview-features", "workspace-resolution-axes", "3.12",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The requested Python version `3.12` is incompatible with the project `requires-python` value of `==3.13.*`.
    ");
    assert!(!modern.child(".python-version").exists());

    uv_snapshot!(context.filters(), context.python_pin().current_dir(&modern).args([
        "--offline", "--preview-features", "workspace-resolution-axes", "3.13",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    Pinned `.python-version` to `3.13`
    ");

    modern.child(".python-version").write_str("3.12\n")?;
    uv_snapshot!(context.filters(), context.python_pin().current_dir(&modern).args([
        "--offline", "--preview-features", "workspace-resolution-axes",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.12

    ----- stderr -----
    warning: The pinned Python version `3.12` is incompatible with the project `requires-python` value of `==3.13.*`.
    ");

    uv_snapshot!(context.filters(), context.python_pin().args([
        "--offline", "--preview-features", "workspace-resolution-axes", "3.12",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    Pinned `.python-version` to `3.12`
    ");
    uv_snapshot!(context.filters(), context.python_pin().args([
        "--offline", "--preview-features", "workspace-resolution-axes", "3.13",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    Updated `.python-version` from `3.12` -> `3.13`
    ");
    Ok(())
}
