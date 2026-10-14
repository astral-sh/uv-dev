use std::fmt::Write;
use std::path::PathBuf;

use anyhow::{Context, Result};
use assert_fs::prelude::*;
use async_zip::base::write::ZipFileWriter;
use async_zip::{Compression, ZipEntryBuilder};
use futures::executor::block_on;
use insta::assert_snapshot;

use uv_cache::CacheBucket;
use uv_fs::PortablePath;
use uv_test::package_server::PackageServer;
use uv_test::{TestContext, assert_path_missing, uv_snapshot};

#[test]
fn binary_payloads_stay_in_archive_without_preview_local() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheel = binary_payload_wheel(&context)?;

    uv_snapshot!(context.filters(), context.pip_install().arg(&wheel), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + binary-payload==0.1.0 (from file://[TEMP_DIR]/binary_payload-0.1.0-py3-none-any.whl)
    ");

    assert_path_missing(context.cache_dir.child("files-v0"));
    let archive_files = context.cache_files(CacheBucket::Archive)?;
    let archive_binary = archive_files
        .iter()
        .find(|path| path.ends_with("binary_payload/native.so"))
        .context("binary payload is missing from the archive")?;
    assert_eq!(context.read_bytes(archive_binary), BINARY_PAYLOAD_CONTENTS);
    assert_eq!(
        context.read_bytes(context.site_packages().join("binary_payload/native.so")),
        BINARY_PAYLOAD_CONTENTS,
    );
    Ok(())
}

#[tokio::test]
async fn binary_payloads_stay_in_archive_without_preview_streaming() -> Result<()> {
    let server = PackageServer::new(&"binary-payload".parse()?).await;
    let filename = "binary_payload-0.1.0-py3-none-any.whl";
    let context = uv_test::test_context!("3.12");
    let wheel = binary_payload_wheel(&context)?;
    server
        .serve(filename, &context.read_bytes(&wheel), None)
        .await;

    uv_snapshot!(context.filters(), context.pip_install().arg(server.file_url(filename)), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + binary-payload==0.1.0 (from http://[LOCALHOST]/binary_payload-0.1.0-py3-none-any.whl)
    ");

    assert_path_missing(context.cache_dir.child("files-v0"));
    let archive_files = context.cache_files(CacheBucket::Archive)?;
    let archive_binary = archive_files
        .iter()
        .find(|path| path.ends_with("binary_payload/native.so"))
        .context("binary payload is missing from the archive")?;
    assert_eq!(context.read_bytes(archive_binary), BINARY_PAYLOAD_CONTENTS);
    assert_eq!(
        context.read_bytes(context.site_packages().join("binary_payload/native.so")),
        BINARY_PAYLOAD_CONTENTS,
    );
    Ok(())
}

#[test]
fn all_files_except_record_use_archive_file_store_local_serial() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_concurrent_installs("1");
    let wheel = binary_payload_wheel(&context)?;
    uv_snapshot!(context.filters(), context.pip_install().args(["--preview-features", "content-addressed-cache"]).arg(&wheel), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + binary-payload==0.1.0 (from file://[TEMP_DIR]/binary_payload-0.1.0-py3-none-any.whl)
    ");

    let objects = context.cache_files(CacheBucket::Files)?;
    // Identical contents still need separate executable and non-executable objects.
    let mut snapshot = String::new();
    for object in &objects {
        writeln!(
            snapshot,
            "{}",
            PortablePath::from(object.strip_prefix(context.cache_dir.path())?)
        )?;
    }
    assert_snapshot!(snapshot, @"
    files-v0/0c/0c8d68fa16e023b913e926be9b281c5a133c33291b3e60a54c310376f5602a45
    files-v0/2f/2f4d468b80be8a639ba0bdcfc738be8d912c35dd686a1eb15272e8f56096358c
    files-v0/4a/4a3f63865c29c673794f181932bc0e2c4779275f22e03a896d3d6ca3ac447332
    files-v0/80/8043c55c494befd8bb44cf59c112ae6a944da98d04b550ff4fd055e2ebfabeb8
    files-v0/92/920a0fbc7cd79a94ab2adabfaa8b93804bf6e3e858c454d45958cc9902554248
    files-v0/bf/bf13d7b1c373edcb8588b94aae048664a6684f665fa4a7e8cd814360e537a049
    files-v0/fa/fad1ac6fba02614a6ee120fbb397cd676f48c101afee95ab29d146c76df03596
    ");
    assert_archive_files_share_objects_except_record(&context, &objects)?;
    Ok(())
}

#[test]
fn all_files_except_record_use_archive_file_store_local_concurrent() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_concurrent_installs("4");
    let wheel = binary_payload_wheel(&context)?;
    uv_snapshot!(context.filters(), context.pip_install().args(["--preview-features", "content-addressed-cache"]).arg(&wheel), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + binary-payload==0.1.0 (from file://[TEMP_DIR]/binary_payload-0.1.0-py3-none-any.whl)
    ");

    let objects = context.cache_files(CacheBucket::Files)?;
    // Identical contents still need separate executable and non-executable objects.
    let mut snapshot = String::new();
    for object in &objects {
        writeln!(
            snapshot,
            "{}",
            PortablePath::from(object.strip_prefix(context.cache_dir.path())?)
        )?;
    }
    assert_snapshot!(snapshot, @"
    files-v0/0c/0c8d68fa16e023b913e926be9b281c5a133c33291b3e60a54c310376f5602a45
    files-v0/2f/2f4d468b80be8a639ba0bdcfc738be8d912c35dd686a1eb15272e8f56096358c
    files-v0/4a/4a3f63865c29c673794f181932bc0e2c4779275f22e03a896d3d6ca3ac447332
    files-v0/80/8043c55c494befd8bb44cf59c112ae6a944da98d04b550ff4fd055e2ebfabeb8
    files-v0/92/920a0fbc7cd79a94ab2adabfaa8b93804bf6e3e858c454d45958cc9902554248
    files-v0/bf/bf13d7b1c373edcb8588b94aae048664a6684f665fa4a7e8cd814360e537a049
    files-v0/fa/fad1ac6fba02614a6ee120fbb397cd676f48c101afee95ab29d146c76df03596
    ");
    assert_archive_files_share_objects_except_record(&context, &objects)?;
    Ok(())
}

#[tokio::test]
async fn all_files_except_record_use_archive_file_store_streaming_serial() -> Result<()> {
    let server = PackageServer::new(&"binary-payload".parse()?).await;
    let filename = "binary_payload-0.1.0-py3-none-any.whl";
    let context = uv_test::test_context!("3.12").with_concurrent_installs("1");
    let wheel = binary_payload_wheel(&context)?;
    server
        .serve(filename, &context.read_bytes(&wheel), None)
        .await;
    uv_snapshot!(context.filters(), context.pip_install().args(["--preview-features", "content-addressed-cache"]).arg(server.file_url(filename)), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + binary-payload==0.1.0 (from http://[LOCALHOST]/binary_payload-0.1.0-py3-none-any.whl)
    ");

    let objects = context.cache_files(CacheBucket::Files)?;
    // Identical contents still need separate executable and non-executable objects.
    let mut snapshot = String::new();
    for object in &objects {
        writeln!(
            snapshot,
            "{}",
            PortablePath::from(object.strip_prefix(context.cache_dir.path())?)
        )?;
    }
    assert_snapshot!(snapshot, @"
    files-v0/0c/0c8d68fa16e023b913e926be9b281c5a133c33291b3e60a54c310376f5602a45
    files-v0/2f/2f4d468b80be8a639ba0bdcfc738be8d912c35dd686a1eb15272e8f56096358c
    files-v0/4a/4a3f63865c29c673794f181932bc0e2c4779275f22e03a896d3d6ca3ac447332
    files-v0/80/8043c55c494befd8bb44cf59c112ae6a944da98d04b550ff4fd055e2ebfabeb8
    files-v0/92/920a0fbc7cd79a94ab2adabfaa8b93804bf6e3e858c454d45958cc9902554248
    files-v0/bf/bf13d7b1c373edcb8588b94aae048664a6684f665fa4a7e8cd814360e537a049
    files-v0/fa/fad1ac6fba02614a6ee120fbb397cd676f48c101afee95ab29d146c76df03596
    ");
    assert_archive_files_share_objects_except_record(&context, &objects)?;
    Ok(())
}

#[tokio::test]
async fn all_files_except_record_use_archive_file_store_streaming_concurrent() -> Result<()> {
    let server = PackageServer::new(&"binary-payload".parse()?).await;
    let filename = "binary_payload-0.1.0-py3-none-any.whl";
    let context = uv_test::test_context!("3.12").with_concurrent_installs("4");
    let wheel = binary_payload_wheel(&context)?;
    server
        .serve(filename, &context.read_bytes(&wheel), None)
        .await;
    uv_snapshot!(context.filters(), context.pip_install().args(["--preview-features", "content-addressed-cache"]).arg(server.file_url(filename)), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + binary-payload==0.1.0 (from http://[LOCALHOST]/binary_payload-0.1.0-py3-none-any.whl)
    ");

    let objects = context.cache_files(CacheBucket::Files)?;
    // Identical contents still need separate executable and non-executable objects.
    let mut snapshot = String::new();
    for object in &objects {
        writeln!(
            snapshot,
            "{}",
            PortablePath::from(object.strip_prefix(context.cache_dir.path())?)
        )?;
    }
    assert_snapshot!(snapshot, @"
    files-v0/0c/0c8d68fa16e023b913e926be9b281c5a133c33291b3e60a54c310376f5602a45
    files-v0/2f/2f4d468b80be8a639ba0bdcfc738be8d912c35dd686a1eb15272e8f56096358c
    files-v0/4a/4a3f63865c29c673794f181932bc0e2c4779275f22e03a896d3d6ca3ac447332
    files-v0/80/8043c55c494befd8bb44cf59c112ae6a944da98d04b550ff4fd055e2ebfabeb8
    files-v0/92/920a0fbc7cd79a94ab2adabfaa8b93804bf6e3e858c454d45958cc9902554248
    files-v0/bf/bf13d7b1c373edcb8588b94aae048664a6684f665fa4a7e8cd814360e537a049
    files-v0/fa/fad1ac6fba02614a6ee120fbb397cd676f48c101afee95ab29d146c76df03596
    ");
    assert_archive_files_share_objects_except_record(&context, &objects)?;
    Ok(())
}

#[test]
fn binary_payloads_use_archive_file_store() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_file_counts()
        .with_filtered_sizes_and_units()
        .with_filtered_python_names()
        .with_filtered_virtualenv_bin();
    let wheel = binary_payload_wheel(&context)?;

    uv_snapshot!(context.filters(), context.pip_install()
        .args(["--preview-features", "content-addressed-cache", "--link-mode", "copy"])
        .arg(&wheel), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + binary-payload==0.1.0 (from file://[TEMP_DIR]/binary_payload-0.1.0-py3-none-any.whl)
    ");

    let objects = context.cache_files(CacheBucket::Files)?;
    let archive_binary = context
        .cache_files(CacheBucket::Archive)?
        .into_iter()
        .find(|path| path.ends_with("binary_payload/native.so"))
        .context("missing cached binary payload")?;
    let archive = archive_binary
        .parent()
        .and_then(|path| path.parent())
        .context("binary payload should be inside a cached archive")?;
    let object = objects
        .iter()
        .find(|object| uv_fs::is_same_file_allow_missing(&archive_binary, object) == Some(true))
        .context("missing shared binary payload")?;

    uv_snapshot!(context.filters(), context.prune(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Pruning cache at: [CACHE_DIR]/
    No unused entries found
    ");
    assert_eq!(context.read_bytes(object), BINARY_PAYLOAD_CONTENTS);

    // The archive remains complete even if its file-store entry is removed.
    fs_err::remove_file(object)?;
    let target = context.temp_dir.child("retained-target");
    uv_snapshot!(context.filters(), context.pip_install()
        .args(["--preview-features", "content-addressed-cache", "--link-mode", "copy"])
        .arg("--target")
        .arg(target.path())
        .arg(&wheel), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: .venv/[BIN]/[PYTHON]
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + binary-payload==0.1.0 (from file://[TEMP_DIR]/binary_payload-0.1.0-py3-none-any.whl)
    ");
    assert_eq!(
        context.read_bytes(target.join("binary_payload/native.so")),
        BINARY_PAYLOAD_CONTENTS
    );
    fs_err::hard_link(&archive_binary, object)?;

    // Links outside the cache also keep objects alive after their archives are removed.
    let retained_binary = context.temp_dir.child("retained-native.so");
    fs_err::hard_link(object, &retained_binary)?;
    uv_snapshot!(context.filters(), context.clean().arg("binary-payload"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed [N] files ([SIZE])
    ");
    assert_path_missing(archive);
    assert_eq!(
        context.cache_files(CacheBucket::Files)?,
        vec![object.clone()]
    );
    assert_eq!(
        context.read_bytes(&retained_binary),
        BINARY_PAYLOAD_CONTENTS
    );

    fs_err::remove_file(&retained_binary)?;
    uv_snapshot!(context.filters(), context.prune(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Pruning cache at: [CACHE_DIR]/
    Removed [N] files ([SIZE])
    ");
    let remaining_objects = context.cache_files(CacheBucket::Files)?;
    assert!(remaining_objects.is_empty(), "{remaining_objects:?}");

    Ok(())
}

/// Requires `UV_INTERNAL__TEST_ALT_FS`.
#[test]
fn binary_payload_copy_fallback_uses_archive_file_store() -> Result<()> {
    let Some(context) = uv_test::test_context!("3.12")
        .with_filtered_python_names()
        .with_filtered_virtualenv_bin()
        .with_cache_on_alt_fs()?
    else {
        return Ok(());
    };
    let wheel = binary_payload_wheel(&context)?;

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview-features")
        .arg("content-addressed-cache")
        .arg(&wheel), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    warning: Failed to hardlink files; falling back to full copy. This may lead to degraded performance.
             If the cache and target directories are on different filesystems, hardlinking may not be supported.
             If this is intentional, set `export UV_LINK_MODE=copy` or use `--link-mode=copy` to suppress this warning.
    Installed 1 package in [TIME]
     + binary-payload==0.1.0 (from file://[TEMP_DIR]/binary_payload-0.1.0-py3-none-any.whl)
    ");

    let archive_files = context.cache_files(CacheBucket::Files)?;
    assert_eq!(archive_files.len(), 7);

    let target = context.temp_dir.child("fallback-target");
    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview-features")
        .arg("content-addressed-cache")
        .arg("--target")
        .arg(target.path())
        .arg(&wheel), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: .venv/[BIN]/[PYTHON]
    Resolved 1 package in [TIME]
    warning: Failed to hardlink files; falling back to full copy. This may lead to degraded performance.
             If the cache and target directories are on different filesystems, hardlinking may not be supported.
             If this is intentional, set `export UV_LINK_MODE=copy` or use `--link-mode=copy` to suppress this warning.
    Installed 1 package in [TIME]
     + binary-payload==0.1.0 (from file://[TEMP_DIR]/binary_payload-0.1.0-py3-none-any.whl)
    ");

    assert_eq!(
        context.read_bytes(target.path().join("binary_payload").join("native.so")),
        BINARY_PAYLOAD_CONTENTS,
    );

    Ok(())
}

fn assert_archive_files_share_objects_except_record(
    context: &TestContext,
    objects: &[PathBuf],
) -> Result<()> {
    let mut record_count = 0;
    for path in context.cache_files(CacheBucket::Archive)? {
        let is_record = path.ends_with("RECORD");
        record_count += usize::from(is_record);
        let shares_object = objects
            .iter()
            .any(|object| uv_fs::is_same_file_allow_missing(&path, object) == Some(true));
        assert_eq!(
            shares_object,
            !is_record,
            "unexpected file-store sharing for {}",
            path.display()
        );
    }
    assert_eq!(record_count, 1, "expected one RECORD in the cached archive");
    Ok(())
}

const BINARY_PAYLOAD_CONTENTS: &[u8] = b"binary payload contents\n";

fn binary_payload_wheel(context: &TestContext) -> Result<PathBuf> {
    const METADATA: &[u8] = b"Metadata-Version: 2.1\nName: binary-payload\nVersion: 0.1.0\n";
    const WHEEL: &[u8] =
        b"Wheel-Version: 1.0\nGenerator: uv-test\nRoot-Is-Purelib: true\nTag: py3-none-any\n";
    const RECORD: &[u8] = b"binary_payload/__init__.py,,\n\
binary_payload/module.py,,\n\
binary_payload/native.so,,\n\
binary_payload/plain.so,,\n\
binary_payload/versioned.so.1,,\n\
binary_payload/versioned.so.1.2,,\n\
binary_payload/native.dylib,,\n\
binary_payload/native.DLL,,\n\
binary_payload/native.pyd,,\n\
binary_payload/tool,,\n\
binary_payload/large.dat,,\n\
binary_payload-0.1.0.dist-info/ignored.so,,\n\
binary_payload-0.1.0.dist-info/METADATA,,\n\
binary_payload-0.1.0.dist-info/WHEEL,,\n\
binary_payload-0.1.0.dist-info/RECORD,,\n";

    let wheel = context
        .temp_dir
        .join("binary_payload-0.1.0-py3-none-any.whl");
    let mut writer = ZipFileWriter::new(Vec::new());
    let large_file = vec![0; 2 * 1024 * 1024];
    for (name, contents) in [
        ("binary_payload/__init__.py", &[][..]),
        (
            "binary_payload/module.py",
            b"VALUE = 'not binary'\n" as &[u8],
        ),
        ("binary_payload/native.so", BINARY_PAYLOAD_CONTENTS),
        ("binary_payload/plain.so", BINARY_PAYLOAD_CONTENTS),
        ("binary_payload/versioned.so.1", BINARY_PAYLOAD_CONTENTS),
        ("binary_payload/versioned.so.1.2", BINARY_PAYLOAD_CONTENTS),
        ("binary_payload/native.dylib", BINARY_PAYLOAD_CONTENTS),
        ("binary_payload/native.DLL", BINARY_PAYLOAD_CONTENTS),
        ("binary_payload/native.pyd", BINARY_PAYLOAD_CONTENTS),
        ("binary_payload/tool", BINARY_PAYLOAD_CONTENTS),
        ("binary_payload/large.dat", large_file.as_slice()),
        (
            "binary_payload-0.1.0.dist-info/ignored.so",
            BINARY_PAYLOAD_CONTENTS,
        ),
        ("binary_payload-0.1.0.dist-info/METADATA", METADATA),
        ("binary_payload-0.1.0.dist-info/WHEEL", WHEEL),
        ("binary_payload-0.1.0.dist-info/RECORD", RECORD),
    ] {
        let entry = ZipEntryBuilder::new(name.into(), Compression::Stored).unix_permissions(
            if matches!(name, "binary_payload/native.so" | "binary_payload/tool") {
                0o755
            } else {
                0o644
            },
        );
        block_on(writer.write_entry_whole(entry, contents))?;
    }
    fs_err::write(&wheel, block_on(writer.close())?)?;

    Ok(wheel)
}
