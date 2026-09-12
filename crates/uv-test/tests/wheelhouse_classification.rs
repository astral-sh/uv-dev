//! Read-only compatibility probes for local flat-index type classification.

use std::collections::BTreeMap;
use std::fs::FileType;
use std::io;
use std::path::Path;
use std::str::FromStr;

#[cfg(target_os = "linux")]
use std::ffi::OsString;
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStringExt;
#[cfg(unix)]
use std::path::PathBuf;
#[cfg(unix)]
use std::process::Command;

use anyhow::{Context, Result, ensure};
#[cfg(unix)]
use fs_err::os::unix::fs::symlink;
use fs_err::{DirEntry, ReadDir};
use tempfile::TempDir;
use url::Url;

use uv_cache::Cache;
use uv_client::{BaseClientBuilder, CachedClient, Connectivity, FlatIndexClient};
use uv_distribution_filename::DistFilename;
use uv_distribution_types::{FileLocation, IndexUrl};
use uv_normalize::PackageName;
use uv_pep440::Version;
use uv_test::packse::generate_wheel;

#[cfg(target_os = "linux")]
use rustix::fs::{CWD, Mode, mkfifoat};

#[derive(Debug, PartialEq, Eq)]
struct Projection {
    filename: DistFilename,
    raw_filename: String,
    url: String,
    index: IndexUrl,
}

#[derive(Clone, Copy)]
enum TypeSource {
    Metadata,
    FileType,
}

#[derive(Debug, PartialEq, Eq)]
enum Classification {
    Accepted(Projection),
    Directory,
    UnreadableSymlink,
    NonUtf8,
    InvalidFilename,
}

fn read_entries(path: &Path) -> io::Result<ReadDir> {
    // fs-err delegates both type APIs to the retained standard-library DirEntry.
    fs_err::read_dir(path)
}

fn index(path: &Path) -> Result<IndexUrl> {
    IndexUrl::parse(path.to_str().context("test directory is not UTF-8")?, None).map_err(Into::into)
}

/// Mirror the pinned classifier's ordering, retaining the entry URL and raw link-target check.
fn classify_entry(
    entry: &DirEntry,
    file_type: io::Result<FileType>,
    index: &IndexUrl,
) -> io::Result<Classification> {
    let file_type = file_type?;
    if file_type.is_dir() {
        return Ok(Classification::Directory);
    }
    if file_type.is_symlink() {
        let Ok(target) = entry.path().read_link() else {
            return Ok(Classification::UnreadableSymlink);
        };
        if target.is_dir() {
            return Ok(Classification::Directory);
        }
    }
    let filename = entry.file_name();
    let Some(raw_filename) = filename.to_str() else {
        return Ok(Classification::NonUtf8);
    };
    let url = Url::from_file_path(entry.path())
        .map_err(|()| io::Error::other("test directory must have a file URL"))?;
    let Some(filename) = DistFilename::try_from_normalized_filename(raw_filename) else {
        return Ok(Classification::InvalidFilename);
    };
    Ok(Classification::Accepted(Projection {
        filename,
        raw_filename: raw_filename.to_owned(),
        url: url.to_string(),
        index: index.clone(),
    }))
}

fn collect(path: &Path, source: TypeSource) -> Result<Vec<Projection>> {
    let index = index(path)?;
    let mut entries = Vec::new();
    for entry in read_entries(path)? {
        let entry = entry?;
        let file_type = match source {
            TypeSource::Metadata => entry.metadata().map(|metadata| metadata.file_type()),
            TypeSource::FileType => entry.file_type(),
        };
        if let Classification::Accepted(entry) = classify_entry(&entry, file_type, &index)? {
            entries.push(entry);
        }
    }
    entries.sort_by(|left, right| {
        left.filename
            .cmp(&right.filename)
            .then_with(|| left.index.cmp(&right.index))
    });
    Ok(entries)
}

fn production(path: &Path, cache: &Path) -> Result<Vec<Projection>> {
    let index = index(path)?;
    let cache = Cache::from_path(cache);
    let client = CachedClient::new(
        BaseClientBuilder::default()
            .connectivity(Connectivity::Offline)
            .build()?,
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let (entries, offline) = runtime
        .block_on(
            FlatIndexClient::new(&client, Connectivity::Offline, &cache)
                .fetch_all(std::iter::once(&index)),
        )?
        .into_parts();
    ensure!(!offline, "a local flat index cannot be an offline miss");
    entries
        .into_iter()
        .map(|entry| {
            let (filename, file, index) = entry.into_parts();
            ensure!(
                file.dist_info_metadata.is_none()
                    && file.hashes.is_empty()
                    && file.requires_python.is_none()
                    && file.size.is_none()
                    && file.upload_time_utc_ms.is_none()
                    && file.yanked.is_none(),
                "directory entries must not synthesize index metadata"
            );
            let url = match file.url {
                FileLocation::AbsoluteUrl(url) => url.to_url()?.to_string(),
                FileLocation::RelativeUrl(_, _) => {
                    return Err(anyhow::anyhow!("directory entry has a relative URL"));
                }
            };
            Ok(Projection {
                filename,
                raw_filename: file.filename.to_string(),
                url,
                index,
            })
        })
        .collect()
}

fn assert_equivalent(path: &Path, cache: &Path) -> Result<Vec<Projection>> {
    let ordinary = production(path, cache)?;
    assert_eq!(collect(path, TypeSource::Metadata)?, ordinary);
    assert_eq!(collect(path, TypeSource::FileType)?, ordinary);
    Ok(ordinary)
}

fn write_wheel(directory: &Path, name: &str) -> Result<String> {
    let (filename, bytes) = generate_wheel(
        &PackageName::from_str(name)?,
        &Version::from_str("1.0.0")?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
    );
    fs_err::write(directory.join(&filename), bytes)?;
    Ok(filename)
}

fn retained_entry(directory: &Path, filename: &str) -> Result<DirEntry> {
    read_entries(directory)?
        .find_map(|entry| match entry {
            Ok(entry) if entry.file_name() == filename => Some(Ok(entry)),
            Ok(_) => None,
            Err(err) => Some(Err(err)),
        })
        .transpose()?
        .context("missing test entry")
}

#[cfg(unix)]
fn is_accepted(classification: &Classification) -> bool {
    match classification {
        Classification::Accepted(_) => true,
        Classification::Directory
        | Classification::UnreadableSymlink
        | Classification::NonUtf8
        | Classification::InvalidFilename => false,
    }
}

#[test]
fn stable_directory_matches_production() -> Result<()> {
    let root = TempDir::new()?;
    let wheelhouse = root.path().join("wheelhouse");
    fs_err::create_dir(&wheelhouse)?;
    write_wheel(&wheelhouse, "zebra")?;
    write_wheel(&wheelhouse, "alpha")?;
    // The classifier does not open distribution files; this is a filename-only sdist probe.
    fs_err::write(wheelhouse.join("alpha-1.0.0.tar.gz"), b"")?;
    fs_err::write(wheelhouse.join("not-a-distribution"), b"")?;
    fs_err::write(wheelhouse.join("uppercase-1.0.0-py3-none-any.WHL"), b"")?;
    fs_err::create_dir(wheelhouse.join("directory-1.0.0-py3-none-any.whl"))?;
    let entries = assert_equivalent(&wheelhouse, &root.path().join("cache"))?;
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.raw_filename.as_str())
            .collect::<Vec<_>>(),
        [
            "alpha-1.0.0.tar.gz",
            "alpha-1.0.0-py3-none-any.whl",
            "zebra-1.0.0-py3-none-any.whl"
        ]
    );
    Ok(())
}

#[test]
fn metadata_errors_precede_filename_filtering() -> Result<()> {
    let root = TempDir::new()?;
    fs_err::write(root.path().join("irrelevant"), b"")?;
    let entry = retained_entry(root.path(), "irrelevant")?;
    let error = classify_entry(
        &entry,
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "injected")),
        &index(root.path())?,
    )
    .expect_err("metadata failure must be reported");
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    let missing = root.path().join("missing");
    assert!(production(&missing, &root.path().join("cache")).is_err());
    assert!(collect(&missing, TypeSource::Metadata).is_err());
    assert!(collect(&missing, TypeSource::FileType).is_err());
    Ok(())
}

#[cfg(unix)]
#[test]
fn symlinks_match_production() -> Result<()> {
    let root = TempDir::new()?;
    let wheelhouse = root.path().join("wheelhouse");
    fs_err::create_dir(&wheelhouse)?;
    let target = root.path().join("target");
    fs_err::write(&target, b"")?;
    symlink(&target, wheelhouse.join("file-1.0.0-py3-none-any.whl"))?;
    symlink(
        root.path(),
        wheelhouse.join("directory-1.0.0-py3-none-any.whl"),
    )?;
    symlink(
        root.path().join("absent"),
        wheelhouse.join("dangling-1.0.0-py3-none-any.whl"),
    )?;
    let entries = assert_equivalent(&wheelhouse, &root.path().join("cache"))?;
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.raw_filename.as_str())
            .collect::<Vec<_>>(),
        [
            "dangling-1.0.0-py3-none-any.whl",
            "file-1.0.0-py3-none-any.whl"
        ]
    );
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn non_utf8_and_special_files_match_production() -> Result<()> {
    let root = TempDir::new()?;
    let wheelhouse = root.path().join("wheelhouse");
    fs_err::create_dir(&wheelhouse)?;
    fs_err::write(wheelhouse.join(OsString::from_vec(vec![0xff])), b"")?;
    mkfifoat(
        CWD,
        wheelhouse.join("fifo-1.0.0-py3-none-any.whl"),
        Mode::RUSR | Mode::WUSR,
    )?;
    let entries = assert_equivalent(&wheelhouse, &root.path().join("cache"))?;
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.raw_filename.as_str())
            .collect::<Vec<_>>(),
        ["fifo-1.0.0-py3-none-any.whl"]
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn relative_symlink_targets_use_process_cwd() -> Result<()> {
    const CHILD_ROOT: &str = "UV_WHEELHOUSE_CLASSIFICATION_CHILD_ROOT";
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let root = PathBuf::from(root);
        let entries = assert_equivalent(&root.join("wheelhouse"), &root.join("cache"))?;
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.raw_filename.as_str())
                .collect::<Vec<_>>(),
            ["wheelhouse_only-1.0.0-py3-none-any.whl"]
        );
        return Ok(());
    }
    let root = TempDir::new()?;
    let wheelhouse = root.path().join("wheelhouse");
    let cwd = root.path().join("cwd");
    fs_err::create_dir(&wheelhouse)?;
    fs_err::create_dir(&cwd)?;
    fs_err::create_dir(cwd.join("cwd-directory"))?;
    fs_err::create_dir(wheelhouse.join("wheelhouse-directory"))?;
    symlink(
        "cwd-directory",
        wheelhouse.join("cwd_only-1.0.0-py3-none-any.whl"),
    )?;
    symlink(
        "wheelhouse-directory",
        wheelhouse.join("wheelhouse_only-1.0.0-py3-none-any.whl"),
    )?;
    let output = Command::new(std::env::current_exe()?)
        .args([
            "--exact",
            "relative_symlink_targets_use_process_cwd",
            "--nocapture",
        ])
        .env(CHILD_ROOT, root.path())
        .current_dir(cwd)
        .output()?;
    ensure!(
        output.status.success(),
        "child failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn retained_entry_deletion_exposes_the_freshness_choice() -> Result<()> {
    let root = TempDir::new()?;
    let filename = write_wheel(root.path(), "vanished")?;
    let entry = retained_entry(root.path(), &filename)?;
    let initial_type = entry.file_type()?;
    let index = index(root.path())?;
    fs_err::remove_file(entry.path())?;
    let fresh_error = classify_entry(
        &entry,
        entry.metadata().map(|metadata| metadata.file_type()),
        &index,
    )
    .expect_err("Unix DirEntry metadata must perform a fresh lookup");
    assert_eq!(fresh_error.kind(), io::ErrorKind::NotFound);
    assert!(is_accepted(&classify_entry(
        &entry,
        Ok(initial_type),
        &index
    )?));
    // A filesystem with unknown directory-entry types can need another lookup here.
    match entry.file_type() {
        Ok(file_type) => {
            assert_eq!(file_type, initial_type);
            assert!(is_accepted(&classify_entry(&entry, Ok(file_type), &index)?));
        }
        Err(err) => assert_eq!(err.kind(), io::ErrorKind::NotFound),
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn removed_symlink_is_skipped_after_type_classification() -> Result<()> {
    let root = TempDir::new()?;
    let filename = "removed-1.0.0-py3-none-any.whl";
    symlink(root.path().join("absent"), root.path().join(filename))?;
    let entry = retained_entry(root.path(), filename)?;
    let file_type = entry.file_type()?;
    fs_err::remove_file(entry.path())?;
    assert_eq!(
        classify_entry(&entry, Ok(file_type), &index(root.path())?)?,
        Classification::UnreadableSymlink
    );
    Ok(())
}
