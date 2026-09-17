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
        py312 = { members = ["legacy"], requires-python = "==3.12.*" }
        py313 = { members = ["modern"], requires-python = "==3.13.*" }
    "#})?;
    for (name, requires_python) in [("legacy", "==3.12.*"), ("modern", "==3.13.*")] {
        context
            .temp_dir
            .child(format!("members/{name}/pyproject.toml"))
            .write_str(&formatdoc! {r#"
            [project]
            name = "{name}"
            version = "0.1.0"
            requires-python = "{requires_python}"
            dependencies = []

            [tool.uv]
            package = false
        "#})?;
    }
    let wheels = context.temp_dir.child("wheels");
    wheels.create_dir_all()?;
    write_wheel_with_metadata(
        wheels.child("edit_leaf-1.0.0-py3-none-any.whl").path(),
        "edit-leaf",
        "1.0.0",
        "edit_leaf-1.0.0",
        "",
        &[],
    )?;
    Ok(())
}

/// Editing a member selects its own Python lane for interpreter discovery while retaining the
/// complete universal lock. The synchronization phase must receive an ordinary selected view.
#[test]
fn workspace_resolution_axes_add_remove_member() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"]);
    workspace(&context)?;
    context
        .add()
        .args(AXES)
        .args([
            "--package",
            "modern",
            "--python",
            "3.13",
            "edit-leaf==1.0.0",
        ])
        .assert()
        .success();
    assert!(
        context
            .read("members/modern/pyproject.toml")
            .contains("edit-leaf==1.0.0")
    );
    assert!(context.read("uv.lock").starts_with("version = 3\n"));
    uv_snapshot!(context.filters(), context.export().args(AXES).args([
        "--frozen", "--package", "modern", "--no-header", "--no-hashes", "--no-annotate",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    edit-leaf==1.0.0
    ");
    uv_snapshot!(context.filters(), context.export().args(AXES).args([
        "--frozen", "--package", "legacy", "--no-header", "--no-hashes", "--no-annotate",
    ]), @"exit_code: 0 (success)");
    context
        .remove()
        .args(AXES)
        .args(["--package", "modern", "edit-leaf"])
        .assert()
        .success();
    assert!(
        !context
            .read("members/modern/pyproject.toml")
            .contains("edit-leaf")
    );
    context.lock().args(AXES).arg("--locked").assert().success();
    assert!(context.read("uv.lock").starts_with("version = 3\n"));
    Ok(())
}

/// Anonymous root requirements retain their resolved source identity, even when another Python
/// lane contains a workspace member with the same name and version. Overrides change the actual
/// root distribution without changing the original dependency-group declaration.
#[test]
fn workspace_resolution_axes_anonymous_root_source_identity() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"]);
    workspace(&context)?;
    let base = context.read("pyproject.toml").replace(
        "no-index = true\n",
        "no-index = true\nno-sources-package = [\"legacy\"]\ndefault-groups = []\n",
    );
    let root = formatdoc! {r#"
        {base}

        [dependency-groups]
        tooling = [
            "legacy==0.1.0; python_version >= '3.13'",
            "override-root==1.0.0; python_version >= '3.13'",
        ]
    "#};
    context.temp_dir.child("pyproject.toml").write_str(&root)?;
    for (name, version, metadata) in [
        ("legacy", "0.1.0", "Requires-Dist: root-leaf==1.0.0\n"),
        ("root-leaf", "1.0.0", ""),
        (
            "override-root",
            "1.0.0",
            "Requires-Dist: override-leaf==1.0.0\n",
        ),
        (
            "override-root",
            "2.0.0",
            "Requires-Dist: override-leaf==2.0.0\n",
        ),
        ("override-leaf", "1.0.0", ""),
        ("override-leaf", "2.0.0", ""),
    ] {
        let stem = format!("{}-{version}", name.replace('-', "_"));
        write_wheel_with_metadata(
            context
                .temp_dir
                .child(format!("wheels/{stem}-py3-none-any.whl"))
                .path(),
            name,
            version,
            &stem,
            metadata,
            &[],
        )?;
    }

    for (override_dependency, expected) in [
        (
            "",
            "legacy==0.1.0\noverride-leaf==1.0.0\noverride-root==1.0.0\nroot-leaf==1.0.0\n",
        ),
        (
            "override-dependencies = [\"override-root==2.0.0; python_version >= '3.13'\"]\n",
            "legacy==0.1.0\noverride-leaf==2.0.0\noverride-root==2.0.0\nroot-leaf==1.0.0\n",
        ),
    ] {
        context
            .temp_dir
            .child("pyproject.toml")
            .write_str(&root.replace(
                "no-index = true\n",
                &format!("no-index = true\n{override_dependency}"),
            ))?;
        context.lock().args(AXES).assert().success();
        context.lock().args(AXES).arg("--locked").assert().success();
        let output = context
            .export()
            .args(AXES)
            .args([
                "--frozen",
                "--package",
                "modern",
                "--only-group",
                "tooling",
                "--no-install-workspace",
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
                "--package",
                "legacy",
                "--only-group",
                "tooling",
                "--no-header",
                "--no-hashes",
                "--no-annotate",
            ])
            .output()?;
        output.clone().assert().success();
        assert!(output.stdout.is_empty());
    }
    Ok(())
}
