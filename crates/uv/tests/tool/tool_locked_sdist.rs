use std::path::Path;

use anyhow::Result;
use indoc::{formatdoc, indoc};
use sha2::{Digest, Sha256};
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{method, path},
};

use uv_fs::PythonExt;
use uv_test::archive::write_tar_gz;
use uv_test::package_server::PackageServer;
use uv_test::packse::WheelBuilder;
use uv_test::uv_snapshot;

/// Install a registry tool available only as a source distribution. Its backend removes the lock
/// from the extracted source tree, so later installs need the lock from the original archive.
#[tokio::test]
async fn locked_tool_source_distribution() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_tool_dirs();
    let dependency_server = PackageServer::new(&"demo-dependency".parse()?).await;
    let (dependency_filename, dependency) =
        WheelBuilder::new(&"demo-dependency".parse()?, &"1.0".parse()?).build();
    let dependency_hash = hex::encode(Sha256::digest(&dependency));
    dependency_server
        .serve(&dependency_filename, &dependency, Some(&dependency_hash))
        .await;
    let dependency_index = dependency_server.index_url();
    let dependency_url = dependency_server.file_url(&dependency_filename);
    let lock = formatdoc! {r#"
        lock-version = "1.0"
        created-by = "uv"

        [[packages]]
        name = "demo-dependency"
        version = "1.0"
        index = "{dependency_index}"
        wheels = [{{url = "{dependency_url}", hashes = {{sha256 = "{dependency_hash}"}}}}]
    "#};

    let tool_server = PackageServer::new(&"locked-source".parse()?).await;
    let archive = locked_source_archive(&lock, &context.temp_dir.join("backend-ran"))?;
    let source_hash = hex::encode(Sha256::digest(&archive));
    tool_server
        .serve("locked_source-1.0.tar.gz", &archive, Some(&source_hash))
        .await;
    let tool_index = tool_server.index_url();

    uv_snapshot!(context.filters(), context.tool_run().args([
        "--preview-features", "locked-tools", "--locked",
        "--index", &tool_index, "--default-index", &dependency_index, "locked-source",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    1.0

    ----- stderr -----
    Installed [N] packages in [TIME]
     + demo-dependency==1.0
     + locked-source==1.0
    ");

    uv_snapshot!(context.filters(), context.tool_run().args([
        "--preview-features", "locked-tools", "--locked", "--offline",
        "--index", &tool_index, "--default-index", &dependency_index, "locked-source",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    1.0
    ");

    Ok(())
}

/// Reject a source distribution's forged artifact URL before fetching it or importing its backend.
#[tokio::test]
async fn locked_tool_source_distribution_rejects_forged_url_before_build() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    let dependency_server = PackageServer::new(&"demo-dependency".parse()?).await;
    let (dependency_filename, dependency) =
        WheelBuilder::new(&"demo-dependency".parse()?, &"1.0".parse()?).build();
    let dependency_hash = hex::encode(Sha256::digest(&dependency));
    dependency_server
        .serve(&dependency_filename, &dependency, Some(&dependency_hash))
        .await;
    let dependency_index = dependency_server.index_url();
    let forged_filename = format!("forged/{dependency_filename}");
    let forged_url = dependency_server.file_url(&forged_filename);
    Mock::given(method("GET"))
        .and(path(format!("/{forged_filename}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(dependency))
        .expect(0)
        .mount(dependency_server.mock_server())
        .await;
    let lock = formatdoc! {r#"
        lock-version = "1.0"
        created-by = "uv"

        [[packages]]
        name = "demo-dependency"
        version = "1.0"
        index = "{dependency_index}"
        wheels = [{{url = "{forged_url}", hashes = {{sha256 = "{dependency_hash}"}}}}]
    "#};

    let tool_server = PackageServer::new(&"locked-source".parse()?).await;
    let marker = context.temp_dir.join("backend-ran");
    let archive = locked_source_archive(&lock, &marker)?;
    let source_hash = hex::encode(Sha256::digest(&archive));
    tool_server
        .serve("locked_source-1.0.tar.gz", &archive, Some(&source_hash))
        .await;
    let tool_index = tool_server.index_url();

    uv_snapshot!(context.filters(), context.tool_run().args([
        "--preview-features", "locked-tools", "--locked",
        "--index", &tool_index, "--default-index", &dependency_index, "locked-source",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The packaged lock for `locked-source==1.0` contains unverified artifacts
      cause: URL for `demo-dependency==1.0` is not listed by http://[LOCALHOST]/simple: http://[LOCALHOST]/forged/demo_dependency-1.0-py3-none-any.whl
    ");
    assert!(!marker.exists());

    Ok(())
}

fn locked_source_archive(lock: &str, marker: &Path) -> Result<Vec<u8>> {
    let (wheel_filename, wheel) = WheelBuilder::new(&"locked-source".parse()?, &"1.0".parse()?)
        .requires(&["demo-dependency>=1".parse()?])
        .files(&[
            (
                "locked_source-1.0.dist-info/entry_points.txt",
                "[console_scripts]\nlocked-source = locked_source.cli:main\n",
            ),
            (
                "locked_source/cli.py",
                "from importlib.metadata import version\ndef main(): print(version('demo-dependency'))\n",
            ),
        ])
        .build();
    let pyproject = indoc! {r#"
        [build-system]
        requires = []
        build-backend = "backend"
        backend-path = ["."]
    "#};
    let pkg_info = indoc! {"
        Metadata-Version: 2.2
        Name: locked-source
        Version: 1.0
        Dynamic: Requires-Dist
    "};
    let marker = marker.escape_for_python();
    let backend = formatdoc! {r#"
        import shutil
        from pathlib import Path

        Path({marker}).touch()
        WHEEL = Path(__file__).with_name("{wheel_filename}")

        def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
            WHEEL.with_name("pylock.toml").unlink(missing_ok=True)
            shutil.copyfile(WHEEL, Path(wheel_directory) / WHEEL.name)
            return WHEEL.name
    "#};
    let mut archive = Vec::new();
    write_tar_gz(
        &mut archive,
        &[
            ("locked_source-1.0/pyproject.toml", pyproject.as_bytes()),
            ("locked_source-1.0/PKG-INFO", pkg_info.as_bytes()),
            ("locked_source-1.0/pylock.toml", lock.as_bytes()),
            ("locked_source-1.0/backend.py", backend.as_bytes()),
            (
                &format!("locked_source-1.0/{wheel_filename}"),
                wheel.as_slice(),
            ),
        ],
    )?;
    Ok(archive)
}
