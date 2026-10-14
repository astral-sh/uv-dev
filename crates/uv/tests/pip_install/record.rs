use anyhow::Result;
use assert_fs::fixture::ChildPath;
use assert_fs::prelude::*;
use async_zip::base::write::ZipFileWriter;
use async_zip::{Compression, ZipEntryBuilder};
use fs_err::File;
use futures::executor::block_on;
use indoc::indoc;

use uv_cache::{Cache, CacheBucket};
use uv_extract::dirhash::{DirectoryDigest, dirhash_path};
use uv_install_wheel::validate_and_heal_record;
use uv_test::{apply_filters, uv_snapshot};

/// Repair missing and extraneous RECORD entries before caching and installing the wheel.
#[test]
fn handle_record_mismatches() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filter((
        regex::escape("foo-0.1.0.dist-info/uv_cache.json,sha256=") + ".*",
        "foo-0.1.0.dist-info/uv_cache.json,sha256=[SHA256],[SIZE]",
    ));
    let wheel = context.temp_dir.join("foo-0.1.0-py3-none-any.whl");
    // The RECORD omits the module and METADATA, retains valid entries, and names a nonexistent path.
    let entries = [
        ("foo/__init__.py", "__version__ = \"0.1.0\"\n"),
        ("foo/py.typed", ""),
        (
            "foo-0.1.0.dist-info/METADATA",
            "Metadata-Version: 2.3\nName: foo\nVersion: 0.1.0\n",
        ),
        (
            "foo-0.1.0.dist-info/WHEEL",
            "Wheel-Version: 1.0\nGenerator: uv-test\nRoot-Is-Purelib: true\nTag: py3-none-any\n",
        ),
        (
            "foo-0.1.0.dist-info/RECORD",
            indoc! {"
                foo/py.typed,sha256=47DEQpj8HBSa-_TImW-5JCeuQeRkm5NMpJWZG3hSuFU,0
                foo-0.1.0.dist-info/WHEEL,sha256=ujr00BDMtYYidJ71ulklWmNFpiGqy5NyjK1fX-JwFO4,78
                foo-0.1.0.dist-info/RECORD,,
                ../../../../etc/passwd,,0
            "},
        ),
    ];
    let mut writer = ZipFileWriter::new(Vec::new());
    for (name, contents) in entries {
        let entry = ZipEntryBuilder::new(name.into(), Compression::Stored);
        block_on(writer.write_entry_whole(entry, contents.as_bytes()))?;
    }
    fs_err::write(&wheel, block_on(writer.close())?)?;

    let extracted = context.temp_dir.join("foo-extracted");
    let (hashed_files, unhealed_tree) =
        uv_extract::unzip_and_hash(File::open(&wheel)?, &extracted)?;
    let unhealed_digest = DirectoryDigest::from(unhealed_tree.hash());
    assert!(
        validate_and_heal_record(
            &extracted,
            hashed_files.iter().map(|file| (file.path(), file.size())),
            "foo",
        )?
        .is_some(),
        "the malformed fixture must require RECORD repair"
    );
    let healed_digest = DirectoryDigest::from(dirhash_path(&extracted)?);
    assert_ne!(unhealed_digest, healed_digest);

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--find-links")
        .arg(context.temp_dir.as_ref())
        .arg("--offline")
        .arg("--preview-features")
        .arg("content-addressed-cache")
        .args(["--link-mode", "hardlink"])
        .arg("foo"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + foo==0.1.0
    ");

    let archive =
        ChildPath::new(Cache::from_path(context.cache_dir.path()).bucket(CacheBucket::Archive));
    archive
        .child(unhealed_digest.as_str())
        .assert(predicates::path::missing());
    let cached_record = archive
        .child(healed_digest.as_str())
        .child("foo-0.1.0.dist-info/RECORD");
    assert_eq!(
        context.read_bytes(cached_record.path()),
        context.read_bytes(extracted.join("foo-0.1.0.dist-info/RECORD")),
        "installation must not mutate the healed RECORD retained in the archive"
    );

    let installed_record = context.read(context.site_packages().join("foo-0.1.0.dist-info/RECORD"));
    insta::assert_snapshot!(apply_filters(installed_record, context.filters()), @"
    foo-0.1.0.dist-info/INSTALLER,sha256=5hhM4Q4mYTT9z6QB6PGpUAW81PGNFrYrdXMj4oM_6ak,2
    foo-0.1.0.dist-info/METADATA,,47
    foo-0.1.0.dist-info/RECORD,,
    foo-0.1.0.dist-info/REQUESTED,sha256=47DEQpj8HBSa-_TImW-5JCeuQeRkm5NMpJWZG3hSuFU,0
    foo-0.1.0.dist-info/WHEEL,sha256=ujr00BDMtYYidJ71ulklWmNFpiGqy5NyjK1fX-JwFO4,78
    foo-0.1.0.dist-info/uv_cache.json,sha256=[SHA256],[SIZE]
    foo/__init__.py,,22
    foo/py.typed,sha256=47DEQpj8HBSa-_TImW-5JCeuQeRkm5NMpJWZG3hSuFU,0
    ");
    Ok(())
}
