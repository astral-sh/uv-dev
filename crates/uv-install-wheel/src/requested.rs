use std::collections::BTreeSet;
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};

use data_encoding::BASE64URL_NOPAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use uv_fs::{Simplified, write_atomic_sync};
use uv_normalize::ExtraName;

use crate::RecordEntry;

pub(crate) const FILE_NAME: &str = "uv_requested.json";

/// Extras explicitly requested when installing a package with uv.
///
/// Absence of this metadata does not imply that no extras were requested: other installers and
/// older versions of uv do not record these selections.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestedExtras {
    pub extras: BTreeSet<ExtraName>,
}

impl RequestedExtras {
    /// Read requested extras from an installed distribution's `.dist-info` directory.
    pub fn read(dist_info: &Path) -> Result<Option<Self>, RequestedExtrasError> {
        let file = match fs_err::File::open(dist_info.join(FILE_NAME)) {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err.into()),
        };
        Ok(Some(serde_json::from_reader(BufReader::new(file))?))
    }

    /// Plan a receipt and its `RECORD` entry without changing the environment.
    /// Distributions managed without a `RECORD` cannot track an installer receipt.
    pub fn plan(
        &self,
        dist_info: &Path,
    ) -> Result<Option<RequestedExtrasUpdate>, RequestedExtrasError> {
        let metadata_path = dist_info.join(FILE_NAME);
        let content = if self.extras.is_empty() {
            None
        } else {
            Some(serde_json::to_vec(self)?)
        };
        let existing = match fs_err::read(&metadata_path) {
            Ok(content) => Some(content),
            Err(err) if err.kind() == io::ErrorKind::NotFound => None,
            Err(err) => return Err(err.into()),
        };
        if existing.is_none() && content.is_none() {
            return Ok(None);
        }

        let record_path = dist_info.join("RECORD");
        let existing_record = match fs_err::read(&record_path) {
            Ok(content) => content,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err.into()),
        };
        // Keep unrelated entries exactly as recorded, including absolute paths.
        let mut record = csv::ReaderBuilder::new()
            .has_headers(false)
            .escape(Some(b'"'))
            .from_reader(existing_record.as_slice())
            .into_deserialize::<RecordEntry>()
            .collect::<Result<Vec<_>, _>>()?;
        let directory = dist_info
            .file_name()
            .ok_or_else(|| RequestedExtrasError::InvalidDistInfoPath(dist_info.to_path_buf()))?;
        let relative_path = Path::new(directory)
            .join(FILE_NAME)
            .portable_display()
            .to_string();
        let expected = content.as_ref().map(|content| RecordEntry {
            path: relative_path.clone(),
            hash: Some(format!(
                "sha256={}",
                BASE64URL_NOPAD.encode(&Sha256::digest(content))
            )),
            size: Some(content.len() as u64),
        });
        // Keep an accurate declaration in place. Repair missing, stale, or duplicate entries.
        let mut found = false;
        let mut record_changed = false;
        record.retain(|entry| {
            if uv_fs::normalize_path(Path::new(&entry.path.replace('\\', "/"))).as_ref()
                != Path::new(&relative_path)
            {
                return true;
            }
            if !found
                && expected
                    .as_ref()
                    .is_some_and(|want| entry.hash == want.hash && entry.size == want.size)
            {
                found = true;
                true
            } else {
                record_changed = true;
                false
            }
        });
        if !found && let Some(entry) = expected {
            let pos = record
                .iter()
                .position(|current| current.path > entry.path)
                .unwrap_or(record.len());
            record.insert(pos, entry);
            record_changed = true;
        }

        let receipt = if existing == content {
            ReceiptChange::Unchanged
        } else if let Some(content) = content {
            ReceiptChange::Write(content)
        } else {
            ReceiptChange::Remove
        };
        if !record_changed && matches!(receipt, ReceiptChange::Unchanged) {
            return Ok(None);
        }
        let record = if record_changed {
            let mut writer = csv::WriterBuilder::new()
                .has_headers(false)
                .escape(b'"')
                .from_writer(Vec::new());
            for entry in record {
                writer.serialize(entry)?;
            }
            Some(
                writer
                    .into_inner()
                    .map_err(csv::IntoInnerError::into_error)?,
            )
        } else {
            None
        };
        Ok(Some(RequestedExtrasUpdate {
            dist_info: dist_info.to_owned(),
            receipt,
            record,
        }))
    }
}

/// A pending update to an installed distribution's requested extras.
pub struct RequestedExtrasUpdate {
    dist_info: PathBuf,
    receipt: ReceiptChange,
    record: Option<Vec<u8>>,
}

enum ReceiptChange {
    Unchanged,
    Write(Vec<u8>),
    Remove,
}

impl RequestedExtrasUpdate {
    /// Apply the planned update. Publish the receipt after tracking it in `RECORD`,
    /// and remove it before dropping its entry so an interrupted update remains recoverable.
    pub fn apply(self) -> Result<(), RequestedExtrasError> {
        if let ReceiptChange::Remove = self.receipt {
            fs_err::remove_file(self.dist_info.join(FILE_NAME))?;
        }
        if let Some(record) = self.record {
            write_atomic_sync(self.dist_info.join("RECORD"), record)?;
        }
        if let ReceiptChange::Write(content) = self.receipt {
            write_atomic_sync(self.dist_info.join(FILE_NAME), content)?;
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum RequestedExtrasError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("Invalid `uv_requested.json`")]
    Json(#[from] serde_json::Error),
    #[error("Invalid `RECORD`")]
    Record(#[from] csv::Error),
    #[error("Invalid `.dist-info` path: {}", .0.user_display())]
    InvalidDistInfoPath(PathBuf),
}
