//! Validated, independently owned wheel-uninstall fixtures.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::Metadata;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use anyhow::{Context, Result, bail, ensure};
use async_zip::base::read::mem::ZipFileReader;
use async_zip::base::write::ZipFileWriter;
use async_zip::{AttributeCompatibility, Compression, ZipEntryBuilder};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use futures::executor::block_on;
use futures::io::AsyncReadExt;
use sha2::{Digest, Sha256, Sha384, Sha512};
use tempfile::{Builder, TempDir};
use walkdir::WalkDir;

use uv_cache_info::CacheInfo;
use uv_distribution_filename::WheelFilename;
use uv_install_wheel::{InstallState, Layout, LinkMode, RecordEntry, Uninstall, WheelFile};
use uv_pep440::Version;
use uv_pep508::VerbatimUrl;
use uv_preview::Preview;
use uv_pypi_types::{DirectUrl, Metadata10, Scheme};

const MANYFILES_FILENAME: &str = "manyfiles-0.0.0-py3-none-any.whl";
const MANYFILES_COUNT: usize = 10_000;
const MAX_WHEEL_BYTES: u64 = 512 * 1024 * 1024;
const MAX_MEMBER_BYTES: u64 = 256 * 1024 * 1024;
const MAX_EXPANDED_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_MEMBERS: usize = 100_000;
const LEASE_SENTINEL: &str = ".uv-bench-unlink-sentinel";
const SENTINEL_BYTES: &[u8] = b"uv-bench wheel-unlink sentinel\n";

/// Keeps a unique mutation root alive until every operation using it has retired.
#[derive(Debug)]
pub(super) struct FixtureLease {
    directory: Option<TempDir>,
    path: PathBuf,
    quarantined: AtomicBool,
}

impl FixtureLease {
    fn new(directory: TempDir) -> Result<Arc<Self>> {
        let metadata = fs_err::symlink_metadata(directory.path())?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "fixture lease is not an owned directory"
        );
        let path = fs_err::canonicalize(directory.path())?;
        Ok(Arc::new(Self {
            directory: Some(directory),
            path,
            quarantined: AtomicBool::new(false),
        }))
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    /// Retain the directory if a submitted mutation cannot be proved complete.
    #[cfg_attr(
        not(all(
            target_os = "linux",
            any(
                target_arch = "x86_64",
                target_arch = "aarch64",
                target_arch = "riscv64",
                target_arch = "loongarch64",
                target_arch = "powerpc64"
            )
        )),
        allow(
            dead_code,
            reason = "Used by the Linux mutation driver and integration tests"
        )
    )]
    pub(super) fn quarantine(&self) {
        self.quarantined.store(true, Ordering::Release);
    }

    pub(super) fn is_quarantined(&self) -> bool {
        self.quarantined.load(Ordering::Acquire)
    }
}

impl Drop for FixtureLease {
    fn drop(&mut self) {
        if self.is_quarantined()
            && let Some(directory) = self.directory.take()
        {
            // Pending unlinks must never reach a later tree created at this pathname.
            drop(directory.keep());
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TreeEntry {
    Directory {
        mode: u32,
    },
    File {
        mode: u32,
        size: u64,
        sha256: [u8; 32],
    },
    Symlink {
        target: PathBuf,
    },
}

type TreeSnapshot = BTreeMap<PathBuf, TreeEntry>;

/// Independent leaves beneath a fresh, owned root. No external deletion base is accepted.
#[derive(Debug)]
pub(super) struct OwnedLeafTrial {
    lease: Arc<FixtureLease>,
    paths: Vec<PathBuf>,
    relative_paths: Vec<PathBuf>,
    initial: TreeSnapshot,
}

impl OwnedLeafTrial {
    /// Capture a freshly created test tree and validate its ordered leaf paths.
    pub(super) fn new(directory: TempDir, relative_paths: Vec<PathBuf>) -> Result<Self> {
        let lease = FixtureLease::new(directory)?;
        validate_independent_paths(&relative_paths)?;
        let sentinel = Path::new(LEASE_SENTINEL);
        for relative in &relative_paths {
            ensure!(
                !relative.starts_with(sentinel) && !sentinel.starts_with(relative),
                "a deletion path overlaps the fixture sentinel"
            );
            validate_ancestors(lease.path(), relative)?;
        }
        let mut guard = fs_err::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(lease.path().join(sentinel))?;
        guard.write_all(SENTINEL_BYTES)?;
        drop(guard);
        let initial = snapshot(lease.path())?;
        let paths = relative_paths
            .iter()
            .map(|relative| lease.path().join(relative))
            .collect();
        Ok(Self {
            lease,
            paths,
            relative_paths,
            initial,
        })
    }

    pub(super) fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    pub(super) fn relative_paths(&self) -> &[PathBuf] {
        &self.relative_paths
    }

    pub(super) fn lease(&self) -> Arc<FixtureLease> {
        Arc::clone(&self.lease)
    }

    /// Verify that exactly the successful leaf operations changed the tree.
    pub(super) fn assert_leaf_results(&self, results: &[io::Result<()>]) -> Result<()> {
        ensure!(
            !self.lease.is_quarantined(),
            "cannot inspect a quarantined mutation fixture"
        );
        ensure!(
            results.len() == self.relative_paths.len(),
            "leaf result count differs from the ordered input"
        );
        let mut expected = self.initial.clone();
        for (relative, result) in self.relative_paths.iter().zip(results) {
            match result {
                Ok(()) => match expected.remove(relative) {
                    Some(TreeEntry::File { .. } | TreeEntry::Symlink { .. }) => {}
                    Some(TreeEntry::Directory { .. }) => {
                        bail!(
                            "leaf removal unexpectedly removed directory {}",
                            relative.display()
                        );
                    }
                    None => bail!(
                        "leaf removal succeeded for missing path {}",
                        relative.display()
                    ),
                },
                Err(error) => {
                    if !expected.contains_key(relative) {
                        ensure!(
                            error.kind() == io::ErrorKind::NotFound,
                            "missing leaf returned an unexpected error: {}: {error}",
                            relative.display()
                        );
                    }
                }
            }
        }
        ensure!(
            snapshot(self.lease.path())? == expected,
            "leaf deletion changed an unexpected file, directory, mode, or sentinel"
        );
        Ok(())
    }
}

#[derive(Debug)]
struct ProductionExpectation {
    tree: TreeSnapshot,
    file_count: usize,
    dir_count: usize,
}

/// A byte-faithful copy of a production copy-mode installation.
#[derive(Debug)]
pub(super) struct Trial {
    leaves: OwnedLeafTrial,
    layout: Layout,
    dist_info: PathBuf,
    production: Arc<ProductionExpectation>,
}

impl Trial {
    pub(super) fn leaves(&self) -> &OwnedLeafTrial {
        &self.leaves
    }

    pub(super) fn layout(&self) -> &Layout {
        &self.layout
    }

    pub(super) fn dist_info(&self) -> &Path {
        &self.dist_info
    }

    pub(super) fn assert_leaf_results(&self, results: &[io::Result<()>]) -> Result<()> {
        ensure!(
            results.iter().all(Result::is_ok),
            "a present regular performance leaf was not removed"
        );
        self.leaves.assert_leaf_results(results)
    }

    pub(super) fn assert_production_success(&self, result: &Uninstall) -> Result<()> {
        ensure!(
            !self.leaves.lease.is_quarantined(),
            "cannot inspect a quarantined mutation fixture"
        );
        ensure!(
            result.file_count == self.production.file_count
                && result.dir_count == self.production.dir_count,
            "production uninstall counts differ from the untimed reference"
        );
        ensure!(
            snapshot(self.leaves.lease.path())? == self.production.tree,
            "production uninstall changed an unexpected file, directory, mode, or sentinel"
        );
        Ok(())
    }
}

/// An immutable installed template plus the identities needed to reproduce each trial.
#[derive(Debug)]
pub(super) struct WheelFixture {
    _directory: TempDir,
    scratch_parent: PathBuf,
    archive: PathBuf,
    template: PathBuf,
    template_tree: TreeSnapshot,
    relative_paths: Vec<PathBuf>,
    dist_info_relative: PathBuf,
    name: String,
    wheel_filename: String,
    sha256: String,
    installed_manifest_sha256: String,
    production: Arc<ProductionExpectation>,
}

impl WheelFixture {
    pub(super) fn manyfiles(scratch_parent: &Path) -> Result<Self> {
        let scratch_parent = canonical_scratch_parent(scratch_parent)?;
        let source = Builder::new()
            .prefix("uv-unlink-manyfiles-input-")
            .tempdir_in(&scratch_parent)?;
        let path = source.path().join(MANYFILES_FILENAME);
        let bytes = manyfiles_wheel()?;
        let sha256 = hex::encode(Sha256::digest(&bytes));
        fs_err::write(&path, bytes)?;
        Self::from_wheel(&path, &sha256, &scratch_parent)
    }

    pub(super) fn from_wheel(
        path: &Path,
        required_sha256: &str,
        scratch_parent: &Path,
    ) -> Result<Self> {
        let required_sha256 = parse_sha256(required_sha256)?;
        let scratch_parent = canonical_scratch_parent(scratch_parent)?;
        let filename = path
            .file_name()
            .and_then(|filename| filename.to_str())
            .context("wheel input must have a UTF-8 wheel filename")?;
        let filename = WheelFilename::from_str(filename)?;
        let source = fs_err::canonicalize(path)?;
        ensure!(
            fs_err::metadata(&source)?.is_file(),
            "wheel input is not a file"
        );
        let mut input = fs_err::File::open(&source)?;
        let mut bytes = Vec::new();
        Read::take(&mut input, MAX_WHEEL_BYTES + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= MAX_WHEEL_BYTES,
            "wheel input exceeds the fixture size limit"
        );
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        ensure!(
            digest == required_sha256,
            "wheel SHA-256 does not match the required digest"
        );
        let sha256 = hex::encode(digest);
        let zip = block_on(ZipFileReader::new(bytes)).context("failed to parse wheel ZIP")?;
        let validated = validate_archive(&zip, &filename)?;

        let directory = Builder::new()
            .prefix("uv-unlink-wheel-template-")
            .tempdir_in(&scratch_parent)?;
        let archive = directory.path().join(filename.to_string());
        fs_err::write(&archive, zip.data())?;
        let extracted = directory.path().join("extracted");
        fs_err::create_dir(&extracted)?;
        let unpacked = uv_extract::unzip(fs_err::File::open(&archive)?, &extracted)?;
        validate_extracted(&extracted, &unpacked, &validated)?;
        ensure!(
            uv_install_wheel::validate_and_heal_record(
                &extracted,
                unpacked.iter().map(|file| (file.path(), file.size())),
                &filename,
            )?
            .is_none(),
            "a strictly validated source RECORD unexpectedly required healing"
        );

        let template = directory.path().join("template");
        fs_err::create_dir(&template)?;
        let layout = make_layout(&template);
        create_scheme(&template, &layout)?;
        let dist_info = uv_install_wheel::installed_dist_info_path(&layout, &extracted)?;
        let site_packages = dist_info
            .parent()
            .context("installed dist-info has no parent")?;
        validate_mapped_members(&template, &layout, site_packages, &validated, &filename)?;
        let direct_url: DirectUrl = serde_json::from_value(serde_json::json!({
            "url": VerbatimUrl::from_absolute_path(&source)?.into_url().to_string(),
            "archive_info": { "hashes": { "sha256": sha256 } },
        }))?;
        let cache_info = CacheInfo::from_path(&archive)?;
        let state = InstallState::new(Preview::default());
        uv_install_wheel::install_wheel(
            &layout,
            false,
            &extracted,
            &filename,
            Some(&direct_url),
            Some(&cache_info),
            None::<&()>,
            Some("uv"),
            true,
            LinkMode::Copy,
            &state,
        )?;
        state.warn_package_conflicts()?;
        let dist_info_relative = relative_under(&dist_info, &template)?;
        let relative_paths = installed_record_paths(&template, &layout, &dist_info, &validated)?;
        let installed_manifest_sha256 = installed_manifest_digest(
            &snapshot(&template)?,
            &fs_err::read(dist_info.join("RECORD"))?,
        )?;
        let unrelated = site_packages.join(".uv-bench-unrelated");
        fs_err::create_dir(&unrelated).context("wheel overlaps the unrelated-package sentinel")?;
        fs_err::write(unrelated.join("sentinel"), SENTINEL_BYTES)?;
        let template_tree = snapshot(&template)?;

        // The complete production path supplies the directory-cleanup oracle outside timing.
        let reference = copy_trial(&scratch_parent, &template, &template_tree, &relative_paths)?;
        let reference_layout = make_layout(reference.lease.path());
        let reference_dist_info = reference.lease.path().join(&dist_info_relative);
        let result =
            uv_install_wheel::uninstall_wheel(&reference_dist_info, &filename, &reference_layout)?;
        ensure!(
            result.file_count == relative_paths.len(),
            "production reference did not remove every installed RECORD leaf"
        );
        let production = Arc::new(ProductionExpectation {
            tree: snapshot(reference.lease.path())?,
            file_count: result.file_count,
            dir_count: result.dir_count,
        });

        Ok(Self {
            _directory: directory,
            scratch_parent,
            archive,
            template,
            template_tree,
            relative_paths,
            dist_info_relative,
            name: filename.name.to_string(),
            wheel_filename: filename.to_string(),
            sha256,
            installed_manifest_sha256,
            production,
        })
    }

    pub(super) fn name(&self) -> &str {
        &self.name
    }

    pub(super) fn wheel_filename(&self) -> &str {
        &self.wheel_filename
    }

    pub(super) fn leaf_count(&self) -> usize {
        self.relative_paths.len()
    }

    pub(super) fn sha256(&self) -> &str {
        &self.sha256
    }

    /// Hash the production-installed tree and exact RECORD bytes, excluding benchmark sentinels.
    pub(super) fn installed_manifest_sha256(&self) -> &str {
        &self.installed_manifest_sha256
    }

    pub(super) fn trial(&self) -> Result<Trial> {
        ensure!(
            hex::encode(hash_file(&self.archive)?) == self.sha256,
            "owned source wheel changed"
        );
        let leaves = copy_trial(
            &self.scratch_parent,
            &self.template,
            &self.template_tree,
            &self.relative_paths,
        )?;
        let layout = make_layout(leaves.lease.path());
        let dist_info = leaves.lease.path().join(&self.dist_info_relative);
        Ok(Trial {
            leaves,
            layout,
            dist_info,
            production: Arc::clone(&self.production),
        })
    }
}

#[derive(Debug)]
struct ArchiveMember {
    index: usize,
    directory: bool,
    size: u64,
}

#[derive(Debug)]
struct ValidatedArchive {
    dist_info: PathBuf,
    files: BTreeMap<PathBuf, (u64, [u8; 32])>,
    signatures: BTreeSet<PathBuf>,
}

fn validate_archive(zip: &ZipFileReader, filename: &WheelFilename) -> Result<ValidatedArchive> {
    ensure!(
        zip.file().entries().len() <= MAX_MEMBERS,
        "wheel has too many archive members"
    );
    let mut members = BTreeMap::new();
    let mut expanded_size = 0_u64;
    let mut dist_infos = BTreeSet::new();
    for (index, entry) in zip.file().entries().iter().enumerate() {
        let directory = entry.dir()?;
        let path = member_path(entry.filename().as_str()?, directory)?;
        let mode = entry.external_file_attribute() >> 16;
        let kind = mode & 0o170_000;
        ensure!(
            if directory {
                kind == 0 || kind == 0o040_000
            } else {
                kind == 0 || kind == 0o100_000
            },
            "wheel contains a symlink or special archive member: {}",
            path.display()
        );
        ensure!(
            entry.uncompressed_size() <= MAX_MEMBER_BYTES,
            "wheel member exceeds the fixture size limit: {}",
            path.display()
        );
        expanded_size = expanded_size
            .checked_add(entry.uncompressed_size())
            .context("wheel expanded size overflow")?;
        ensure!(
            expanded_size <= MAX_EXPANDED_BYTES,
            "wheel exceeds the expanded fixture size limit"
        );
        if let Some(component) = path.components().next()
            && component
                .as_os_str()
                .to_string_lossy()
                .ends_with(".dist-info")
        {
            dist_infos.insert(PathBuf::from(component.as_os_str()));
        }
        ensure!(
            members
                .insert(
                    path.clone(),
                    ArchiveMember {
                        index,
                        directory,
                        size: entry.uncompressed_size()
                    }
                )
                .is_none(),
            "duplicate wheel member: {}",
            path.display()
        );
    }
    for path in members.keys() {
        for ancestor in path
            .ancestors()
            .skip(1)
            .take_while(|path| !path.as_os_str().is_empty())
        {
            if let Some(member) = members.get(ancestor) {
                ensure!(
                    member.directory,
                    "wheel file overlaps a descendant: {}",
                    ancestor.display()
                );
            }
        }
    }
    ensure!(
        dist_infos.len() == 1,
        "wheel must contain exactly one top-level .dist-info directory"
    );
    let dist_info = dist_infos
        .into_iter()
        .next()
        .context("missing dist-info directory")?;
    let expected_dist_info = PathBuf::from(format!(
        "{}-{}.dist-info",
        filename.name.as_dist_info_name(),
        filename.version
    ));
    ensure!(
        dist_info == expected_dist_info,
        "unsupported non-matching dist-info directory: {}",
        dist_info.display()
    );
    let record_path = dist_info.join("RECORD");
    let record_member = members
        .get(&record_path)
        .context("wheel is missing RECORD")?;
    ensure!(!record_member.directory, "wheel RECORD is a directory");
    let record_bytes = read_zip_member(zip, record_member)?;
    let mut record = BTreeMap::new();
    for entry in raw_record(&record_bytes)? {
        let path = member_path(&entry.path, false)?;
        ensure!(
            record.insert(path.clone(), entry).is_none(),
            "duplicate source RECORD destination: {}",
            path.display()
        );
    }
    let signatures = BTreeSet::from([dist_info.join("RECORD.jws"), dist_info.join("RECORD.p7s")]);
    let mut present_signatures = BTreeSet::new();
    let mut files = BTreeMap::new();
    let mut metadata = None;
    let mut wheel_metadata = None;
    for (path, member) in &members {
        if member.directory {
            ensure!(
                member.size == 0,
                "wheel directory has a payload: {}",
                path.display()
            );
            read_zip_member(zip, member)?;
            ensure!(
                !record.contains_key(path),
                "source RECORD names an archive directory: {}",
                path.display()
            );
            continue;
        }
        let bytes = read_zip_member(zip, member)?;
        if signatures.contains(path) {
            ensure!(
                !record.contains_key(path),
                "wheel signature must not be listed in RECORD: {}",
                path.display()
            );
            present_signatures.insert(path.clone());
        } else {
            let entry = record.remove(path).with_context(|| {
                format!("wheel member is missing from RECORD: {}", path.display())
            })?;
            if path == &record_path {
                ensure!(
                    entry.hash.is_none() && entry.size.is_none(),
                    "RECORD must not hash itself"
                );
            } else {
                verify_record_contents(&entry, &bytes)?;
            }
        }
        if path == &dist_info.join("METADATA") {
            metadata = Some(bytes.clone());
        }
        if path == &dist_info.join("WHEEL") {
            wheel_metadata = Some(bytes.clone());
        }
        files.insert(
            path.clone(),
            (bytes.len() as u64, Sha256::digest(&bytes).into()),
        );
    }
    ensure!(
        record.is_empty(),
        "source RECORD names files absent from the wheel"
    );
    let metadata = Metadata10::parse_pkg_info(&metadata.context("wheel is missing METADATA")?)?;
    ensure!(
        metadata.name == filename.name,
        "wheel metadata name differs from its filename"
    );
    let version = Version::from_str(&metadata.version)?;
    ensure!(
        version == filename.version || version == filename.version.clone().without_local(),
        "wheel metadata version differs from its filename"
    );
    let wheel_bytes = wheel_metadata.context("wheel is missing WHEEL")?;
    let wheel = WheelFile::parse(std::str::from_utf8(&wheel_bytes)?)?;
    let mut tags = BTreeSet::new();
    for tag in wheel
        .tags()
        .context("WHEEL is missing compatibility tags")?
    {
        let tagged = WheelFilename::from_str(&format!(
            "{}-{}-{tag}.whl",
            filename.name.as_dist_info_name(),
            filename.version
        ))?;
        tags.extend(expanded_tags(&tagged));
    }
    ensure!(
        tags == expanded_tags(filename),
        "WHEEL compatibility tags differ from its filename"
    );
    Ok(ValidatedArchive {
        dist_info,
        files,
        signatures: present_signatures,
    })
}

fn expanded_tags(filename: &WheelFilename) -> BTreeSet<String> {
    let mut tags = BTreeSet::new();
    for python in filename.python_tags() {
        for abi in filename.abi_tags() {
            for platform in filename.platform_tags() {
                tags.insert(format!("{python}-{abi}-{platform}"));
            }
        }
    }
    tags
}

fn read_zip_member(zip: &ZipFileReader, member: &ArchiveMember) -> Result<Vec<u8>> {
    let mut reader = block_on(zip.reader_with_entry(member.index))?;
    let mut bytes = Vec::new();
    block_on(AsyncReadExt::take(&mut reader, member.size + 1).read_to_end(&mut bytes))?;
    ensure!(
        bytes.len() as u64 == member.size,
        "wheel member size differs from the ZIP directory"
    );
    ensure!(
        reader.compute_hash() == reader.entry().crc32(),
        "wheel member has an invalid ZIP CRC32"
    );
    Ok(bytes)
}

fn raw_record(bytes: &[u8]) -> Result<Vec<RecordEntry>> {
    csv::ReaderBuilder::new()
        .has_headers(false)
        .escape(Some(b'"'))
        .from_reader(bytes)
        .into_deserialize()
        .collect::<std::result::Result<Vec<RecordEntry>, csv::Error>>()
        .context("invalid raw RECORD CSV")
}

fn verify_record_contents(entry: &RecordEntry, bytes: &[u8]) -> Result<()> {
    ensure!(
        entry.size == Some(bytes.len() as u64),
        "RECORD size differs for {:?}",
        entry.path
    );
    let (algorithm, encoded) = entry
        .hash
        .as_deref()
        .and_then(|hash| hash.split_once('='))
        .with_context(|| format!("RECORD is missing a content hash for {:?}", entry.path))?;
    let expected = URL_SAFE_NO_PAD
        .decode(encoded)
        .context("invalid RECORD hash encoding")?;
    let actual = match algorithm {
        "sha256" => Sha256::digest(bytes).to_vec(),
        "sha384" => Sha384::digest(bytes).to_vec(),
        "sha512" => Sha512::digest(bytes).to_vec(),
        _ => bail!("unsupported RECORD hash algorithm: {algorithm}"),
    };
    ensure!(
        actual == expected,
        "RECORD content hash differs for {:?}",
        entry.path
    );
    Ok(())
}

fn validate_extracted(
    extracted: &Path,
    unpacked: &[uv_extract::dirhash::UnhashedFile],
    validated: &ValidatedArchive,
) -> Result<()> {
    let mut actual = BTreeMap::new();
    for file in unpacked {
        ensure!(
            actual
                .insert(file.path().to_path_buf(), file.size())
                .is_none(),
            "extractor returned a duplicate member"
        );
    }
    let expected = validated
        .files
        .iter()
        .map(|(path, (size, _))| (path.clone(), *size))
        .collect::<BTreeMap<_, _>>();
    ensure!(
        actual == expected,
        "extraction changed the raw ZIP member destinations"
    );
    let tree = snapshot(extracted)?;
    let mut extracted_files = BTreeMap::new();
    for (path, entry) in tree {
        match entry {
            TreeEntry::Directory { .. } => {}
            TreeEntry::File { size, sha256, .. } => {
                extracted_files.insert(path, (size, sha256));
            }
            TreeEntry::Symlink { .. } => bail!("extraction created an unexpected symlink"),
        }
    }
    ensure!(
        extracted_files == validated.files,
        "extracted wheel contents differ from the validated archive"
    );
    Ok(())
}

fn validate_mapped_members(
    root: &Path,
    layout: &Layout,
    site_packages: &Path,
    validated: &ValidatedArchive,
    filename: &WheelFilename,
) -> Result<()> {
    let data_prefix = validated.dist_info.with_extension("data");
    let mut destinations = Vec::new();
    for path in validated.files.keys() {
        let destination = if let Ok(relative) = path.strip_prefix(&data_prefix) {
            let mut components = relative.components();
            let category = components
                .next()
                .context("wheel data member has no category")?;
            let suffix = components.as_path();
            ensure!(
                !suffix.as_os_str().is_empty(),
                "wheel data member has no destination"
            );
            match category.as_os_str().to_str() {
                Some("purelib") => layout.scheme.purelib.join(suffix),
                Some("platlib") => layout.scheme.platlib.join(suffix),
                Some("scripts") => layout.scheme.scripts.join(suffix),
                Some("data") => layout.scheme.data.join(suffix),
                Some("headers") => layout
                    .scheme
                    .include
                    .join(filename.name.as_str())
                    .join(suffix),
                _ => bail!("unsupported wheel data category: {category:?}"),
            }
        } else {
            site_packages.join(path)
        };
        destinations.push(relative_under(&destination, root)?);
    }
    for sidecar in ["REQUESTED", "direct_url.json", "uv_cache.json", "INSTALLER"] {
        destinations.push(relative_under(
            &site_packages.join(&validated.dist_info).join(sidecar),
            root,
        )?);
    }
    validate_independent_paths(&destinations)?;
    for relative in destinations {
        validate_ancestors(root, &relative)?;
    }
    Ok(())
}

fn installed_record_paths(
    root: &Path,
    layout: &Layout,
    dist_info: &Path,
    validated: &ValidatedArchive,
) -> Result<Vec<PathBuf>> {
    let site_packages = dist_info
        .parent()
        .context("installed dist-info has no parent")?;
    let record_path = dist_info.join("RECORD");
    let bytes = fs_err::read(&record_path)?;
    let raw = raw_record(&bytes)?;
    let record = uv_install_wheel::read_record(bytes.as_slice())?;
    ensure!(raw == record, "installed RECORD needed path rewriting");
    let environment = &layout.scheme.data;
    let roots = scheme_roots(layout);
    let mut paths = Vec::with_capacity(record.len());
    for entry in record {
        let path = Path::new(&entry.path);
        ensure!(
            !entry.path.contains(['\\', '\0']) && !path.is_absolute(),
            "invalid installed RECORD path: {:?}",
            entry.path
        );
        ensure!(
            path.components().all(|component| match component {
                Component::Normal(_) | Component::CurDir | Component::ParentDir => true,
                Component::Prefix(_) | Component::RootDir => false,
            }),
            "installed RECORD contains a path prefix"
        );
        let destination = uv_fs::normalize_path_under(site_packages.join(path), environment)
            .with_context(|| {
                format!(
                    "installed RECORD escapes the owned environment: {:?}",
                    entry.path
                )
            })?;
        ensure!(
            roots.iter().any(|root| destination.starts_with(root)),
            "installed RECORD escapes the installation scheme"
        );
        let relative = relative_under(&destination, root)?;
        validate_ancestors(root, &relative)?;
        let metadata = fs_err::symlink_metadata(&destination)?;
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "performance RECORD leaf is not a present regular file: {}",
            relative.display()
        );
        if destination != record_path {
            verify_record_contents(&entry, &fs_err::read(&destination)?)?;
        }
        paths.push(relative);
    }
    validate_independent_paths(&paths)?;
    let mut expected_files = paths.iter().cloned().collect::<BTreeSet<_>>();
    for signature in &validated.signatures {
        expected_files.insert(relative_under(&site_packages.join(signature), root)?);
    }
    let mut actual_files = BTreeSet::new();
    for (path, entry) in snapshot(root)? {
        match entry {
            TreeEntry::Directory { .. } => {}
            TreeEntry::File { .. } => {
                actual_files.insert(path);
            }
            TreeEntry::Symlink { .. } => bail!("installed template contains an unexpected symlink"),
        }
    }
    ensure!(
        actual_files == expected_files,
        "installed files and RECORD destinations differ"
    );
    Ok(paths)
}

fn member_path(name: &str, directory: bool) -> Result<PathBuf> {
    let name = if directory {
        name.strip_suffix('/')
            .context("ZIP directory has no trailing slash")?
    } else {
        name
    };
    ensure!(
        !name.is_empty() && !name.contains(['\\', '\0', ':']),
        "unsupported or escaping archive member: {name:?}"
    );
    ensure!(
        name.split('/')
            .all(|component| !component.is_empty() && component != "." && component != ".."),
        "non-normalized archive member: {name:?}"
    );
    let path = PathBuf::from(name);
    validate_relative_path(&path)?;
    Ok(path)
}

fn validate_relative_path(path: &Path) -> Result<()> {
    ensure!(!path.as_os_str().is_empty(), "empty deletion destination");
    ensure!(
        !path.as_os_str().as_encoded_bytes().contains(&0),
        "deletion destination contains NUL"
    );
    ensure!(
        path.components().all(|component| match component {
            Component::Normal(_) => true,
            Component::Prefix(_)
            | Component::RootDir
            | Component::CurDir
            | Component::ParentDir => false,
        }),
        "destination is not a normalized relative path: {}",
        path.display()
    );
    ensure!(
        uv_fs::normalize_path(path).as_os_str() == path.as_os_str(),
        "destination is not normalized: {}",
        path.display()
    );
    Ok(())
}

fn validate_independent_paths(paths: &[PathBuf]) -> Result<()> {
    let mut unique = BTreeSet::new();
    for path in paths {
        validate_relative_path(path)?;
        ensure!(
            unique.insert(path.as_path()),
            "duplicate deletion destination: {}",
            path.display()
        );
    }
    for path in paths {
        for ancestor in path
            .ancestors()
            .skip(1)
            .take_while(|path| !path.as_os_str().is_empty())
        {
            ensure!(
                !unique.contains(ancestor),
                "deletion destinations overlap: {} and {}",
                ancestor.display(),
                path.display()
            );
        }
    }
    Ok(())
}

fn validate_ancestors(root: &Path, relative: &Path) -> Result<()> {
    validate_relative_path(relative)?;
    let metadata = fs_err::symlink_metadata(root)?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "owned fixture root is not a directory"
    );
    let mut path = root.to_path_buf();
    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        if components.peek().is_none() {
            break;
        }
        path.push(component);
        match fs_err::symlink_metadata(&path) {
            Ok(metadata) => ensure!(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                "deletion destination has a symlink or non-directory ancestor: {}",
                path.display()
            ),
            Err(error) if error.kind() == io::ErrorKind::NotFound => break,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn relative_under(path: &Path, root: &Path) -> Result<PathBuf> {
    let normalized = uv_fs::normalize_path_under(path, root)
        .with_context(|| format!("destination escapes its owned root: {}", path.display()))?;
    let relative = normalized.strip_prefix(root)?.to_path_buf();
    validate_relative_path(&relative)?;
    Ok(relative)
}

fn canonical_scratch_parent(path: &Path) -> Result<PathBuf> {
    let path = fs_err::canonicalize(path).context("mutation scratch parent must already exist")?;
    let metadata = fs_err::symlink_metadata(&path)?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "mutation scratch parent is not a directory"
    );
    Ok(path)
}

fn make_layout(root: &Path) -> Layout {
    let environment = root.join("environment");
    let (site_packages, scripts, include, executable, os_name) = if cfg!(windows) {
        (
            environment.join("Lib/site-packages"),
            environment.join("Scripts"),
            environment.join("Include"),
            "python.exe",
            "nt",
        )
    } else {
        (
            environment.join("lib/python3.12/site-packages"),
            environment.join("bin"),
            environment.join("include/site/python3.12"),
            "python",
            "posix",
        )
    };
    Layout {
        sys_executable: scripts.join(executable),
        python_version: (3, 12),
        os_name: os_name.to_string(),
        scheme: Scheme {
            purelib: site_packages.clone(),
            platlib: site_packages,
            scripts,
            data: environment,
            include,
        },
    }
}

fn scheme_roots(layout: &Layout) -> [&Path; 5] {
    [
        &layout.scheme.purelib,
        &layout.scheme.platlib,
        &layout.scheme.scripts,
        &layout.scheme.data,
        &layout.scheme.include,
    ]
}

fn create_scheme(root: &Path, layout: &Layout) -> Result<()> {
    for destination in scheme_roots(layout) {
        let relative = relative_under(destination, root)?;
        validate_ancestors(root, &relative)?;
        fs_err::create_dir_all(destination)?;
        ensure!(
            fs_err::canonicalize(destination)? == destination,
            "installation scheme contains a symlink"
        );
    }
    Ok(())
}

fn copy_trial(
    scratch_parent: &Path,
    template: &Path,
    template_tree: &TreeSnapshot,
    relative_paths: &[PathBuf],
) -> Result<OwnedLeafTrial> {
    ensure!(
        snapshot(template)? == *template_tree,
        "installed template changed"
    );
    let directory = Builder::new()
        .prefix("uv-unlink-wheel-trial-")
        .tempdir_in(scratch_parent)?;
    copy_tree(template, directory.path(), template_tree)?;
    ensure!(
        snapshot(directory.path())? == *template_tree,
        "fresh trial differs from installed template"
    );
    OwnedLeafTrial::new(directory, relative_paths.to_vec())
}

fn copy_tree(source: &Path, destination: &Path, tree: &TreeSnapshot) -> Result<()> {
    ensure!(
        fs_err::read_dir(destination)?.next().is_none(),
        "trial root is not empty"
    );
    for (relative, entry) in tree {
        if relative.as_os_str().is_empty() {
            continue;
        }
        validate_ancestors(source, relative)?;
        validate_ancestors(destination, relative)?;
        let from = source.join(relative);
        let to = destination.join(relative);
        match entry {
            TreeEntry::Directory { .. } => fs_err::create_dir(&to)?,
            TreeEntry::File { .. } => {
                let mut input = fs_err::File::open(&from)?;
                let mut output = fs_err::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&to)?;
                io::copy(&mut input, &mut output)?;
                drop(output);
                fs_err::set_permissions(&to, fs_err::metadata(&from)?.permissions())?;
            }
            TreeEntry::Symlink { .. } => bail!("installed template contains a symlink"),
        }
    }
    for (relative, entry) in tree.iter().rev() {
        if let TreeEntry::Directory { .. } = entry {
            fs_err::set_permissions(
                destination.join(relative),
                fs_err::metadata(source.join(relative))?.permissions(),
            )?;
        }
    }
    Ok(())
}

fn snapshot(root: &Path) -> Result<TreeSnapshot> {
    let mut tree = BTreeMap::new();
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry?;
        let path = entry.path();
        let relative = path.strip_prefix(root)?.to_path_buf();
        let metadata = fs_err::symlink_metadata(path)?;
        let value = if metadata.file_type().is_symlink() {
            TreeEntry::Symlink {
                target: fs_err::read_link(path)?,
            }
        } else if metadata.is_dir() {
            TreeEntry::Directory {
                mode: mode(&metadata),
            }
        } else if metadata.is_file() {
            TreeEntry::File {
                mode: mode(&metadata),
                size: metadata.len(),
                sha256: hash_file(path)?,
            }
        } else {
            bail!("fixture contains a special file: {}", relative.display());
        };
        ensure!(
            tree.insert(relative, value).is_none(),
            "duplicate fixture-tree path"
        );
    }
    Ok(tree)
}

fn mode(metadata: &Metadata) -> u32 {
    #[cfg(unix)]
    {
        metadata.permissions().mode() & 0o7777
    }
    #[cfg(not(unix))]
    {
        u32::from(metadata.permissions().readonly())
    }
}

fn hash_file(path: &Path) -> Result<[u8; 32]> {
    let mut file = fs_err::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher.finalize().into())
}

fn installed_manifest_digest(tree: &TreeSnapshot, record: &[u8]) -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(b"uv-bench installed wheel manifest v1\0");
    for (path, entry) in tree {
        let portable = path
            .components()
            .map(|component| {
                component
                    .as_os_str()
                    .to_str()
                    .context("installed manifest path is not UTF-8")
            })
            .collect::<Result<Vec<_>>>()?
            .join("/");
        hasher.update((portable.len() as u64).to_le_bytes());
        hasher.update(portable.as_bytes());
        match entry {
            TreeEntry::Directory { mode } => {
                hasher.update(b"d");
                hasher.update(mode.to_le_bytes());
            }
            TreeEntry::File { mode, size, sha256 } => {
                hasher.update(b"f");
                hasher.update(mode.to_le_bytes());
                hasher.update(size.to_le_bytes());
                hasher.update(sha256);
            }
            TreeEntry::Symlink { .. } => {
                bail!("installed manifest contains an unsupported symlink");
            }
        }
    }
    hasher.update((record.len() as u64).to_le_bytes());
    hasher.update(record);
    Ok(hex::encode(hasher.finalize()))
}

fn parse_sha256(value: &str) -> Result<[u8; 32]> {
    ensure!(
        value.len() == 64,
        "required wheel SHA-256 must contain 64 hexadecimal digits"
    );
    let decoded = hex::decode(value).context("invalid wheel SHA-256")?;
    decoded
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid wheel SHA-256 length"))
}

fn manyfiles_wheel() -> Result<Vec<u8>> {
    let mut writer = ZipFileWriter::new(Vec::new());
    let mut record = csv::WriterBuilder::new()
        .has_headers(false)
        .from_writer(Vec::new());
    for index in 0..MANYFILES_COUNT {
        write_wheel_member(
            &mut writer,
            &mut record,
            &format!("manyfiles/{index}.txt"),
            b"",
        )?;
    }
    write_wheel_member(
        &mut writer,
        &mut record,
        "manyfiles-0.0.0.dist-info/METADATA",
        b"Metadata-Version: 2.1\nName: manyfiles\nVersion: 0.0.0\n",
    )?;
    write_wheel_member(
        &mut writer,
        &mut record,
        "manyfiles-0.0.0.dist-info/WHEEL",
        b"Wheel-Version: 1.0\nGenerator: uv-bench\nRoot-Is-Purelib: true\nTag: py3-none-any\n",
    )?;
    record.serialize(RecordEntry {
        path: "manyfiles-0.0.0.dist-info/RECORD".to_string(),
        hash: None,
        size: None,
    })?;
    let record = record.into_inner()?;
    let entry = ZipEntryBuilder::new(
        "manyfiles-0.0.0.dist-info/RECORD".into(),
        Compression::Stored,
    )
    .attribute_compatibility(AttributeCompatibility::Unix)
    .unix_permissions(0o100_644);
    block_on(writer.write_entry_whole(entry, &record))?;
    Ok(block_on(writer.close())?)
}

fn write_wheel_member(
    writer: &mut ZipFileWriter<Vec<u8>>,
    record: &mut csv::Writer<Vec<u8>>,
    path: &str,
    bytes: &[u8],
) -> Result<()> {
    let entry = ZipEntryBuilder::new(path.into(), Compression::Stored)
        .attribute_compatibility(AttributeCompatibility::Unix)
        .unix_permissions(0o100_644);
    block_on(writer.write_entry_whole(entry, bytes))?;
    record.serialize(RecordEntry {
        path: path.to_string(),
        hash: Some(format!(
            "sha256={}",
            URL_SAFE_NO_PAD.encode(Sha256::digest(bytes))
        )),
        size: Some(bytes.len() as u64),
    })?;
    Ok(())
}
