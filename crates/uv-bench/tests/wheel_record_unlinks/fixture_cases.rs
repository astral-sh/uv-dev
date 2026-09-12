use std::io;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

#[cfg(unix)]
use fs_err::os::unix::fs::symlink;

use anyhow::Result;
use async_zip::base::write::ZipFileWriter;
use async_zip::{AttributeCompatibility, Compression, ZipEntryBuilder};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use futures::executor::block_on;
use sha2::{Digest, Sha256};
use uv_install_wheel::RecordEntry;

use super::fixture::{OwnedLeafTrial, WheelFixture};

const FILENAME: &str = "fixture_pkg-1.0.0-py3-none-any.whl";
const DIST_INFO: &str = "fixture_pkg-1.0.0.dist-info";

#[derive(Clone, Copy, Debug)]
enum RecordCorruption {
    Absolute,
    Missing,
    Duplicate,
    Size,
    Hash,
}

#[derive(Clone)]
struct Member {
    path: String,
    bytes: Vec<u8>,
    mode: u16,
}

impl Member {
    fn file(path: &str, bytes: &[u8]) -> Self {
        Self {
            path: path.to_string(),
            bytes: bytes.to_vec(),
            mode: 0o100_644,
        }
    }
}

fn base_members() -> Vec<Member> {
    vec![
        Member::file("fixture_pkg/__init__.py", b"def main():\n    return 0\n"),
        Member::file(
            "fixture_pkg-1.0.0.dist-info/METADATA",
            b"Metadata-Version: 2.1\nName: fixture-pkg\nVersion: 1.0.0\n",
        ),
        Member::file(
            "fixture_pkg-1.0.0.dist-info/WHEEL",
            b"Wheel-Version: 1.0\nGenerator: uv-bench-test\nRoot-Is-Purelib: true\nTag: py3-none-any\n",
        ),
    ]
}

fn wheel_bytes(
    members: &[Member],
    edit_record: impl FnOnce(&mut Vec<RecordEntry>),
) -> Result<Vec<u8>> {
    let mut record = members
        .iter()
        .filter(|member| !member.path.ends_with('/'))
        .map(|member| RecordEntry {
            path: member.path.clone(),
            hash: Some(format!(
                "sha256={}",
                URL_SAFE_NO_PAD.encode(Sha256::digest(&member.bytes))
            )),
            size: Some(member.bytes.len() as u64),
        })
        .collect::<Vec<_>>();
    record.push(RecordEntry {
        path: format!("{DIST_INFO}/RECORD"),
        hash: None,
        size: None,
    });
    edit_record(&mut record);
    let mut record_writer = csv::WriterBuilder::new()
        .has_headers(false)
        .from_writer(Vec::new());
    for entry in record {
        record_writer.serialize(entry)?;
    }
    let record = record_writer.into_inner()?;

    let mut writer = ZipFileWriter::new(Vec::new());
    for member in members {
        let entry = ZipEntryBuilder::new(member.path.clone().into(), Compression::Stored)
            .attribute_compatibility(AttributeCompatibility::Unix)
            .unix_permissions(member.mode);
        block_on(writer.write_entry_whole(entry, &member.bytes))?;
    }
    let entry = ZipEntryBuilder::new(format!("{DIST_INFO}/RECORD").into(), Compression::Stored)
        .attribute_compatibility(AttributeCompatibility::Unix)
        .unix_permissions(0o100_644);
    block_on(writer.write_entry_whole(entry, &record))?;
    Ok(block_on(writer.close())?)
}

fn write_wheel(root: &Path, bytes: &[u8]) -> Result<(PathBuf, String)> {
    let path = root.join(FILENAME);
    fs_err::write(&path, bytes)?;
    Ok((path, hex::encode(Sha256::digest(bytes))))
}

fn simple_fixture(root: &Path) -> Result<(PathBuf, String, WheelFixture)> {
    let bytes = wheel_bytes(&base_members(), |_| {})?;
    let (path, sha256) = write_wheel(root, &bytes)?;
    let fixture = WheelFixture::from_wheel(&path, &sha256, root)?;
    Ok((path, sha256, fixture))
}

fn remove_all(trial: &OwnedLeafTrial) -> Vec<io::Result<()>> {
    trial.paths().iter().map(fs_err::remove_file).collect()
}

#[test]
fn installed_trials_preserve_record_bytes_modes_and_scheme_destinations() -> Result<()> {
    let scratch = tempfile::tempdir()?;
    let mut members = base_members();
    members.extend([
        Member {
            path: "fixture_pkg/executable".to_string(),
            bytes: b"executable bytes\n".to_vec(),
            mode: 0o100_755,
        },
        Member::file(
            "fixture_pkg-1.0.0.dist-info/entry_points.txt",
            b"[console_scripts]\nfixture-command = fixture_pkg:main\n",
        ),
        Member::file(
            "fixture_pkg-1.0.0.data/purelib/extra_pure.txt",
            b"purelib\n",
        ),
        Member::file(
            "fixture_pkg-1.0.0.data/platlib/extra_plat.txt",
            b"platlib\n",
        ),
        Member::file("fixture_pkg-1.0.0.data/headers/header.h", b"header\n"),
        Member::file(
            "fixture_pkg-1.0.0.data/scripts/fixture-helper",
            b"#!python\nprint('helper')\n",
        ),
        Member::file(
            "fixture_pkg-1.0.0.data/data/share/fixture/data.txt",
            b"data\n",
        ),
    ]);
    let bytes = wheel_bytes(&members, |_| {})?;
    let (path, sha256) = write_wheel(scratch.path(), &bytes)?;
    let wheel = WheelFixture::from_wheel(&path, &sha256, scratch.path())?;
    assert_eq!(wheel.name(), "fixture-pkg");
    assert_eq!(wheel.wheel_filename(), FILENAME);
    assert_eq!(wheel.sha256(), sha256);
    assert_eq!(wheel.installed_manifest_sha256().len(), 64);
    assert_eq!(wheel.leaf_count(), 16);

    let first = wheel.trial()?;
    let second = wheel.trial()?;
    let first_lease = first.leaves().lease();
    let second_lease = second.leaves().lease();
    assert_ne!(first_lease.path(), second_lease.path());
    assert_eq!(
        first.leaves().relative_paths(),
        second.leaves().relative_paths()
    );
    assert_eq!(first.leaves().paths().len(), wheel.leaf_count());
    for (relative, absolute) in first
        .leaves()
        .relative_paths()
        .iter()
        .zip(first.leaves().paths())
    {
        assert_eq!(first_lease.path().join(relative), *absolute);
    }
    let first_record = fs_err::read(first.dist_info().join("RECORD"))?;
    assert_eq!(
        first_record,
        fs_err::read(second.dist_info().join("RECORD"))?
    );
    let parsed = uv_install_wheel::read_record(first_record.as_slice())?;
    let site_packages = first.dist_info().parent().expect("dist-info parent");
    let ordered = parsed
        .iter()
        .map(|entry| uv_fs::normalize_path(site_packages.join(&entry.path)).into_owned())
        .collect::<Vec<_>>();
    assert_eq!(ordered, first.leaves().paths());

    let scheme = &first.layout().scheme;
    for owned in [
        &scheme.purelib,
        &scheme.platlib,
        &scheme.scripts,
        &scheme.data,
        &scheme.include,
    ] {
        assert!(owned.starts_with(first_lease.path()));
    }
    assert!(scheme.purelib.join("extra_pure.txt").is_file());
    assert!(scheme.platlib.join("extra_plat.txt").is_file());
    assert!(scheme.include.join("fixture-pkg/header.h").is_file());
    assert!(scheme.scripts.join("fixture-helper").is_file());
    assert!(scheme.data.join("share/fixture/data.txt").is_file());
    let executable = scheme.purelib.join("fixture_pkg/executable");
    assert_eq!(fs_err::read(&executable)?, b"executable bytes\n");
    #[cfg(unix)]
    {
        let first_metadata = fs_err::metadata(&executable)?;
        let second_metadata = fs_err::metadata(
            second
                .layout()
                .scheme
                .purelib
                .join("fixture_pkg/executable"),
        )?;
        // Extraction keeps the OS defaults for non-executable permission bits.
        let first_mode = first_metadata.permissions().mode() & 0o7777;
        let second_mode = second_metadata.permissions().mode() & 0o7777;
        assert_eq!(first_mode & 0o111, 0o111);
        assert_eq!(first_mode, second_mode);
        assert_ne!(
            (first_metadata.dev(), first_metadata.ino()),
            (second_metadata.dev(), second_metadata.ino())
        );
    }

    let results = remove_all(first.leaves());
    first.assert_leaf_results(&results)?;
    let result =
        uv_install_wheel::uninstall_wheel(second.dist_info(), wheel.name(), second.layout())?;
    second.assert_production_success(&result)?;
    assert_eq!(hex::encode(Sha256::digest(fs_err::read(path)?)), sha256);
    Ok(())
}

#[test]
fn manyfiles_has_the_census_record_shape() -> Result<()> {
    let scratch = tempfile::tempdir()?;
    let wheel = WheelFixture::manyfiles(scratch.path())?;
    assert_eq!(wheel.name(), "manyfiles");
    assert_eq!(wheel.wheel_filename(), "manyfiles-0.0.0-py3-none-any.whl");
    assert_eq!(wheel.sha256().len(), 64);
    assert_eq!(wheel.leaf_count(), 10_007);
    let trial = wheel.trial()?;
    assert_eq!(trial.leaves().paths().len(), 10_007);
    trial.assert_leaf_results(&remove_all(trial.leaves()))?;
    Ok(())
}

#[test]
fn required_wheel_digest_is_checked_before_zip_parsing() -> Result<()> {
    let scratch = tempfile::tempdir()?;
    let (path, _) = write_wheel(scratch.path(), b"not a ZIP")?;
    let before = fs_err::read_dir(scratch.path())?.count();
    let error = WheelFixture::from_wheel(&path, &"00".repeat(32), scratch.path())
        .expect_err("wrong outer digest must fail");
    assert_eq!(
        error.to_string(),
        "wheel SHA-256 does not match the required digest"
    );
    assert_eq!(fs_err::read_dir(scratch.path())?.count(), before);
    let error = WheelFixture::from_wheel(&path, "not-a-digest", scratch.path())
        .expect_err("an explicit full digest is required");
    assert_eq!(
        error.to_string(),
        "required wheel SHA-256 must contain 64 hexadecimal digits"
    );
    Ok(())
}

#[test]
fn malformed_raw_record_is_not_healed_into_a_fixture() -> Result<()> {
    let scratch = tempfile::tempdir()?;
    let cases = [
        (
            RecordCorruption::Absolute,
            "non-normalized archive member: \"/fixture_pkg/__init__.py\"",
        ),
        (
            RecordCorruption::Missing,
            "wheel member is missing from RECORD: fixture_pkg/__init__.py",
        ),
        (
            RecordCorruption::Duplicate,
            "duplicate source RECORD destination: fixture_pkg/__init__.py",
        ),
        (
            RecordCorruption::Size,
            "RECORD size differs for \"fixture_pkg/__init__.py\"",
        ),
        (
            RecordCorruption::Hash,
            "RECORD content hash differs for \"fixture_pkg/__init__.py\"",
        ),
    ];
    for (case, expected) in cases {
        let input = tempfile::tempdir_in(scratch.path())?;
        let bytes = wheel_bytes(&base_members(), |record| match case {
            RecordCorruption::Absolute => record[0].path.insert(0, '/'),
            RecordCorruption::Missing => {
                record.remove(0);
            }
            RecordCorruption::Duplicate => record.push(RecordEntry {
                path: record[0].path.clone(),
                hash: record[0].hash.clone(),
                size: record[0].size,
            }),
            RecordCorruption::Size => record[0].size = Some(1),
            RecordCorruption::Hash => {
                record[0].hash = Some(format!("sha256={}", URL_SAFE_NO_PAD.encode([0_u8; 32])));
            }
        })?;
        let (path, digest) = write_wheel(input.path(), &bytes)?;
        let before = fs_err::read_dir(scratch.path())?.count();
        let error = WheelFixture::from_wheel(&path, &digest, scratch.path())
            .expect_err("invalid original RECORD must fail");
        assert_eq!(error.to_string(), expected, "case {case:?}");
        assert_eq!(fs_err::read_dir(scratch.path())?.count(), before);
    }
    Ok(())
}

#[test]
fn archive_paths_and_types_are_checked_before_extraction() -> Result<()> {
    let scratch = tempfile::tempdir()?;
    let cases = [
        (
            "traversal",
            Member::file("../outside", b"outside"),
            "non-normalized archive member: \"../outside\"",
        ),
        (
            "backslash",
            Member::file("..\\outside", b"outside"),
            "unsupported or escaping archive member: \"..\\\\outside\"",
        ),
        (
            "duplicate",
            Member::file("fixture_pkg/__init__.py", b"duplicate"),
            "duplicate wheel member: fixture_pkg/__init__.py",
        ),
        (
            "symlink",
            Member {
                path: "fixture_pkg/link".to_string(),
                bytes: b"../../outside".to_vec(),
                mode: 0o120_777,
            },
            "wheel contains a symlink or special archive member: fixture_pkg/link",
        ),
    ];
    for (case, member, expected) in cases {
        let input = tempfile::tempdir_in(scratch.path())?;
        let mut members = base_members();
        members.push(member);
        let bytes = wheel_bytes(&members, |_| {})?;
        let (path, digest) = write_wheel(input.path(), &bytes)?;
        let before = fs_err::read_dir(scratch.path())?.count();
        let error = WheelFixture::from_wheel(&path, &digest, scratch.path())
            .expect_err("invalid raw archive member must fail");
        assert_eq!(error.to_string(), expected, "case {case}");
        assert_eq!(fs_err::read_dir(scratch.path())?.count(), before);
    }
    Ok(())
}

#[test]
fn mapped_data_destinations_must_be_independent() -> Result<()> {
    let scratch = tempfile::tempdir()?;
    let mut members = base_members();
    members.extend([
        Member::file("fixture_pkg/data.txt", b"root"),
        Member::file(
            "fixture_pkg-1.0.0.data/purelib/fixture_pkg/data.txt",
            b"data",
        ),
    ]);
    let bytes = wheel_bytes(&members, |_| {})?;
    let (path, digest) = write_wheel(scratch.path(), &bytes)?;
    let error = WheelFixture::from_wheel(&path, &digest, scratch.path())
        .expect_err("mapped data must not overwrite another member");
    let site_packages = Path::new("environment").join(if cfg!(windows) {
        "Lib/site-packages"
    } else {
        "lib/python3.12/site-packages"
    });
    assert_eq!(
        error.to_string(),
        format!(
            "duplicate deletion destination: {}",
            site_packages.join("fixture_pkg/data.txt").display()
        )
    );
    Ok(())
}

#[test]
fn owned_leaf_boundary_rejects_invalid_or_overlapping_paths() -> Result<()> {
    let scratch = tempfile::tempdir()?;
    for paths in [
        vec![PathBuf::new()],
        vec![PathBuf::from("../outside")],
        vec![PathBuf::from("/absolute")],
        vec![PathBuf::from("file\0suffix")],
        vec![PathBuf::from("a//b")],
        vec![PathBuf::from("a/./b")],
        vec![PathBuf::from("same"), PathBuf::from("same")],
        vec![PathBuf::from("parent"), PathBuf::from("parent/child")],
        vec![PathBuf::from(".uv-bench-unlink-sentinel")],
    ] {
        let root = tempfile::tempdir_in(scratch.path())?;
        assert!(OwnedLeafTrial::new(root, paths).is_err());
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn owned_leaf_boundary_rejects_symlink_ancestors() -> Result<()> {
    let scratch = tempfile::tempdir()?;
    let outside = tempfile::tempdir_in(scratch.path())?;
    let sentinel = outside.path().join("sentinel");
    fs_err::write(&sentinel, b"unchanged")?;
    let root = tempfile::tempdir_in(scratch.path())?;
    symlink(outside.path(), root.path().join("link"))?;
    let error = OwnedLeafTrial::new(root, vec![PathBuf::from("link/sentinel")])
        .expect_err("a symlink ancestor must fail before mutation");
    assert!(
        error
            .to_string()
            .starts_with("deletion destination has a symlink or non-directory ancestor:")
    );
    assert_eq!(fs_err::read(sentinel)?, b"unchanged");
    Ok(())
}

#[cfg(unix)]
#[test]
fn leaf_oracle_handles_missing_files_symlinks_and_directories() -> Result<()> {
    let scratch = tempfile::tempdir()?;
    let root = tempfile::tempdir_in(scratch.path())?;
    fs_err::write(root.path().join("file"), b"file")?;
    fs_err::create_dir(root.path().join("directory"))?;
    fs_err::create_dir(root.path().join("targets"))?;
    fs_err::write(root.path().join("targets/file"), b"target")?;
    fs_err::create_dir(root.path().join("targets/directory"))?;
    symlink("absent", root.path().join("dangling"))?;
    symlink("targets/file", root.path().join("file-link"))?;
    symlink("targets/directory", root.path().join("directory-link"))?;
    let relative = [
        "file",
        "missing",
        "dangling",
        "file-link",
        "directory-link",
        "directory",
    ]
    .map(PathBuf::from)
    .to_vec();
    let trial = OwnedLeafTrial::new(root, relative.clone())?;
    assert_eq!(trial.relative_paths(), relative);
    let results = remove_all(&trial);
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 4);
    assert_eq!(
        results[1].as_ref().expect_err("missing file").kind(),
        io::ErrorKind::NotFound
    );
    assert!(results[5].is_err());
    trial.assert_leaf_results(&results)?;
    assert_eq!(
        fs_err::read(trial.lease().path().join("targets/file"))?,
        b"target"
    );
    Ok(())
}

#[test]
fn quarantine_outlives_the_installed_template() -> Result<()> {
    let scratch = tempfile::tempdir()?;
    let (path, digest, wheel) = simple_fixture(scratch.path())?;
    let trial = wheel.trial()?;
    let lease = trial.leaves().lease();
    let quarantined_path = lease.path().to_path_buf();
    assert!(!lease.is_quarantined());
    lease.quarantine();
    lease.quarantine();
    assert!(lease.is_quarantined());
    drop(trial);
    drop(wheel);
    drop(lease);
    assert!(quarantined_path.join(".uv-bench-unlink-sentinel").is_file());

    let replacement = WheelFixture::from_wheel(&path, &digest, scratch.path())?;
    let replacement_trial = replacement.trial()?;
    assert_ne!(replacement_trial.leaves().lease().path(), quarantined_path);
    drop(replacement_trial);
    drop(replacement);
    assert!(quarantined_path.is_dir());
    // This test submitted no operations, so its deliberately retained directory is reclaimable.
    fs_err::remove_dir_all(quarantined_path)?;
    Ok(())
}
