use std::collections::HashMap;
use std::fs::Permissions;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use data_encoding::BASE64URL_NOPAD;
use fs_err::File;
use sha2::{Digest, Sha256, Sha384, Sha512};

use uv_fs::{normalize_path_under, persist_with_retry_sync};

use crate::script::{EntryPoints, Script};
use crate::wheel::{
    ValidatedScript, copy_and_hash, format_shebang, get_script_executable, render_script_launcher,
};
use crate::{Error, Layout, RecordEntry, read_record};

#[cfg(test)]
mod tests;

/// A generated file already finalized by another environment component.
///
/// Matching SHA2 RECORD entries can be advanced without changing the file again. Nonmatching
/// provider entries, absent hashes, and unrecognized hash algorithms are retained.
pub struct RecordUpdate<'a> {
    pub path: &'a Path,
    pub before: &'a [u8],
    pub after: &'a [u8],
}

/// Finalize generated scripts in a private, fully prepared environment before moving it.
///
/// Builds and bytecode compilation must finish before this operation: the physical interpreter
/// remains necessary while preparing the environment. Only recorded scripts are considered;
/// modified entry-point launchers, unrecognized scripts and filesystem links are retained.
pub fn finalize_scripts(
    layout: &Layout,
    final_executable: &Path,
    dist_info: &Path,
    record_updates: &[RecordUpdate<'_>],
) -> Result<(), Error> {
    let site_packages = dist_info.parent().ok_or_else(|| {
        Error::BrokenVenv(format!(
            "Distribution metadata has no parent: {}",
            dist_info.display()
        ))
    })?;
    if !fs_err::symlink_metadata(dist_info)?.is_dir() {
        return Err(Error::BrokenVenv(format!(
            "Distribution metadata is not a directory: {}",
            dist_info.display()
        )));
    }
    let record_path = dist_info.join("RECORD");
    let mut record = read_record(File::open(&record_path)?)?;
    let entrypoints = EntryPoints::read(
        dist_info.join("entry_points.txt"),
        None,
        layout.python_version.1,
    )?;
    let mut launchers = HashMap::new();
    for (scripts, is_gui) in [
        (&entrypoints.console_scripts, false),
        (&entrypoints.gui_scripts, true),
    ] {
        for script in scripts {
            let validated = ValidatedScript::try_from_script(script, layout)?;
            launchers.insert(validated.as_path().to_path_buf(), (script, is_gui));
        }
    }
    let prefixes = [false, true].map(|is_gui| {
        let (physical, final_path) = script_executables(layout, final_executable, is_gui);
        let newline = if layout.os_name == "nt" { "\r\n" } else { "\n" };
        (
            format!(
                "{}{newline}",
                format_shebang(physical, &layout.os_name, false)
            ),
            format!(
                "{}{newline}",
                format_shebang(final_path, &layout.os_name, false)
            ),
        )
    });
    let mut updates = HashMap::new();
    for entry in &record {
        let Some(path) = recorded_script_path(site_packages, entry, &layout.scheme.scripts) else {
            continue;
        };
        if updates.contains_key(&path) || !plain_script(&path, &layout.scheme.scripts)? {
            continue;
        }
        let digest = if let Some(update) = record_updates.iter().find(|update| update.path == path)
        {
            reconcile_record_update(&path, entry, update)?
        } else if let Some((script, is_gui)) = launchers.get(&path) {
            finalize_launcher(layout, final_executable, script, *is_gui, &path)?
        } else {
            finalize_prefix(&path, entry, &prefixes)?
        };
        if let Some(digest) = digest {
            updates.insert(path, digest);
        }
    }
    let mut changed = false;
    for entry in &mut record {
        let Some(path) = recorded_script_path(site_packages, entry, &layout.scheme.scripts) else {
            continue;
        };
        if let Some(digest) = updates.get(&path)
            && !digest.matches(entry)
        {
            entry.hash = Some(digest.hash.clone());
            entry.size = Some(digest.size);
            changed = true;
        }
    }
    if changed {
        // RECORD may be shared with another environment or the wheel cache. Replace this entry.
        let permissions = fs_err::metadata(&record_path)?.permissions();
        let mut temporary = uv_fs::tempfile_in(dist_info)?;
        temporary.as_file().set_permissions(permissions)?;
        {
            let mut writer = csv::WriterBuilder::new()
                .has_headers(false)
                .escape(b'"')
                .from_writer(&mut temporary);
            record.sort();
            for entry in record {
                writer.serialize(entry)?;
            }
            writer.flush()?;
        }
        persist_with_retry_sync(temporary, &record_path)?;
    }
    Ok(())
}

fn reconcile_record_update(
    path: &Path,
    entry: &RecordEntry,
    update: &RecordUpdate<'_>,
) -> Result<Option<FileDigest>, Error> {
    if fs_err::metadata(path)?.len() != update.after.len() as u64
        || fs_err::read(path)? != update.after
        || (!recorded_contents_match(entry, update.before)
            && !recorded_contents_match(entry, update.after))
    {
        return Ok(None);
    }
    let mut finalized = update.after;
    Ok(Some(FileDigest::read(&mut finalized)?))
}

fn recorded_contents_match(entry: &RecordEntry, contents: &[u8]) -> bool {
    if entry.size.is_some_and(|size| size != contents.len() as u64) {
        return false;
    }
    let Some((algorithm, expected)) = entry.hash.as_deref().and_then(|hash| hash.split_once('='))
    else {
        return false;
    };
    let digest = match algorithm {
        "sha256" => BASE64URL_NOPAD.encode(&Sha256::digest(contents)),
        "sha384" => BASE64URL_NOPAD.encode(&Sha384::digest(contents)),
        "sha512" => BASE64URL_NOPAD.encode(&Sha512::digest(contents)),
        _ => return false,
    };
    digest == expected
}

fn recorded_script_path(
    site_packages: &Path,
    entry: &RecordEntry,
    scripts: &Path,
) -> Option<PathBuf> {
    normalize_path_under(site_packages.join(&entry.path), scripts)
}

/// Avoid following package-provided aliases when updating generated files.
fn plain_script(path: &Path, scripts: &Path) -> io::Result<bool> {
    let mut parent = path.parent();
    while let Some(directory) = parent {
        if !directory.starts_with(scripts) {
            return Ok(false);
        }
        match fs_err::symlink_metadata(directory) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => return Ok(false),
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(err) => return Err(err),
        }
        if directory == scripts {
            break;
        }
        parent = directory.parent();
    }
    match fs_err::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.is_file()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err),
    }
}

fn script_executables(
    layout: &Layout,
    final_executable: &Path,
    is_gui: bool,
) -> (PathBuf, PathBuf) {
    let physical = get_script_executable(&layout.sys_executable, is_gui);
    // The final environment may still contain a different interpreter. Select pythonw from the
    // prepared environment, then retain that filename at the final destination.
    let final_path = if physical == layout.sys_executable {
        final_executable.to_path_buf()
    } else if let Some(name) = physical.file_name() {
        final_executable.with_file_name(name)
    } else {
        final_executable.to_path_buf()
    };
    (physical, final_path)
}

struct FileDigest {
    size: u64,
    hash: String,
}

impl FileDigest {
    fn read(reader: &mut impl Read) -> io::Result<Self> {
        let (size, hash) = copy_and_hash(reader, &mut io::sink())?;
        Ok(Self { size, hash })
    }

    fn matches(&self, entry: &RecordEntry) -> bool {
        entry.size == Some(self.size) && entry.hash.as_deref() == Some(self.hash.as_str())
    }
}

fn replace_script(
    path: &Path,
    mut reader: impl Read,
    permissions: Permissions,
) -> Result<FileDigest, Error> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::BrokenVenv(format!("Script has no parent: {}", path.display())))?;
    let mut temporary = uv_fs::tempfile_in(parent)?;
    temporary.as_file().set_permissions(permissions)?;
    let (size, hash) = copy_and_hash(&mut reader, &mut temporary)?;
    // Windows cannot replace the destination while the streamed input still holds it open.
    drop(reader);
    persist_with_retry_sync(temporary, path)?;
    Ok(FileDigest { size, hash })
}

fn finalize_launcher(
    layout: &Layout,
    final_executable: &Path,
    script: &Script,
    is_gui: bool,
    path: &Path,
) -> Result<Option<FileDigest>, Error> {
    let (physical, final_path) = script_executables(layout, final_executable, is_gui);
    let old = render_script_launcher(script, &physical, &layout.os_name, false, is_gui)?;
    let new = render_script_launcher(script, &final_path, &layout.os_name, false, is_gui)?;
    let metadata = fs_err::metadata(path)?;
    if metadata.len() != old.len() as u64 && metadata.len() != new.len() as u64 {
        return Ok(None);
    }
    let contents = fs_err::read(path)?;
    if contents == new {
        // Identical providers can share a launcher already finalized by another distribution.
        return Ok(Some(FileDigest::read(&mut new.as_slice())?));
    }
    if contents != old {
        return Ok(None);
    }
    Ok(Some(replace_script(
        path,
        new.as_slice(),
        metadata.permissions(),
    )?))
}

fn finalize_prefix(
    path: &Path,
    entry: &RecordEntry,
    prefixes: &[(String, String)],
) -> Result<Option<FileDigest>, Error> {
    // Rewritten wheel placeholders always have a SHA256 digest and size. They also establish
    // ownership when distributions install different scripts under the same name.
    if entry.size.is_none()
        || !entry
            .hash
            .as_deref()
            .is_some_and(|hash| hash.starts_with("sha256="))
    {
        return Ok(None);
    }
    let maximum = prefixes
        .iter()
        .flat_map(|(old, new)| [old.len(), new.len()])
        .max()
        .unwrap_or(0);
    let mut file = File::open(path)?;
    let mut head = Vec::with_capacity(maximum);
    file.by_ref().take(maximum as u64).read_to_end(&mut head)?;
    for (old, new) in prefixes {
        if head.starts_with(old.as_bytes()) {
            file.rewind()?;
            if !FileDigest::read(&mut file)?.matches(entry) {
                return Ok(None);
            }
            file.seek(SeekFrom::Start(old.len() as u64))?;
            let permissions = file.metadata()?.permissions();
            return Ok(Some(replace_script(
                path,
                new.as_bytes().chain(file),
                permissions,
            )?));
        }
        if head.starts_with(new.as_bytes()) {
            file.rewind()?;
            let current = FileDigest::read(&mut file)?;
            if current.matches(entry) {
                return Ok(Some(current));
            }
            // A shared identical script may already use the final prefix. Verify this
            // distribution's original digest before bringing its RECORD up to date.
            file.seek(SeekFrom::Start(new.len() as u64))?;
            if FileDigest::read(&mut old.as_bytes().chain(file))?.matches(entry) {
                return Ok(Some(current));
            }
            return Ok(None);
        }
    }
    Ok(None)
}
