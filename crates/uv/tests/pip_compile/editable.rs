use anyhow::Result;
use assert_cmd::prelude::*;
use assert_fs::prelude::*;
use indoc::indoc;

/// Prepare dynamic editable metadata once for equivalent requirement paths.
#[test]
fn deduplicate_editable_metadata() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let project = context.temp_dir.child("project");
    project.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        dynamic = ["dependencies"]

        [build-system]
        requires = []
        backend-path = ["."]
        build-backend = "backend"
    "#})?;
    project.child("backend.py").write_str(indoc! {r#"
        from pathlib import Path

        def prepare_metadata_for_build_editable(metadata_directory, config_settings=None):
            counter = Path(__file__).parent.parent / "metadata-invocations.txt"
            with counter.open("ab") as file:
                file.write(b"called\n")
            dist_info = Path(metadata_directory) / "project-0.1.0.dist-info"
            dist_info.mkdir()
            (dist_info / "METADATA").write_text(
                "Metadata-Version: 2.1\nName: project\nVersion: 0.1.0\n"
            )
            return dist_info.name
    "#})?;
    context
        .temp_dir
        .child("requirements.in")
        .write_str("-e ./project\n-e ./project/.\n")?;

    context
        .pip_compile()
        .arg("requirements.in")
        .arg("--offline")
        .arg("--no-header")
        .arg("--no-annotate")
        .assert()
        .success()
        .stdout("-e ./project\n");
    context
        .temp_dir
        .child("metadata-invocations.txt")
        .assert("called\n");
    Ok(())
}
