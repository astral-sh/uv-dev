use std::fs::Permissions;
use std::io::{self, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::Error;

const DIRECTORY: &str = ".uv-metadata";
const JOURNAL: &str = "journal.toml";
const COMMITTED: &str = "committed.toml";
const RECEIPT: &str = "uv-receipt.toml";
const LOCK: &str = "uv.lock";

/// Serialized tool metadata, ready to publish without further serialization.
pub struct ToolMetadata {
    receipt: String,
    lock: Option<String>,
}

impl ToolMetadata {
    pub(crate) fn new(receipt: String, lock: Option<String>) -> Self {
        Self { receipt, lock }
    }

    /// The exact receipt bytes that will be published.
    fn receipt_bytes(&self) -> &[u8] {
        self.receipt.as_bytes()
    }

    /// The exact lock bytes that will be published, or `None` to remove the lock.
    fn lock_bytes(&self) -> Option<&[u8]> {
        self.lock.as_deref().map(str::as_bytes)
    }
}

/// The presence of this record means both files must be restored from the saved pair.
/// Backups retain raw bytes, including legacy receipts and invalid locks.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u8,
    receipt: bool,
    lock: bool,
}

struct Snapshot {
    contents: Vec<u8>,
    permissions: Permissions,
}

impl Snapshot {
    fn read(path: &Path) -> io::Result<Self> {
        let mut file = fs_err::File::open(path)?;
        let permissions = file.metadata()?.permissions();
        let mut contents = Vec::new();
        file.read_to_end(&mut contents)?;
        Ok(Self {
            contents,
            permissions,
        })
    }

    fn read_optional(path: &Path) -> io::Result<Option<Self>> {
        match Self::read(path) {
            Ok(snapshot) => Ok(Some(snapshot)),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err),
        }
    }

    fn backup(&self, path: &Path) -> io::Result<()> {
        fs_err::write(path, &self.contents)?;
        fs_err::set_permissions(path, self.permissions.clone())
    }

    fn restore(&self, path: &Path) -> io::Result<()> {
        // Recovery can resume after either file was restored, including a read-only receipt.
        if let Some(current) = Self::read_optional(path)?
            && current.contents == self.contents
            && current.permissions == self.permissions
        {
            return Ok(());
        }
        write_file(path, &self.contents, Some(&self.permissions))
    }
}

#[derive(Debug)]
struct Transaction {
    directory: PathBuf,
    journal: PathBuf,
}

impl Transaction {
    fn begin(directory: &Path) -> io::Result<Self> {
        recover(directory)?;
        let journal = directory.join(DIRECTORY);
        if journal.try_exists()? {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "Tool metadata journal `{}` still needs cleanup",
                    journal.display()
                ),
            ));
        }
        let receipt = Snapshot::read_optional(&directory.join(RECEIPT))?;
        let lock = Snapshot::read_optional(&directory.join(LOCK))?;
        let record = Journal {
            version: 1,
            receipt: receipt.is_some(),
            lock: lock.is_some(),
        };
        let record = toml::to_string(&record).map_err(io::Error::other)?;
        let mut builder = tempfile::Builder::new();
        builder.prefix(".uv-metadata-");
        // Restrict traversal at creation, before a backup can contain the previous metadata.
        #[cfg(unix)]
        builder.permissions(Permissions::from_mode(0o700));
        let temporary = builder.tempdir_in(directory)?;
        if let Some(receipt) = receipt {
            receipt.backup(&temporary.path().join(RECEIPT))?;
        }
        if let Some(lock) = lock {
            lock.backup(&temporary.path().join(LOCK))?;
        }
        fs_err::write(temporary.path().join(JOURNAL), record)?;

        // Publish the complete backup before changing either live file. The journal survives
        // process interruption; it is not a guarantee against power loss or filesystem corruption.
        fs_err::rename(temporary.path(), &journal)?;
        Ok(Self {
            directory: directory.to_path_buf(),
            journal,
        })
    }

    fn commit(self, publish: impl FnOnce() -> io::Result<()>) -> Result<(), Error> {
        if let Err(operation) = publish()
            .and_then(|()| fs_err::rename(self.journal.join(JOURNAL), self.journal.join(COMMITTED)))
        {
            return match recover(&self.directory) {
                Ok(()) => Err(operation.into()),
                Err(recovery) => Err(Error::MetadataRecovery {
                    directory: self.directory,
                    operation: Box::new(operation),
                    recovery: Box::new(recovery),
                }),
            };
        }
        // Renaming the record commits the pair. Retain that ownership record until cleanup has
        // removed the backups, so an interrupted cleanup remains distinguishable from user data.
        cleanup_committed(&self.journal);
        Ok(())
    }
}

pub(crate) fn commit(directory: &Path, metadata: &ToolMetadata) -> Result<(), Error> {
    Transaction::begin(directory)?.commit(|| {
        write_optional(&directory.join(LOCK), metadata.lock_bytes())?;
        write_optional(&directory.join(RECEIPT), Some(metadata.receipt_bytes()))
    })
}

/// Recover under the tool-root lock, before any reader consumes either metadata file.
pub(crate) fn recover(directory: &Path) -> io::Result<()> {
    let journal = directory.join(DIRECTORY);
    let Some(record) = read_optional(&journal.join(JOURNAL))? else {
        if let Some(record) = read_optional(&journal.join(COMMITTED))? {
            parse_record(&record)?;
            cleanup_committed(&journal);
            return Ok(());
        }
        // A reserved name alone does not establish ownership of a nonempty directory.
        return remove_empty_journal(&journal);
    };
    let record = parse_record(&record)?;
    if journal.join(COMMITTED).try_exists()? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Conflicting tool metadata journal records",
        ));
    }
    // Read both backups before mutation. A missing required backup is an error, not evidence that
    // the corresponding live file should be removed.
    let receipt = record
        .receipt
        .then(|| Snapshot::read(&journal.join(RECEIPT)))
        .transpose()?;
    let lock = record
        .lock
        .then(|| Snapshot::read(&journal.join(LOCK)))
        .transpose()?;
    restore_optional(&directory.join(LOCK), lock.as_ref())?;
    restore_optional(&directory.join(RECEIPT), receipt.as_ref())?;
    fs_err::rename(journal.join(JOURNAL), journal.join(COMMITTED))?;
    cleanup_committed(&journal);
    Ok(())
}

fn parse_record(record: &[u8]) -> io::Result<Journal> {
    let record = std::str::from_utf8(record).map_err(io::Error::other)?;
    let record: Journal = toml::from_str(record).map_err(io::Error::other)?;
    if record.version != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "Unsupported tool metadata journal version {}",
                record.version
            ),
        ));
    }
    Ok(record)
}

fn read_optional(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs_err::read(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

fn write_optional(path: &Path, contents: Option<&[u8]>) -> io::Result<()> {
    if let Some(contents) = contents {
        let permissions = match fs_err::metadata(path) {
            Ok(metadata) => Some(metadata.permissions()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => None,
            Err(err) => return Err(err),
        };
        write_file(path, contents, permissions.as_ref())
    } else {
        match fs_err::remove_file(path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err),
        }
    }
}

fn restore_optional(path: &Path, snapshot: Option<&Snapshot>) -> io::Result<()> {
    if let Some(snapshot) = snapshot {
        snapshot.restore(path)
    } else {
        write_optional(path, None)
    }
}

fn write_file(path: &Path, contents: &[u8], permissions: Option<&Permissions>) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Tool metadata path has no parent",
        )
    })?;
    let mut temporary = if permissions.is_some() {
        uv_fs::tempfile_in_private(parent)?
    } else {
        uv_fs::tempfile_in(parent)?
    };
    temporary.write_all(contents)?;
    if let Some(permissions) = permissions {
        temporary.as_file().set_permissions(permissions.clone())?;
    }
    // Receipts and locks are uv-owned directory entries. Replace those entries without modifying
    // other names that may link to the previous file, while retaining its access permissions.
    uv_fs::persist_with_retry_sync(temporary, path)
}

fn remove_backup(path: &Path) -> io::Result<()> {
    // Windows refuses to remove a read-only backup even after the pair has committed.
    #[cfg(windows)]
    match fs_err::metadata(path) {
        Ok(metadata) => {
            let mut permissions = metadata.permissions();
            if permissions.readonly() {
                #[expect(
                    clippy::permissions_set_readonly_false,
                    reason = "Windows-only: clear the readonly attribute on an owned, committed backup"
                )]
                permissions.set_readonly(false);
                fs_err::set_permissions(path, permissions)?;
            }
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    }
    write_optional(path, None)
}

fn cleanup(journal: &Path) -> io::Result<()> {
    for entry in fs_err::read_dir(journal)? {
        let name = entry?.file_name();
        if name != RECEIPT && name != LOCK && name != COMMITTED {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Unrecognized contents in tool metadata journal",
            ));
        }
    }
    remove_backup(&journal.join(RECEIPT))?;
    remove_backup(&journal.join(LOCK))?;
    // Delete the ownership record last. Any remaining markerless directory must be empty.
    fs_err::remove_file(journal.join(COMMITTED))?;
    fs_err::remove_dir(journal)
}

fn cleanup_committed(journal: &Path) {
    // The live pair is complete. Leave failed cleanup for a later attempt without blocking readers.
    if let Err(err) = cleanup(journal) {
        warn!("Failed to remove completed tool metadata journal: {err}");
    }
}

fn remove_empty_journal(journal: &Path) -> io::Result<()> {
    let mut entries = match fs_err::read_dir(journal) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    if entries.next().transpose()?.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Unrecognized tool metadata journal without a record",
        ));
    }
    drop(entries);
    if let Err(err) = fs_err::remove_dir(journal) {
        warn!("Failed to remove empty tool metadata journal directory: {err}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use super::{
        COMMITTED, DIRECTORY, JOURNAL, LOCK, RECEIPT, ToolMetadata, Transaction, commit, recover,
        write_optional,
    };
    use crate::Error;

    fn previous_pair() -> io::Result<tempfile::TempDir> {
        let directory = tempfile::tempdir()?;
        // Recovery must retain bytes without reserializing an old receipt or parsing its lock.
        fs_err::write(
            directory.path().join(RECEIPT),
            b"# legacy\n[tool]\nrequirements = []\n",
        )?;
        fs_err::write(directory.path().join(LOCK), b"old lock\xff")?;
        Ok(directory)
    }

    fn assert_previous_pair(directory: &std::path::Path) -> io::Result<()> {
        assert_eq!(
            fs_err::read(directory.join(RECEIPT))?,
            b"# legacy\n[tool]\nrequirements = []\n"
        );
        assert_eq!(fs_err::read(directory.join(LOCK))?, b"old lock\xff");
        assert!(!directory.join(DIRECTORY).exists());
        Ok(())
    }

    #[test]
    fn receipt_failure_restores_replaced_lock() -> Result<(), Error> {
        let directory = previous_pair()?;
        let error = Transaction::begin(directory.path())?
            .commit(|| {
                write_optional(&directory.path().join(LOCK), Some(b"new lock"))?;
                Err(io::Error::other("receipt publication failed"))
            })
            .expect_err("receipt publication fails");
        assert_eq!(
            error.as_io_error().map(io::Error::kind),
            Some(io::ErrorKind::Other)
        );
        assert_previous_pair(directory.path())?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn journal_backups_are_private() -> io::Result<()> {
        let directory = previous_pair()?;
        fs_err::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o755))?;
        let receipt = directory.path().join(RECEIPT);
        fs_err::set_permissions(&receipt, std::fs::Permissions::from_mode(0o600))?;
        let transaction = Transaction::begin(directory.path())?;
        assert_eq!(
            fs_err::metadata(&transaction.journal)?.permissions().mode() & 0o077,
            0
        );
        assert_eq!(
            fs_err::metadata(transaction.journal.join(RECEIPT))?
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        drop(transaction);
        recover(directory.path())?;
        Ok(())
    }

    #[test]
    fn receipt_failure_restores_removed_lock() -> Result<(), Error> {
        let directory = previous_pair()?;
        Transaction::begin(directory.path())?
            .commit(|| {
                write_optional(&directory.path().join(LOCK), None)?;
                Err(io::Error::other("receipt publication failed"))
            })
            .expect_err("receipt publication fails");
        assert_previous_pair(directory.path())?;
        Ok(())
    }

    #[test]
    fn interrupted_lock_publication_recovers() -> io::Result<()> {
        let directory = previous_pair()?;
        let transaction = Transaction::begin(directory.path())?;
        write_optional(&directory.path().join(LOCK), Some(b"new lock"))?;
        drop(transaction);
        recover(directory.path())?;
        assert_previous_pair(directory.path())
    }

    #[test]
    fn interrupted_receipt_publication_recovers() -> io::Result<()> {
        let directory = previous_pair()?;
        let transaction = Transaction::begin(directory.path())?;
        write_optional(&directory.path().join(LOCK), None)?;
        write_optional(&directory.path().join(RECEIPT), Some(b"new receipt"))?;
        drop(transaction);
        recover(directory.path())?;
        assert_previous_pair(directory.path())
    }

    #[test]
    fn fresh_install_interruption_restores_absence() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let transaction = Transaction::begin(directory.path())?;
        write_optional(&directory.path().join(LOCK), Some(b"new lock"))?;
        write_optional(&directory.path().join(RECEIPT), Some(b"new receipt"))?;
        drop(transaction);
        recover(directory.path())?;
        assert!(!directory.path().join(RECEIPT).exists());
        assert!(!directory.path().join(LOCK).exists());
        Ok(())
    }

    #[test]
    fn failed_rollback_remains_retryable() -> Result<(), Error> {
        let directory = previous_pair()?;
        let transaction = Transaction::begin(directory.path())?;
        fs_err::remove_file(directory.path().join(RECEIPT))?;
        fs_err::create_dir(directory.path().join(RECEIPT))?;
        let error = transaction
            .commit(|| {
                write_optional(&directory.path().join(LOCK), Some(b"new lock"))?;
                write_optional(&directory.path().join(RECEIPT), Some(b"new receipt"))
            })
            .expect_err("receipt directory blocks publication and recovery");
        assert!(matches!(error, Error::MetadataRecovery { .. }));
        assert!(directory.path().join(DIRECTORY).join(JOURNAL).exists());
        fs_err::remove_dir(directory.path().join(RECEIPT))?;
        recover(directory.path())?;
        assert_previous_pair(directory.path())?;
        Ok(())
    }

    #[test]
    fn rollback_cleanup_obstruction_keeps_previous_pair_readable() -> Result<(), Error> {
        let directory = previous_pair()?;
        let receipt = fs_err::read(directory.path().join(RECEIPT))?;
        let lock = fs_err::read(directory.path().join(LOCK))?;
        let transaction = Transaction::begin(directory.path())?;
        let obstruction = transaction.journal.join("unrelated.txt");
        fs_err::write(&obstruction, "keep")?;
        let error = transaction
            .commit(|| {
                write_optional(&directory.path().join(LOCK), Some(b"new lock"))?;
                Err(io::Error::other("receipt publication failed"))
            })
            .expect_err("publication fails, but rollback succeeds");
        assert_eq!(
            error.as_io_error().map(io::Error::kind),
            Some(io::ErrorKind::Other)
        );
        recover(directory.path())?;
        assert_eq!(fs_err::read(directory.path().join(RECEIPT))?, receipt);
        assert_eq!(fs_err::read(directory.path().join(LOCK))?, lock);
        let pending = directory.path().join(DIRECTORY);
        assert!(pending.join(COMMITTED).exists());
        assert_eq!(fs_err::read(pending.join(RECEIPT))?, receipt);
        assert_eq!(fs_err::read(pending.join(LOCK))?, lock);
        let error = Transaction::begin(directory.path()).expect_err("cleanup is still pending");
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        fs_err::remove_file(obstruction)?;
        recover(directory.path())?;
        assert_previous_pair(directory.path())?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn metadata_permissions_survive_publication_and_rollback() -> Result<(), Error> {
        let directory = previous_pair()?;
        let receipt = directory.path().join(RECEIPT);
        let lock = directory.path().join(LOCK);
        fs_err::set_permissions(&receipt, std::fs::Permissions::from_mode(0o600))?;
        fs_err::set_permissions(&lock, std::fs::Permissions::from_mode(0o640))?;
        commit(
            directory.path(),
            &ToolMetadata::new("new receipt".into(), Some("new lock".into())),
        )?;
        assert_eq!(
            fs_err::metadata(&receipt)?.permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(fs_err::metadata(&lock)?.permissions().mode() & 0o777, 0o640);

        Transaction::begin(directory.path())?
            .commit(|| {
                write_optional(&lock, None)?;
                Err(io::Error::other("receipt publication failed"))
            })
            .expect_err("publication fails after removing the lock");
        assert_eq!(fs_err::read(&receipt)?, b"new receipt");
        assert_eq!(fs_err::read(&lock)?, b"new lock");
        assert_eq!(
            fs_err::metadata(&receipt)?.permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(fs_err::metadata(&lock)?.permissions().mode() & 0o777, 0o640);
        assert!(!directory.path().join(DIRECTORY).exists());
        Ok(())
    }

    #[test]
    fn publication_does_not_modify_a_hard_linked_receipt_alias() -> Result<(), Error> {
        let directory = previous_pair()?;
        let alias = directory.path().join("external-receipt");
        fs_err::hard_link(directory.path().join(RECEIPT), &alias)?;
        let original = fs_err::read(&alias)?;
        commit(
            directory.path(),
            &ToolMetadata::new("new receipt".into(), None),
        )?;
        assert_eq!(fs_err::read(&alias)?, original);
        assert_eq!(
            fs_err::read(directory.path().join(RECEIPT))?,
            b"new receipt"
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn publication_does_not_follow_a_receipt_symlink() -> Result<(), Error> {
        let directory = previous_pair()?;
        let receipt = directory.path().join(RECEIPT);
        let target = directory.path().join("external-receipt");
        fs_err::rename(&receipt, &target)?;
        let original = fs_err::read(&target)?;
        fs_err::os::unix::fs::symlink(&target, &receipt)?;
        commit(
            directory.path(),
            &ToolMetadata::new("new receipt".into(), None),
        )?;
        assert_eq!(fs_err::read(&target)?, original);
        assert_eq!(fs_err::read(&receipt)?, b"new receipt");
        assert!(fs_err::symlink_metadata(&receipt)?.is_file());
        Ok(())
    }

    #[test]
    fn completed_pair_is_not_rolled_back_during_cleanup() -> Result<(), Error> {
        let directory = previous_pair()?;
        let transaction = Transaction::begin(directory.path())?;
        write_optional(&directory.path().join(LOCK), None)?;
        write_optional(&directory.path().join(RECEIPT), Some(b"new receipt"))?;
        fs_err::rename(
            transaction.journal.join(JOURNAL),
            transaction.journal.join(COMMITTED),
        )?;
        fs_err::remove_file(transaction.journal.join(LOCK))?;
        let backup = transaction.journal.join(RECEIPT);
        let mut permissions = fs_err::metadata(&backup)?.permissions();
        permissions.set_readonly(true);
        fs_err::set_permissions(&backup, permissions)?;
        drop(transaction);
        recover(directory.path())?;
        assert_eq!(
            fs_err::read(directory.path().join(RECEIPT))?,
            b"new receipt"
        );
        assert!(!directory.path().join(LOCK).exists());
        assert!(!directory.path().join(DIRECTORY).exists());
        commit(
            directory.path(),
            &ToolMetadata::new("next receipt".into(), Some("next lock".into())),
        )?;
        assert_eq!(
            fs_err::read(directory.path().join(RECEIPT))?,
            b"next receipt"
        );
        assert_eq!(fs_err::read(directory.path().join(LOCK))?, b"next lock");
        Ok(())
    }
}
