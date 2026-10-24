use anyhow::Result;
use assert_fs::prelude::*;
use indoc::indoc;

use uv_test::archive::generate_failing_source_archive;
use uv_test::package_server::PackageServer;
use uv_test::uv_snapshot;

/// Identify the editable project that introduced a failing registry dependency.
#[tokio::test]
async fn resolve_derivation_chain() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let name = "wsgiref".parse()?;
    let archive = generate_failing_source_archive(&name, &"0.1.2".parse()?)?;
    let server = PackageServer::new(&name).await;
    server.serve("wsgiref-0.1.2.tar.gz", &archive, None).await;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["wsgiref"]
    "#})?;

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("-e").arg(".")
        .arg("--index-url").arg(server.index_url()), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to build `wsgiref==0.1.2`
      cause: The build backend returned an error
      cause: Call to `backend.prepare_metadata_for_build_wheel` failed (exit status: 1)

             [stderr]
             intentional build failure

    hint: `wsgiref` (v0.1.2) was included because `project` (v0.1.0) depends on `wsgiref`

    hint: Build failures usually indicate a problem with the package or the build environment
    ");

    Ok(())
}
