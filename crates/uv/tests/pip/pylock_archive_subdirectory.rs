use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use assert_cmd::prelude::*;
use assert_fs::prelude::*;
use indoc::{formatdoc, indoc};
use sha2::{Digest, Sha256};

use uv_fs::PythonExt;
use uv_test::archive::write_tar_gz;
use uv_test::package_server::PackageServer;
use uv_test::packse::generate_wheel;
use uv_test::uv_snapshot;

fn backend(filename: &str, marker: &Path) -> String {
    formatdoc! {r#"
        import shutil
        from pathlib import Path

        Path({marker}).touch()

        def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
            shutil.copyfile(Path(__file__).with_name("{filename}"), Path(wheel_directory) / "{filename}")
            return "{filename}"
    "#, marker = marker.escape_for_python()}
}

#[tokio::test]
async fn local_archive_subdirectory_is_not_built_as_root() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let root_marker = context.temp_dir.child("root-backend-ran");
    let nested_marker = context.temp_dir.child("nested-backend-ran");
    let root_name = "root-demo".parse()?;
    let nested_name = "nested-demo".parse()?;
    let (root_filename, root_wheel) = generate_wheel(
        &root_name,
        &"1.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    let (nested_filename, nested_wheel) = generate_wheel(
        &nested_name,
        &"2.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    let project = indoc! {r#"
        [build-system]
        requires = []
        build-backend = "backend"
        backend-path = ["."]
    "#};
    let root_backend = backend(&root_filename, root_marker.path());
    let nested_backend = backend(&nested_filename, nested_marker.path());
    let mut archive = Vec::new();
    write_tar_gz(
        &mut archive,
        &[
            ("projects/pyproject.toml", project.as_bytes()),
            ("projects/backend.py", root_backend.as_bytes()),
            (&format!("projects/{root_filename}"), root_wheel.as_slice()),
            ("projects/nested/pyproject.toml", project.as_bytes()),
            ("projects/nested/backend.py", nested_backend.as_bytes()),
            (
                &format!("projects/nested/{nested_filename}"),
                nested_wheel.as_slice(),
            ),
        ],
    )?;
    context
        .temp_dir
        .child("projects.tar.gz")
        .write_binary(&archive)?;
    let hash = hex::encode(Sha256::digest(&archive));
    let lock = context.temp_dir.child("pylock.toml");

    // Populate the local archive's root cache before selecting a different project within it.
    for subdirectory in ["", ", subdirectory = '.'"] {
        lock.write_str(&formatdoc! {r#"
            lock-version = "1.0"
            created-by = "uv"
            requires-python = ">=3.12"

            [[packages]]
            name = "root-demo"
            version = "1.0"
            archive = {{ path = "projects.tar.gz", hashes = {{ sha256 = "{hash}" }}{subdirectory} }}
        "#})?;
        context
            .pip_install()
            .arg("-r")
            .arg(lock.path())
            .arg("--preview-features")
            .arg("pylock")
            .arg("--no-index")
            .assert()
            .success();
    }
    assert!(root_marker.exists());
    fs_err::remove_file(root_marker.path())?;

    lock.write_str(&formatdoc! {r#"
        lock-version = "1.0"
        created-by = "uv"
        requires-python = ">=3.12"

        [[packages]]
        name = "nested-demo"
        version = "2.0"
        archive = {{ path = "projects.tar.gz", subdirectory = "nested", hashes = {{ sha256 = "{hash}" }} }}
    "#})?;
    insta::allow_duplicates! {
        for no_cache in [true, false] {
            let mut command = context.pip_install();
            command.arg("-r").arg(lock.path())
                .arg("--preview-features").arg("pylock").arg("--no-index");
            if no_cache {
                command.arg("--no-cache");
            }
            let output = uv_snapshot!(context.filters(), command, @"
            exit_code: 2 (failure)
            ----- stderr -----
            error: Package `nested-demo` selects subdirectory `nested` in local archive `projects.tar.gz`, which is not supported
            ");
            assert!(!output.status.success());
            assert!(!root_marker.exists());
            assert!(!nested_marker.exists());
        }
    }

    // The same selected project is supported when the archive has a URL source.
    let server = PackageServer::new(&nested_name).await;
    server.serve("projects.tar.gz", &archive, Some(&hash)).await;
    let url = server.file_url("projects.tar.gz");
    lock.write_str(&formatdoc! {r#"
        lock-version = "1.0"
        created-by = "uv"
        requires-python = ">=3.12"

        [[packages]]
        name = "nested-demo"
        version = "2.0"
        archive = {{ url = "{url}", subdirectory = "nested", hashes = {{ sha256 = "{hash}" }} }}
    "#})?;
    context
        .pip_install()
        .arg("-r")
        .arg(lock.path())
        .arg("--preview-features")
        .arg("pylock")
        .arg("--no-index")
        .assert()
        .success();
    context.assert_installed("nested_demo", "2.0");
    assert!(!root_marker.exists());
    assert!(nested_marker.exists());
    Ok(())
}
