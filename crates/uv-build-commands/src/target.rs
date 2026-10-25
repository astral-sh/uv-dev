//! Target descriptions and artifact checks for the experimental cross-build protocol.

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use async_zip::base::read::seek::ZipFileReader;
use async_zip::base::write::ZipFileWriter;
use async_zip::{Compression, ZipEntryBuilder};
use serde::Deserialize;
use tokio::io::{BufReader, copy};
use tokio_util::compat::{
    FuturesAsyncWriteCompatExt, TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt,
};
use walkdir::WalkDir;

use uv_configuration::TargetTriple;
use uv_distribution_filename::{ExpandedTags, WheelFilename};
use uv_distribution_types::{CachedDist, RequiresPython};
use uv_install_wheel::WheelFile;
use uv_pep508::{MarkerEnvironment, MarkerEnvironmentBuilder};
use uv_platform_tags::{Arch, Os, Tags, TagsOptions};
use uv_pypi_types::ResolutionMetadata;
use uv_python_interpreter::Interpreter;

pub(crate) struct BuildTarget {
    details: Option<(PathBuf, Vec<u8>)>,
    markers: MarkerEnvironment,
    tags: Tags,
}

impl BuildTarget {
    pub(crate) fn read(path: &Path, triple: TargetTriple) -> Result<Self> {
        let path = std::path::absolute(path)?;
        let contents = fs_err::read(&path)?;
        let details: BuildDetails =
            serde_json::from_slice(&contents).context("Invalid target `build-details.json`")?;
        ensure!(
            !details.base_prefix.as_os_str().is_empty(),
            "Target `base_prefix` must not be empty"
        );
        ensure!(
            details.schema_version == "1.0",
            "Only `build-details.json` version 1.0 is supported"
        );
        ensure!(
            details.implementation.name == "cpython",
            "Cross-builds currently support only CPython targets"
        );
        let version = &details.implementation.version;
        ensure!(
            version.major == 3 && version.minor >= 8,
            "Cross-builds require a CPython 3.8 or later target"
        );
        let minor = format!("{}.{}", version.major, version.minor);
        ensure!(
            details.language.version == minor,
            "Target language and implementation versions disagree"
        );
        if let Some(language_version) = &details.language.version_info {
            ensure!(
                language_version == version,
                "Target language and implementation versions disagree"
            );
        }
        let suffix = match version.releaselevel.as_str() {
            "final" => String::new(),
            "alpha" => format!("a{}", version.serial),
            "beta" => format!("b{}", version.serial),
            "candidate" => format!("rc{}", version.serial),
            level => bail!("Unsupported target release level: `{level}`"),
        };
        let full_version = format!("{minor}.{}{suffix}", version.micro);
        let markers = triple.markers(MarkerEnvironment::try_from(MarkerEnvironmentBuilder {
            implementation_name: "cpython",
            implementation_version: &full_version,
            platform_python_implementation: "CPython",
            python_full_version: &full_version,
            python_version: &minor,
            os_name: "",
            platform_machine: "",
            platform_release: "",
            platform_system: "",
            platform_version: "",
            sys_platform: "",
        })?);
        let platform = triple.platform();
        let matches_platform = match platform.os() {
            Os::Manylinux { .. } | Os::Musllinux { .. } => {
                details.platform == format!("linux-{}", platform.arch())
            }
            Os::Macos { .. } => {
                let arch = if platform.arch() == Arch::Aarch64 {
                    "arm64"
                } else {
                    "x86_64"
                };
                details.platform.starts_with("macosx-")
                    && details.platform.ends_with(&format!("-{arch}"))
            }
            Os::Windows => {
                let expected = match platform.arch() {
                    Arch::X86_64 => "win-amd64",
                    Arch::Aarch64 => "win-arm64",
                    Arch::X86 => "win32",
                    arch => bail!("Unsupported Windows target architecture: {arch}"),
                };
                details.platform == expected
            }
            os => bail!("Cross-builds do not yet support target platform {os:?}"),
        };
        ensure!(
            matches_platform,
            "Target platform `{}` disagrees with `--python-platform` ({})",
            details.platform,
            platform.pretty()
        );
        for flag in &details.abi.flags {
            ensure!(
                flag == "t" || flag == "d",
                "Unsupported target ABI flag: `{flag}`"
            );
        }
        let tags = Tags::from_env(
            platform,
            (version.major, version.minor),
            "cpython",
            (version.major, version.minor),
            TagsOptions {
                manylinux_compatible: triple.manylinux_compatible(),
                gil_disabled: details.abi.flags.iter().any(|flag| flag == "t"),
                debug_enabled: details.abi.flags.iter().any(|flag| flag == "d"),
                is_cross: true,
            },
        )?;
        Ok(Self {
            details: Some((path, contents)),
            markers,
            tags,
        })
    }

    pub(crate) fn native(interpreter: &Interpreter) -> Result<Self> {
        Ok(Self {
            details: None,
            markers: interpreter.markers().clone(),
            tags: interpreter.tags()?.clone(),
        })
    }

    pub(crate) fn path(&self) -> Option<&Path> {
        self.details.as_ref().map(|(path, _)| path.as_path())
    }

    pub(crate) fn markers(&self) -> &MarkerEnvironment {
        &self.markers
    }

    pub(crate) fn tags(&self) -> &Tags {
        &self.tags
    }

    pub(crate) fn check_unchanged(&self) -> Result<()> {
        if let Some((path, contents)) = &self.details {
            ensure!(
                fs_err::read(path)? == *contents,
                "Target `build-details.json` changed during the build"
            );
        }
        Ok(())
    }

    /// Check both the archive's metadata and its filename before publishing the output.
    pub(crate) async fn validate_wheel(
        &self,
        path: &Path,
        filename: &WheelFilename,
        prepared_metadata: Option<&Path>,
    ) -> Result<()> {
        ensure!(
            filename.is_compatible(&self.tags),
            "Built wheel `{filename}` is incompatible with the target"
        );
        let file = fs_err::tokio::File::open(path).await?;
        let mut reader = ZipFileReader::new(BufReader::new(file).compat()).await?;
        let (index, stem) = uv_metadata::find_archive_dist_info(
            filename,
            reader
                .file()
                .entries()
                .iter()
                .enumerate()
                .filter_map(|(index, entry)| Some((index, entry.filename().as_str().ok()?))),
        )?;
        let wheel_path = format!("{stem}.dist-info/WHEEL");
        let mut metadata_bytes = Vec::new();
        reader
            .reader_with_entry(index)
            .await?
            .read_to_end_checked(&mut metadata_bytes)
            .await?;
        let metadata = ResolutionMetadata::parse_metadata(&metadata_bytes)?;
        ensure!(
            metadata.name == filename.name && metadata.version == filename.version,
            "Built wheel filename and metadata disagree"
        );
        if let Some(requires_python) = metadata.requires_python {
            let requires_python = RequiresPython::from_specifiers(requires_python);
            ensure!(
                requires_python.contains(&self.markers.python_full_version().version),
                "Built wheel requires Python `{requires_python}`, but the target is Python {}",
                self.markers.python_full_version()
            );
        }
        if let Some(prepared) = prepared_metadata {
            ensure!(
                fs_err::read(prepared.join("METADATA"))? == metadata_bytes,
                "Built wheel metadata differs from prepared metadata"
            );
        }
        let index = reader
            .file()
            .entries()
            .iter()
            .position(|entry| entry.filename().as_str().ok() == Some(wheel_path.as_str()))
            .context("Built wheel has no WHEEL metadata")?;
        let mut wheel_bytes = Vec::new();
        reader
            .reader_with_entry(index)
            .await?
            .read_to_end_checked(&mut wheel_bytes)
            .await?;
        let wheel = WheelFile::parse(std::str::from_utf8(&wheel_bytes)?)?;
        if let Some(prepared) = prepared_metadata
            && prepared.join("WHEEL").is_file()
        {
            ensure!(
                fs_err::read(prepared.join("WHEEL"))? == wheel_bytes,
                "Built WHEEL metadata differs from prepared metadata"
            );
        }
        let metadata_tags = ExpandedTags::parse(
            wheel
                .tags()
                .context("Built wheel has no compatibility tags")?
                .iter()
                .map(String::as_str),
        )?;
        ensure!(
            metadata_tags.matches_filename(filename),
            "Built wheel filename and WHEEL tags disagree"
        );
        Ok(())
    }
}

/// Repackage cached wheel contents, including the cached RECORD, without installing them.
/// Keep the archives private to this build's hook sequence.
pub(crate) async fn wheel_archives(
    wheels: Vec<CachedDist>,
    directory: &Path,
) -> Result<Vec<PathBuf>> {
    fs_err::create_dir_all(directory)?;
    let mut paths = Vec::new();
    for wheel in wheels {
        let path = directory.join(wheel.filename().to_string());
        let mut writer =
            ZipFileWriter::new(fs_err::tokio::File::create(&path).await?.compat_write());
        for entry in WalkDir::new(wheel.path()).min_depth(1).sort_by_file_name() {
            let entry = entry?;
            ensure!(
                !entry.file_type().is_symlink(),
                "Target wheel contains a symlink"
            );
            let relative = entry.path().strip_prefix(wheel.path())?;
            let mut name = relative
                .to_str()
                .context("Non-UTF-8 path in target wheel")?
                .replace('\\', "/");
            if entry.file_type().is_dir() {
                name.push('/');
            }
            let builder = ZipEntryBuilder::new(name.into(), Compression::Stored);
            #[cfg(unix)]
            let builder =
                builder.unix_permissions(u16::try_from(entry.metadata()?.permissions().mode())?);
            if entry.file_type().is_dir() {
                writer.write_entry_whole(builder, &[]).await?;
            } else {
                let mut file = fs_err::tokio::File::open(entry.path()).await?;
                let mut entry_writer = writer.write_entry_seekable(builder).await?.compat_write();
                copy(&mut file, &mut entry_writer).await?;
                entry_writer.into_inner().close().await?;
            }
        }
        writer.close().await?;
        paths.push(path);
    }
    paths.sort();
    Ok(paths)
}

#[derive(Deserialize)]
struct BuildDetails {
    schema_version: String,
    base_prefix: PathBuf,
    platform: String,
    language: Language,
    implementation: Implementation,
    abi: Abi,
}

#[derive(Deserialize)]
struct Language {
    version: String,
    version_info: Option<VersionInfo>,
}

#[derive(Deserialize)]
struct Implementation {
    name: String,
    version: VersionInfo,
}

#[derive(Deserialize, PartialEq, Eq)]
struct VersionInfo {
    major: u8,
    minor: u8,
    micro: u64,
    releaselevel: String,
    serial: u64,
}

#[derive(Deserialize)]
struct Abi {
    flags: Vec<String>,
}
