use anyhow::Result;
use assert_fs::prelude::*;
use indoc::indoc;

use uv_redacted::SafeUrl;
use uv_test::archive::generate_failing_source_archive;
use uv_test::package_server::PackageServer;
use uv_test::uv_snapshot;

/// Reject invalid workspace sources before invoking the build backend.
#[test]
fn compile_pyproject_toml_eager_validation() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
            [build-system]
            requires = []
            build-backend = "backend"
            backend-path = ["."]

            [project]
            name = "project"
            dynamic = ["version"]
            requires-python = ">=3.10"
            dependencies = ["anyio==4.7.0"]

            [tool.uv.sources]
            anyio = { workspace = true }
        "#})?;
    context.temp_dir.child("backend.py").write_str(indoc! {r#"
        from pathlib import Path

        Path(__file__).with_name("backend-invoked").touch()
        raise RuntimeError("build backend should not run")
    "#})?;

    uv_snapshot!(context.pip_compile().arg("pyproject.toml"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to parse entry: `anyio`
      cause: `anyio` references a workspace in `tool.uv.sources` (e.g., `anyio = { workspace = true }`), but is not a workspace member
    ");
    context
        .temp_dir
        .child("backend-invoked")
        .assert(predicates::path::missing());

    Ok(())
}

/// Identify the direct parent of a transitive dependency that fails to build.
#[tokio::test]
async fn compile_derivation_chain() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let name = "wsgiref".parse()?;
    let archive = generate_failing_source_archive(&name, &"0.1.2".parse()?)?;
    let server = PackageServer::new(&name).await;
    server.serve("wsgiref-0.1.2.tar.gz", &archive, None).await;

    let child = context.temp_dir.child("child");
    child.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "child"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["wsgiref"]
    "#})?;
    let child_url = SafeUrl::from_directory_path(child.path()).expect("absolute child directory");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&indoc::formatdoc! {r#"
            [build-system]
            requires = ["setuptools>=42"]

            [project]
            name = "project"
            version = "0.1.0"
            dependencies = ["child @ {child_url}"]
        "#})?;

    uv_snapshot!(context.filters(), context.pip_compile()
        .arg("pyproject.toml")
        .arg("--index-url").arg(server.index_url()), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to build `wsgiref==0.1.2`
      cause: The build backend returned an error
      cause: Call to `backend.prepare_metadata_for_build_wheel` failed (exit status: 1)

             [stderr]
             intentional build failure

    hint: `wsgiref` (v0.1.2) was included because `child` (v0.1.0) depends on `wsgiref`

    hint: Build failures usually indicate a problem with the package or the build environment
    ");

    Ok(())
}
