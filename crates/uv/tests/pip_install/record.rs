use std::collections::BTreeMap;

use anyhow::{Context, Result};
use assert_cmd::prelude::*;
use assert_fs::fixture::ChildPath;
use assert_fs::prelude::*;
use async_zip::base::write::ZipFileWriter;
use async_zip::{Compression, ZipEntryBuilder};
use fs_err::File;
use futures::executor::block_on;
use indoc::indoc;

use uv_cache::{Cache, CacheBucket};
use uv_extract::dirhash::{DirectoryDigest, dirhash_path};
use uv_install_wheel::{read_record, validate_and_heal_record};
use uv_test::packse::{PackseServer, generate_wheel_with_files, scenario::Scenario};
use uv_test::{apply_filters, uv_snapshot};

/// Warn when replacing a distribution without a RECORD, and install a usable replacement.
#[test]
fn reinstall_incomplete() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let scenario = toml::from_str::<Scenario>(indoc! {r#"
        name = "missing-installed-record"
        [root]
        [expected]
        satisfiable = true
        [packages.anyio.versions."3.7.0"]
        sdist = false
        requires = ["idna==3.6", "sniffio==1.3.1"]
        [packages.anyio.versions."4.0.0"]
        sdist = false
        requires = ["idna==3.6", "sniffio==1.3.1"]
        [packages.idna.versions."3.6"]
        sdist = false
        [packages.sniffio.versions."1.3.1"]
        sdist = false
    "#})?;
    let index = PackseServer::from_scenario(&scenario);
    let requirements = context.temp_dir.child("requirements.txt");
    requirements.write_str("anyio==3.7.0")?;

    uv_snapshot!(context.filters(), context.pip_install()
        .args(["-r", "requirements.txt", "--default-index"])
        .arg(index.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==3.7.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    fs_err::remove_file(context.site_packages().join("anyio-3.7.0.dist-info/RECORD"))?;
    requirements.write_str("anyio==4.0.0")?;
    uv_snapshot!(context.filters(), context.pip_install()
        .args(["-r", "requirements.txt", "--default-index"])
        .arg(index.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 1 package in [TIME]
    warning: Failed to uninstall package at `[SITE_PACKAGES]/anyio-3.7.0.dist-info` due to missing `RECORD` file. Installation may result in an incomplete environment.
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - anyio==3.7.0
     + anyio==4.0.0
    ");

    context.assert_installed("anyio", "4.0.0");
    context.assert_installed("idna", "3.6");
    context.assert_installed("sniffio", "1.3.1");
    let record = read_record(File::open(
        context.site_packages().join("anyio-4.0.0.dist-info/RECORD"),
    )?)?;
    assert!(
        record.iter().any(|entry| entry.path == "anyio/__init__.py"),
        "replacement RECORD does not contain the installed module"
    );
    Ok(())
}

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
    // Repair changes the extracted tree, so its archive ID must reflect the repaired RECORD.
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

/// RECORD paths for relocated wheel data use forward slashes.
/// See <https://github.com/astral-sh/uv/issues/14446>.
#[test]
fn record_uses_forward_slashes() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let (filename, bytes) = generate_wheel_with_files(
        &"project".parse()?,
        &"0.1.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[
            (
                "project-0.1.0.data/scripts/my_script",
                "#!python\nprint(\"Hello from script\")\n",
            ),
            (
                "project-0.1.0.data/headers/project.h",
                "#include <stdio.h>\n",
            ),
        ],
    );
    let wheel = context.temp_dir.join(filename);
    fs_err::write(&wheel, bytes)?;
    context
        .pip_install()
        .arg(wheel)
        .arg("--offline")
        .assert()
        .success();

    let site_packages = context.site_packages();
    let record = read_record(File::open(
        site_packages.join("project-0.1.0.dist-info/RECORD"),
    )?)?;
    for entry in &record {
        assert!(
            !entry.path.contains('\\'),
            "RECORD path uses a backslash: {}",
            entry.path
        );
    }
    for (suffix, expected) in [
        ("/my_script", "print(\"Hello from script\")"),
        ("/project.h", "#include <stdio.h>"),
    ] {
        let entry = record
            .iter()
            .find(|entry| entry.path.ends_with(suffix))
            .with_context(|| format!("missing relocated RECORD entry ending in {suffix}"))?;
        let path = site_packages.join(&entry.path);
        assert!(
            fs_err::read_to_string(&path)?.contains(expected),
            "unexpected contents at {}",
            path.display()
        );
    }
    Ok(())
}
