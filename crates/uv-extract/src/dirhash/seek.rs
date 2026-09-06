//! Directory hashing while extracting seekable ZIP archives.

use std::path::{Path, PathBuf};
use std::pin::pin;
use std::sync::Mutex;

use crate::vendor::CloneableSeekableReader;
use crate::{Error, insecure_no_validate};
use async_zip::StoredZipEntry;
use async_zip::base::read::seek::ZipFileReader;
use async_zip::error::ZipError;
use futures::executor::block_on;
use futures::io::{AllowStdIo, AsyncReadExt, AsyncSeekExt, AsyncWriteExt, SeekFrom};
use rayon::prelude::*;
use rustc_hash::FxHashSet;
use tokio_util::compat::{FuturesAsyncReadCompatExt, FuturesAsyncWriteCompatExt};
use tracing::warn;
use uv_configuration::initialize_rayon_once;

use super::{
    DirhashTree, HashedFile, UnhashedFile, UnzipOutput, blake3_copy, directory_tree_from_extracted,
};
use crate::archive_path::SanitizedArchivePath;

/// A successfully extracted file, or an explicit directory that can affect the digest.
enum ExtractedEntry {
    File {
        path: SanitizedArchivePath,
        size: u64,
        digest: Option<blake3::Hash>,
        executable: bool,
    },
    Directory(SanitizedArchivePath),
}

/// Unzip a `.zip` archive into the target directory.
pub(crate) fn unzip(reader: fs_err::File, target: &Path) -> Result<Vec<UnhashedFile>, Error> {
    let UnzipOutput::Unhashed(files) = unzip_inner(reader, target, false)? else {
        return Err(Error::Io(std::io::Error::other(
            "seekable ZIP hash tree was unexpectedly computed",
        )));
    };
    Ok(files)
}

/// Unzip a `.zip` archive into the target directory while computing a hash tree of the extracted
/// files.
///
/// Returns the list of unpacked files and their sizes, along with a hash tree containing the
/// canonicalized extracted file paths, contents, and empty directories.
pub(crate) fn unzip_and_hash(
    reader: fs_err::File,
    target: &Path,
) -> Result<(Vec<HashedFile>, DirhashTree), Error> {
    let UnzipOutput::Hashed { files, tree } = unzip_inner(reader, target, true)? else {
        return Err(Error::Io(std::io::Error::other(
            "seekable ZIP hash tree was not computed",
        )));
    };
    Ok((files, tree))
}

fn unzip_inner(
    reader: fs_err::File,
    target: &Path,
    hash_contents: bool,
) -> Result<UnzipOutput, Error> {
    let (reader, _) = reader.into_parts();

    // Parse the central directory once, then clone the archive reader per Rayon worker so
    // extraction stays parallel for already-downloaded wheels. AllowStdIo adapts synchronous
    // file I/O to async_zip; extraction itself runs on blocking and Rayon threads.
    let archive = block_on(ZipFileReader::new(AllowStdIo::new(
        CloneableSeekableReader::new(reader),
    )))?;
    let skip_validation = insecure_no_validate();
    if !skip_validation {
        validate_archive(&archive)?;
    }
    if hash_contents {
        validate_unique_output_paths(archive.file().entries())?;
    }

    let directories = Mutex::new(FxHashSet::default());
    // Initialize the threadpool with the user settings.
    initialize_rayon_once();
    let extract = |file_number| {
        let mut archive = archive.clone();
        extract_entry(
            &mut archive,
            file_number,
            target,
            &directories,
            skip_validation,
            hash_contents,
        )
    };

    if !hash_contents {
        let files = (0..archive.file().entries().len())
            .into_par_iter()
            .map(extract)
            .filter_map(|result| match result {
                Ok(Some(ExtractedEntry::File { path, size, .. })) => {
                    Some(Ok(UnhashedFile::new(path.into_path_buf(), size)))
                }
                Ok(Some(ExtractedEntry::Directory(_)) | None) => None,
                Err(err) => Some(Err(err)),
            })
            .collect::<Result<_, Error>>()?;
        return Ok(UnzipOutput::Unhashed(files));
    }

    let extracted = (0..archive.file().entries().len())
        .into_par_iter()
        .map(extract)
        // Filter out skipped dangerous paths, then collect files and directory candidates.
        .filter_map(Result::transpose)
        .collect::<Result<Vec<_>, Error>>()?;

    let mut hashed_files = Vec::with_capacity(extracted.len());
    let mut digest_directories = FxHashSet::default();
    for extracted in extracted {
        match extracted {
            ExtractedEntry::File {
                path,
                size,
                digest,
                executable,
            } => {
                if let Some(digest) = digest {
                    hashed_files.push(HashedFile::new(path, size, digest, executable));
                }
            }
            ExtractedEntry::Directory(path) => {
                digest_directories.insert(path);
            }
        }
    }
    let tree = directory_tree_from_extracted(&hashed_files, &digest_directories)?;
    Ok(UnzipOutput::Hashed {
        files: hashed_files,
        tree,
    })
}

/// Validate the end-of-central-directory record and any trailing contents.
fn validate_archive<R>(archive: &ZipFileReader<AllowStdIo<R>>) -> Result<(), Error>
where
    R: std::io::BufRead + std::io::Seek + Clone + Unpin,
{
    // Reject comments that appear to contain an embedded ZIP file, as in the streaming extractor.
    let comment = archive.file().comment().as_bytes();
    if comment.iter().any(|&byte| (1..=8).contains(&byte)) {
        return Err(Error::ZipInZip);
    }

    // The seekable reader searches backwards for the end-of-central-directory record and otherwise
    // ignores bytes following it. Find the first structurally valid record so an appended
    // central-directory chain cannot hide trailing contents, while ignoring record signatures that
    // happen to occur in a comment.
    let mut scan_archive = archive.clone();
    let mut validation_archive = archive.clone();
    block_on(async {
        const EOCD_SIGNATURE: &[u8; 4] = b"PK\x05\x06";
        const EOCD_LENGTH: usize = 22;
        const CHUNK_LENGTH: usize = 64 * 1024;

        fn u16_at(bytes: &[u8], offset: usize) -> u16 {
            u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
        }

        fn u32_at(bytes: &[u8], offset: usize) -> u32 {
            u32::from_le_bytes([
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
            ])
        }

        fn u64_at(bytes: &[u8], offset: usize) -> u64 {
            u64::from_le_bytes([
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
                bytes[offset + 4],
                bytes[offset + 5],
                bytes[offset + 6],
                bytes[offset + 7],
            ])
        }

        let scan_reader = scan_archive.inner_mut();
        let validation_reader = validation_archive.inner_mut();
        let length = scan_reader
            .seek(SeekFrom::End(0))
            .await
            .map_err(Error::Io)?;
        scan_reader
            .seek(SeekFrom::Start(0))
            .await
            .map_err(Error::Io)?;

        // Search the complete archive in bounded chunks. Limiting this search to the maximum
        // comment length lets padding hide an earlier, independently valid EOCD.
        let mut buffer = vec![0; CHUNK_LENGTH + EOCD_LENGTH - 1];
        let mut carry = 0;
        let mut start = 0_u64;
        let mut record = None;
        'scan: loop {
            let read = scan_reader
                .read(&mut buffer[carry..])
                .await
                .map_err(Error::Io)?;
            let available = carry + read;

            for offset in 0..available.saturating_sub(EOCD_LENGTH - 1) {
                let candidate = &buffer[offset..offset + EOCD_LENGTH];
                if &candidate[..EOCD_SIGNATURE.len()] != EOCD_SIGNATURE {
                    continue;
                }

                let Some(record_offset) = u64::try_from(offset)
                    .ok()
                    .and_then(|offset| start.checked_add(offset))
                else {
                    continue;
                };
                let central_directory_size = u32_at(candidate, 12);
                let central_directory_offset = u32_at(candidate, 16);

                let standard_directory_end = u64::from(central_directory_offset)
                    .checked_add(u64::from(central_directory_size));
                let valid = if standard_directory_end == Some(record_offset) {
                    true
                } else {
                    const ZIP64_EOCD_SIGNATURE: &[u8; 4] = b"PK\x06\x06";
                    const ZIP64_LOCATOR_SIGNATURE: &[u8; 4] = b"PK\x06\x07";

                    let Some(locator_offset) = record_offset.checked_sub(20) else {
                        continue;
                    };
                    validation_reader
                        .seek(SeekFrom::Start(locator_offset))
                        .await
                        .map_err(Error::Io)?;
                    let mut locator = [0; 20];
                    validation_reader
                        .read_exact(&mut locator)
                        .await
                        .map_err(Error::Io)?;
                    if &locator[..ZIP64_LOCATOR_SIGNATURE.len()] != ZIP64_LOCATOR_SIGNATURE {
                        continue;
                    }

                    let zip64_offset = u64_at(&locator, 8);
                    if zip64_offset
                        .checked_add(56)
                        .is_none_or(|record_end| record_end > length)
                    {
                        continue;
                    }
                    validation_reader
                        .seek(SeekFrom::Start(zip64_offset))
                        .await
                        .map_err(Error::Io)?;
                    let mut zip64_record = [0; 56];
                    validation_reader
                        .read_exact(&mut zip64_record)
                        .await
                        .map_err(Error::Io)?;
                    if &zip64_record[..ZIP64_EOCD_SIGNATURE.len()] != ZIP64_EOCD_SIGNATURE {
                        continue;
                    }

                    let zip64_size = u64_at(&zip64_record, 4);
                    let directory_size = u64_at(&zip64_record, 40);
                    let directory_offset = u64_at(&zip64_record, 48);

                    directory_offset.checked_add(directory_size) == Some(zip64_offset)
                        && zip64_offset
                            .checked_add(12)
                            .and_then(|offset| offset.checked_add(zip64_size))
                            == Some(locator_offset)
                };
                if !valid {
                    continue;
                }

                let comment_length = u16_at(candidate, 20);
                let Some(end) = record_offset
                    .checked_add(22)
                    .and_then(|offset| offset.checked_add(u64::from(comment_length)))
                else {
                    continue;
                };
                if end <= length {
                    record = Some((record_offset, end));
                    break 'scan;
                }
            }

            if read == 0 {
                break;
            }
            carry = available.min(EOCD_LENGTH - 1);
            buffer.copy_within(available - carry..available, 0);
            let advanced =
                u64::try_from(available - carry).map_err(|_| ZipError::InvalidEntryDataRange)?;
            start = start
                .checked_add(advanced)
                .ok_or(ZipError::InvalidEntryDataRange)?;
        }

        let Some((record_offset, end)) = record else {
            return Err(Error::TrailingContents);
        };
        let comment_length = usize::try_from(end - record_offset - 22)
            .map_err(|_| ZipError::InvalidEntryDataRange)?;
        let mut comment = vec![0; comment_length];
        validation_reader
            .seek(SeekFrom::Start(record_offset + 22))
            .await
            .map_err(Error::Io)?;
        validation_reader
            .read_exact(&mut comment)
            .await
            .map_err(Error::Io)?;
        if comment.iter().any(|&byte| (1..=8).contains(&byte)) {
            return Err(Error::ZipInZip);
        }

        validation_reader
            .seek(SeekFrom::Start(end))
            .await
            .map_err(Error::Io)?;
        let mut has_trailing = false;
        loop {
            let read = validation_reader
                .read(&mut buffer[..CHUNK_LENGTH])
                .await
                .map_err(Error::Io)?;
            if read == 0 {
                break;
            }
            if buffer[..read].iter().any(|&byte| byte != 0) {
                return Err(Error::TrailingContents);
            }
            has_trailing = true;
        }
        if has_trailing {
            warn!("Ignoring trailing null bytes in ZIP archive");
        }

        Ok::<(), Error>(())
    })
}

/// Reject entries that would write to the same sanitized output path.
///
/// Duplicate paths can otherwise race to determine which contents are persisted or hashed.
fn validate_unique_output_paths(entries: &[StoredZipEntry]) -> Result<(), Error> {
    let mut paths = FxHashSet::default();
    for (file_number, entry) in entries.iter().enumerate() {
        let file_name = entry_file_name(entry, file_number)?;
        let Ok(Some(path)) = SanitizedArchivePath::from_archive_member(file_name) else {
            continue;
        };
        if !paths.insert(path.clone()) {
            return Err(Error::DuplicateOutputPath {
                path: path.into_path_buf(),
            });
        }
    }
    Ok(())
}

/// Extract a single central-directory entry from a seekable ZIP archive.
fn extract_entry<R>(
    archive: &mut ZipFileReader<AllowStdIo<R>>,
    file_number: usize,
    target: &Path,
    directories: &Mutex<FxHashSet<PathBuf>>,
    skip_validation: bool,
    hash_contents: bool,
) -> Result<Option<ExtractedEntry>, Error>
where
    R: std::io::BufRead + std::io::Seek + Unpin,
{
    let entry = archive.file().entries()[file_number].clone();
    let file_name = entry_file_name(&entry, file_number)?;
    let enclosed_name = match SanitizedArchivePath::from_archive_member(file_name) {
        Ok(path) => path,
        Err(_) if skip_validation => None,
        Err(err) => return Err(err),
    };
    let Some(enclosed_name) = enclosed_name else {
        warn!("Skipping unsafe file name: {file_name}");
        return Ok(None);
    };
    if !skip_validation {
        validate_local_crc32(archive, &entry, enclosed_name.as_path())?;
    }

    let path = target.join(enclosed_name.as_path());
    if entry.dir()? {
        create_directory_once(directories, &path)?;
        if hash_contents {
            validate_directory_entry(&entry, enclosed_name.as_path(), skip_validation)?;
        }
        return Ok(Some(ExtractedEntry::Directory(enclosed_name)));
    }

    if let Some(parent) = path.parent() {
        create_directory_once(directories, parent)?;
    }

    extract_file_entry(
        archive,
        &entry,
        file_number,
        enclosed_name,
        &path,
        skip_validation,
        hash_contents,
    )
    .map(Some)
}

/// Return an entry file name from the central directory.
fn entry_file_name(entry: &StoredZipEntry, file_number: usize) -> Result<&str, Error> {
    match entry.filename().as_str() {
        Ok(file_name) => Ok(file_name),
        Err(ZipError::StringNotUtf8) => Err(Error::CentralDirectoryEntryNotUtf8 {
            index: file_number as u64,
        }),
        Err(err) => Err(err.into()),
    }
}

/// Validate that the local header or data descriptor CRC agrees with the central directory.
fn validate_local_crc32<R>(
    archive: &mut ZipFileReader<AllowStdIo<R>>,
    entry: &StoredZipEntry,
    path: &Path,
) -> Result<(), Error>
where
    R: std::io::BufRead + std::io::Seek + Unpin,
{
    let local_crc32 = block_on(async {
        let reader = archive.inner_mut();
        if entry.data_descriptor() {
            reader
                .seek(SeekFrom::Start(
                    entry
                        .file_offset()
                        .checked_add(26)
                        .ok_or(ZipError::InvalidEntryDataRange)?,
                ))
                .await
                .map_err(Error::Io)?;
            let mut lengths = [0; 4];
            reader.read_exact(&mut lengths).await.map_err(Error::Io)?;
            let filename_length = u16::from_le_bytes([lengths[0], lengths[1]]);
            let extra_length = u16::from_le_bytes([lengths[2], lengths[3]]);
            let descriptor_offset = entry
                .file_offset()
                .checked_add(30)
                .and_then(|offset| offset.checked_add(u64::from(filename_length)))
                .and_then(|offset| offset.checked_add(u64::from(extra_length)))
                .and_then(|offset| offset.checked_add(entry.compressed_size()))
                .ok_or(ZipError::InvalidEntryDataRange)?;
            reader
                .seek(SeekFrom::Start(descriptor_offset))
                .await
                .map_err(Error::Io)?;
            let mut checksum = [0; 4];
            reader.read_exact(&mut checksum).await.map_err(Error::Io)?;
            if checksum == *b"PK\x07\x08" {
                reader.read_exact(&mut checksum).await.map_err(Error::Io)?;
            }
            Ok::<_, Error>(u32::from_le_bytes(checksum))
        } else {
            reader
                .seek(SeekFrom::Start(
                    entry
                        .file_offset()
                        .checked_add(14)
                        .ok_or(ZipError::InvalidEntryDataRange)?,
                ))
                .await
                .map_err(Error::Io)?;
            let mut checksum = [0; 4];
            reader.read_exact(&mut checksum).await.map_err(Error::Io)?;
            Ok::<_, Error>(u32::from_le_bytes(checksum))
        }
    })?;

    if local_crc32 != entry.crc32() {
        return Err(Error::ConflictingChecksums {
            path: path.to_path_buf(),
            offset: entry.file_offset(),
            local_crc32,
            central_directory_crc32: entry.crc32(),
        });
    }

    Ok(())
}

/// Create a directory once across parallel extraction workers.
fn create_directory_once(
    directories: &Mutex<FxHashSet<PathBuf>>,
    path: &Path,
) -> Result<(), Error> {
    let mut directories = directories.lock().map_err(|_| directory_lock_error())?;
    if directories.insert(path.to_path_buf()) {
        fs_err::create_dir_all(path).map_err(Error::Io)?;
    }

    Ok(())
}

/// Validate the metadata for a directory entry.
fn validate_directory_entry(
    entry: &StoredZipEntry,
    path: &Path,
    skip_validation: bool,
) -> Result<(), Error> {
    if skip_validation {
        return Ok(());
    }

    if entry.crc32() != 0 {
        return Err(Error::BadCrc32 {
            path: path.to_path_buf(),
            computed: 0,
            expected: entry.crc32(),
        });
    }

    if entry.uncompressed_size() != 0 {
        return Err(Error::BadUncompressedSize {
            path: path.to_path_buf(),
            computed: 0,
            expected: entry.uncompressed_size(),
        });
    }

    Ok(())
}

/// Extract a regular file entry and return its digest metadata.
fn extract_file_entry<R>(
    archive: &mut ZipFileReader<AllowStdIo<R>>,
    entry: &StoredZipEntry,
    file_number: usize,
    enclosed_name: SanitizedArchivePath,
    path: &Path,
    skip_validation: bool,
    hash_contents: bool,
) -> Result<ExtractedEntry, Error>
where
    R: std::io::BufRead + std::io::Seek + Unpin,
{
    let outfile = if hash_contents {
        fs_err::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
    } else {
        fs_err::File::create(path)
    }
    .map_err(Error::Io)?;
    let size = entry.uncompressed_size();
    let writer = buffered_file_writer(outfile, size);

    // Keep the hashing state out of ordinary extraction, and pin both futures here to avoid
    // moving their large state into `block_on`.
    let (copied, computed_crc32, digest) = if hash_contents {
        let (copied, computed_crc32, digest) =
            block_on(pin!(copy_and_hash_entry(archive, file_number, writer)))?;
        (copied, computed_crc32, Some(digest))
    } else {
        let (copied, computed_crc32) = block_on(pin!(copy_entry(archive, file_number, writer)))?;
        (copied, computed_crc32, None)
    };
    validate_file_entry(
        enclosed_name.as_path(),
        copied,
        size,
        computed_crc32,
        entry.crc32(),
        skip_validation,
    )?;
    #[cfg(unix)]
    preserve_executable_bit(path, entry.unix_permissions())?;

    Ok(ExtractedEntry::File {
        path: enclosed_name,
        size,
        digest,
        executable: entry
            .unix_permissions()
            .is_some_and(|mode| mode & 0o111 != 0),
    })
}

/// Build a buffered writer sized for the expected entry contents.
fn buffered_file_writer(file: fs_err::File, size: u64) -> std::io::BufWriter<fs_err::File> {
    if let Ok(size) = usize::try_from(size) {
        std::io::BufWriter::with_capacity(std::cmp::min(size, 1024 * 1024), file)
    } else {
        std::io::BufWriter::new(file)
    }
}

/// Validate the copied size and CRC for a file entry.
fn validate_file_entry(
    path: &Path,
    copied: u64,
    expected_size: u64,
    computed_crc32: u32,
    expected_crc32: u32,
    skip_validation: bool,
) -> Result<(), Error> {
    if skip_validation {
        return Ok(());
    }

    if copied != expected_size {
        return Err(Error::BadUncompressedSize {
            path: path.to_path_buf(),
            computed: copied,
            expected: expected_size,
        });
    }

    if computed_crc32 != expected_crc32 {
        return Err(Error::BadCrc32 {
            path: path.to_path_buf(),
            computed: computed_crc32,
            expected: expected_crc32,
        });
    }

    Ok(())
}

#[cfg(unix)]
/// Preserve executable permissions according to pip's wheel extraction behavior.
fn preserve_executable_bit(path: &Path, unix_permissions: Option<u16>) -> Result<(), Error> {
    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt;

    let Some(mode) = unix_permissions else {
        return Ok(());
    };

    // https://github.com/pypa/pip/blob/3898741e29b7279e7bffe044ecfbe20f6a438b1e/src/pip/_internal/utils/unpacking.py#L88-L100
    if mode & 0o111 == 0 {
        return Ok(());
    }

    let permissions = fs_err::metadata(path).map_err(Error::Io)?.permissions();
    if permissions.mode() & 0o111 == 0o111 {
        return Ok(());
    }

    fs_err::set_permissions(path, Permissions::from_mode(permissions.mode() | 0o111))
        .map_err(Error::Io)
}

/// Return an error for a poisoned directory memoization lock.
fn directory_lock_error() -> Error {
    Error::Io(std::io::Error::other("directory set lock poisoned"))
}

/// Copy an entry without computing a content digest.
async fn copy_entry<R>(
    archive: &mut ZipFileReader<AllowStdIo<R>>,
    file_number: usize,
    writer: std::io::BufWriter<fs_err::File>,
) -> Result<(u64, u32), Error>
where
    R: std::io::BufRead + std::io::Seek + Unpin,
{
    let mut file = archive.reader_with_entry(file_number).await?;
    let mut writer = AllowStdIo::new(writer);

    let mut copied = 0;
    let mut buffer = vec![0; 128 * 1024];
    loop {
        let read = file.read(&mut buffer).await.map_err(Error::io_or_zip)?;
        if read == 0 {
            break;
        }
        writer.write_all(&buffer[..read]).await.map_err(Error::Io)?;
        copied += read as u64;
    }
    writer.flush().await.map_err(Error::Io)?;
    Ok((copied, file.compute_hash()))
}

/// Copy an entry while hashing the same uncompressed bytes written to disk.
async fn copy_and_hash_entry<R>(
    archive: &mut ZipFileReader<AllowStdIo<R>>,
    file_number: usize,
    writer: std::io::BufWriter<fs_err::File>,
) -> Result<(u64, u32, blake3::Hash), Error>
where
    R: std::io::BufRead + std::io::Seek + Unpin,
{
    let mut file = archive.reader_with_entry(file_number).await?;
    let mut writer = AllowStdIo::new(writer);
    let (copied, digest) = blake3_copy((&mut file).compat(), (&mut writer).compat_write())
        .await
        .map_err(Error::io_or_zip)?;
    Ok((copied, file.compute_hash(), digest))
}
