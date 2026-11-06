use std::ops::Range;
use std::path::{Component, Path};

use anyhow::{Context, Result, bail, ensure};
use async_compression::tokio::bufread::GzipDecoder;
use async_compression::tokio::write::GzipEncoder;
use async_zip::base::read::seek::ZipFileReader;
use async_zip::base::write::ZipFileWriter;
use async_zip::{Compression, ZipDateTime, ZipEntryBuilder};
use base64::{Engine, prelude::BASE64_URL_SAFE_NO_PAD};
use fs_err::tokio::File;
use futures::StreamExt;
use futures::io::{AsyncReadExt as _, BufReader as FuturesBufReader, copy_buf};
use rustc_hash::FxHashSet;
use sha2::{Digest, Sha256};
use tar_codec::{ArchiveBuilder as _, EntryMetadata, TarEncoder};
use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _, AsyncWriteExt as _, BufReader};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use uv_distribution_filename::{
    DistFilename, SourceDistExtension, SourceDistFilename, WheelFilename,
};
use uv_fs::verbatim_path;
use uv_metadata::find_archive_dist_info;

/// Write a copy of a distribution with a packaged `pylock.toml` to a new path.
///
/// An existing `pylock.toml` is replaced. Wheels also get an updated `RECORD` entry.
async fn add_lock_to_distribution(
    source: &Path,
    destination: &Path,
    filename: &DistFilename,
    lock: &[u8],
) -> Result<()> {
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let temporary = tempfile::NamedTempFile::new_in(verbatim_path(parent))
        .context("Failed to create a temporary distribution")?;
    let output = fs_err::tokio::OpenOptions::new()
        .write(true)
        .open(temporary.path())
        .await?;
    match filename {
        DistFilename::SourceDistFilename(filename) => {
            ensure!(
                filename.extension == SourceDistExtension::TarGz,
                "Only `.tar.gz` source distributions can be rewritten with a lock"
            );
            add_to_sdist(source, output, filename, lock).await?;
        }
        DistFilename::WheelFilename(filename) => {
            add_to_wheel(source, output, filename, lock).await?;
        }
    }
    temporary
        .persist_noclobber(verbatim_path(destination))
        .map_err(|error| error.error)
        .context("Failed to save the distribution with its lock")?;
    Ok(())
}

async fn add_to_sdist(
    source: &Path,
    output: File,
    filename: &SourceDistFilename,
    lock: &[u8],
) -> Result<()> {
    let mut archive = tokio_tar::Archive::new(sdist_decoder(File::open(source).await?));
    let mut entries = archive.entries()?;
    let mut top_level = None;
    let mut gnu_format = None;
    let mut existing_lock: Option<Range<u64>> = None;
    let mut has_hard_links = false;
    let mut pyproject_seen = false;
    let mut position = 0;
    while let Some(entry) = entries.next().await {
        let mut entry = entry?;
        gnu_format.get_or_insert_with(|| entry.header().as_gnu().is_some());
        ensure!(
            !entry.header().entry_type().is_gnu_sparse(),
            "Cannot add a lock to a source distribution with sparse entries"
        );
        let global = entry.header().entry_type().is_pax_global_extensions();
        if let Some(extensions) = entry.pax_extensions().await? {
            for extension in extensions {
                let extension = extension?;
                let key = extension.key_bytes();
                ensure!(
                    !(key.starts_with(b"GNU.sparse.")
                        || (global && [b"path".as_slice(), b"linkpath", b"size"].contains(&key))),
                    "Cannot add a lock to a source distribution with these PAX extensions"
                );
            }
        }

        // Copy the original tar headers along with each entry. This preserves PAX and GNU
        // extensions, including local extension headers immediately preceding an entry.
        let end = entry
            .effective_size()
            .checked_next_multiple_of(512)
            .and_then(|size| entry.raw_file_position().checked_add(size))
            .context("Source distribution entry is too large")?;
        ensure!(
            end >= position,
            "Source distribution contains overlapping entries"
        );
        let entry_range = position..end;
        position = end;
        if global {
            continue;
        }

        let path = entry.path()?;
        let mut components = path.components();
        let Some(Component::Normal(root)) =
            components.find(|component| *component != Component::CurDir)
        else {
            bail!(
                "Source distribution contains an invalid path: `{}`",
                path.display()
            );
        };
        let root = root
            .to_str()
            .context("Source distribution root is not UTF-8")?;
        if let Some(top_level) = &top_level {
            ensure!(
                root == top_level,
                "Source distribution contains multiple top-level directories"
            );
        } else {
            let root_filename =
                SourceDistFilename::parsed_normalized_filename(&format!("{root}.tar.gz"))?;
            ensure!(
                root_filename.name == filename.name && root_filename.version == filename.version,
                "Source distribution root does not match its filename"
            );
            top_level = Some(root.to_owned());
        }

        let relative = components.as_path();
        ensure!(
            (!relative.as_os_str().is_empty() || entry.header().entry_type().is_dir())
                && relative
                    .components()
                    .all(|component| matches!(component, Component::Normal(_))),
            "Source distribution contains an invalid path: `{}`",
            path.display()
        );
        if relative == Path::new("pyproject.toml") {
            ensure!(
                !pyproject_seen && entry.header().entry_type().is_file(),
                "Source distribution has an invalid or duplicate pyproject.toml file"
            );
            pyproject_seen = true;
        }
        if relative.starts_with("pylock.toml") {
            ensure!(
                relative == Path::new("pylock.toml")
                    && existing_lock.is_none()
                    && entry.header().entry_type().is_file(),
                "Source distribution has an invalid or duplicate pylock.toml file"
            );
            existing_lock = Some(entry_range);
        }
        has_hard_links |= entry.header().entry_type().is_hard_link();
    }
    let top_level = top_level.context("Source distribution is empty")?;
    ensure!(
        pyproject_seen,
        "Source distribution has no pyproject.toml file"
    );
    // A hard link can reach the existing lock through a symlink. Moving the lock could leave
    // the link's target absent when the archive is extracted.
    ensure!(
        existing_lock.is_none() || !has_hard_links,
        "Cannot replace pylock.toml in a source distribution that contains hard links"
    );
    drop(entries);
    let mut input = archive
        .into_inner()
        .ok()
        .context("Failed to recover the source distribution reader")?
        .into_inner()
        .into_inner();
    input.rewind().await?;
    let mut input = sdist_decoder(input);
    let mut output = GzipEncoder::new(output);
    let mut tail = position;
    if let Some(range) = existing_lock {
        copy_tar_bytes(&mut input, &mut output, range.start).await?;
        copy_tar_bytes(&mut input, &mut tokio::io::sink(), range.end - range.start).await?;
        tail -= range.end;
    }
    copy_tar_bytes(&mut input, &mut output, tail).await?;
    // Read the gzip trailer to validate the checksum of the original archive.
    tokio::io::copy(&mut input, &mut tokio::io::sink()).await?;

    let lock_path = format!("{top_level}/pylock.toml");
    let mut output = if gnu_format.unwrap_or(false) {
        let mut builder = tokio_tar::Builder::new(output);
        let mut header = tokio_tar::Header::new_gnu();
        header.set_entry_type(tokio_tar::EntryType::Regular);
        header.set_size(lock.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(0);
        builder.append_data(&mut header, lock_path, lock).await?;
        builder.into_inner().await?
    } else {
        let mut builder = TarEncoder::new(output).builder();
        builder
            .add_file(lock_path, lock, EntryMetadata::default())
            .await?;
        builder.finish_into_inner().await?.into_inner()
    };
    output.shutdown().await?;
    Ok(())
}

fn sdist_decoder(input: File) -> GzipDecoder<BufReader<File>> {
    let mut input = GzipDecoder::new(BufReader::new(input));
    input.multiple_members(true);
    input
}

async fn copy_tar_bytes(
    input: &mut (impl tokio::io::AsyncRead + Unpin),
    output: &mut (impl tokio::io::AsyncWrite + Unpin),
    size: u64,
) -> Result<()> {
    let copied = tokio::io::copy(&mut input.take(size), output).await?;
    ensure!(
        copied == size,
        "Source distribution ended before its last entry"
    );
    Ok(())
}

async fn add_to_wheel(
    source: &Path,
    output: File,
    filename: &WheelFilename,
    lock: &[u8],
) -> Result<()> {
    let input = File::open(source).await?;
    let mut archive = ZipFileReader::new(BufReader::new(input).compat()).await?;
    let names = archive
        .file()
        .entries()
        .iter()
        .map(|entry| entry.filename().as_str().map(str::to_owned))
        .collect::<Result<Vec<_>, _>>()?;
    let ((), stem) =
        find_archive_dist_info(filename, names.iter().map(|name| ((), name.as_str())))?;
    let lock_path = format!("{stem}.dist-info/pylock.toml");
    let record_path = format!("{stem}.dist-info/RECORD");
    let signatures = [format!("{record_path}.jws"), format!("{record_path}.p7s")];

    let mut writer = ZipFileWriter::new(output.compat_write());
    // The ZIP writer only accepts UTF-8 archive comments.
    if let Ok(comment) = archive.file().comment().as_str() {
        writer.comment(comment.to_owned());
    }
    let mut seen = FxHashSet::default();
    let mut record = None;
    for (index, name) in names.iter().enumerate() {
        let path = name.strip_suffix('/').unwrap_or(name);
        ensure!(
            !path.contains('\\') && path.split('/').all(|part| !matches!(part, "" | "." | "..")),
            "Wheel contains an invalid path: `{name}`"
        );
        ensure!(
            seen.insert(path),
            "Wheel contains a duplicate path: `{name}`"
        );
        ensure!(
            !signatures
                .iter()
                .any(|signature| path.eq_ignore_ascii_case(signature)),
            "Cannot add a lock to a signed wheel"
        );
        if Path::new(path).starts_with(&lock_path)
            || Path::new(path).starts_with(&record_path)
            || path.eq_ignore_ascii_case(&lock_path)
            || path.eq_ignore_ascii_case(&record_path)
        {
            let entry = &archive.file().entries()[index];
            ensure!(
                path == name
                    && (name == &lock_path || name == &record_path)
                    && entry
                        .unix_permissions()
                        .is_none_or(|mode| matches!(mode & 0o170_000, 0 | 0o100_000)),
                "Wheel contains an invalid path or file type: `{name}`"
            );
            if name == &lock_path {
                continue;
            }
        }

        let mut input = archive.reader_with_entry(index).await?;
        let compression = if input.entry().compression() == Compression::Stored {
            Compression::Stored
        } else {
            Compression::Deflate
        };
        // The writer generates new ZIP offsets, sizes, and extra fields.
        let metadata = ZipEntryBuilder::from(input.entry().clone())
            .compression(compression)
            .extra_fields(Vec::new());
        if name == &record_path {
            let mut contents = Vec::new();
            input.read_to_end_checked(&mut contents).await?;
            record = Some((
                metadata,
                rewrite_record(&contents, &record_path, &lock_path, lock)?,
            ));
        } else {
            let mut output = writer.write_entry_seekable(metadata).await?;
            let size = input.entry().uncompressed_size();
            let mut reader = FuturesBufReader::with_capacity(
                128 * 1024,
                (&mut input).take(size.saturating_add(1)),
            );
            let copied = copy_buf(&mut reader, &mut output).await?;
            ensure!(
                copied == size,
                "Wheel entry size does not match its header: `{name}`"
            );
            ensure!(
                input.compute_hash() == input.entry().crc32(),
                "Wheel entry has an invalid CRC: `{name}`"
            );
            output.close().await?;
        }
    }
    let (metadata, record) = record.context("Wheel has no RECORD file")?;
    // Give the new entry a fixed timestamp so the rewrite is reproducible.
    let entry = ZipEntryBuilder::new(lock_path.into(), Compression::Deflate)
        .last_modification_date(ZipDateTime::default())
        .unix_permissions(0o100_644);
    writer.write_entry_whole(entry, lock).await?;
    writer.write_entry_whole(metadata, &record).await?;
    let mut output = writer.close().await?.into_inner();
    output.shutdown().await?;
    Ok(())
}

fn rewrite_record(
    record: &[u8],
    record_path: &str,
    lock_path: &str,
    lock: &[u8],
) -> Result<Vec<u8>> {
    let mut input = csv::ReaderBuilder::new()
        .has_headers(false)
        .from_reader(record);
    let mut output = csv::Writer::from_writer(Vec::new());
    for row in input.records() {
        let row = row.context("Wheel RECORD is invalid")?;
        ensure!(
            row.len() == 3,
            "Wheel RECORD contains a row without three fields"
        );
        if row.get(0) != Some(lock_path) && row.get(0) != Some(record_path) {
            output.write_record(&row)?;
        }
    }
    let hash = BASE64_URL_SAFE_NO_PAD.encode(Sha256::digest(lock));
    output.write_record([
        lock_path,
        &format!("sha256={hash}"),
        &lock.len().to_string(),
    ])?;
    output.write_record([record_path, "", ""])?;
    Ok(output.into_inner()?)
}

#[cfg(test)]
mod tests;
