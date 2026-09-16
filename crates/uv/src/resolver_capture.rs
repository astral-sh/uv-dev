//! Invocation-local publication of the direct lock command's internal resolver evidence.

use std::error::Error;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use uv_fs::verbatim_path;
use uv_resolver::no_solution_capture::CaptureToken;

use crate::commands::{UvError, pip};

pub(crate) struct InvocationCapture {
    destination: PathBuf,
    token: CaptureToken,
}

impl InvocationCapture {
    pub(crate) fn new(
        destination: Option<OsString>,
        request: Option<OsString>,
        producer_pid: u32,
    ) -> Option<Self> {
        let destination = PathBuf::from(destination?);
        let request = request?;
        let token = CaptureToken::new(request.to_str()?, producer_pid)?;
        if !destination.is_absolute()
            || destination.file_name().is_none()
            || !destination.parent().is_some_and(|parent| {
                fs_err::metadata(parent).is_ok_and(|metadata| metadata.is_dir())
            })
        {
            return None;
        }
        Some(Self { destination, token })
    }

    pub(crate) fn token(&self) -> CaptureToken {
        self.token.clone()
    }

    /// A publication failure cannot replace the already selected command error or its output.
    pub(crate) fn publish(&self, error: &UvError) {
        let Some(pip::operations::Error::NoSolution { source, .. }) =
            direct_user_error::<pip::operations::Error>(error)
        else {
            return;
        };
        let Some(evidence) = source.internal_no_solution_capture() else {
            return;
        };
        if !evidence.matches(&self.token) {
            return;
        }
        let Ok(bytes) = evidence.to_json() else {
            return;
        };
        let _ = publish_bytes(&self.destination, &bytes);
    }
}

/// Do not search an anyhow context chain: a nested resolver failure is not the direct operation.
fn direct_user_error<T: Error + Send + Sync + 'static>(error: &UvError) -> Option<&T> {
    match error {
        UvError::User(error) => {
            let outer: &(dyn Error + Send + Sync + 'static) = error.as_ref();
            outer.downcast_ref()
        }
        UvError::Argument(_) | UvError::Unexpected(_) => None,
    }
}

fn publish_bytes(destination: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = destination
        .parent()
        .ok_or_else(|| io::Error::other("capture destination has no parent"))?;
    // The caller supplies a private, pre-existing parent directory. tempfile's native default is
    // 0600 on Unix; Windows inherits the parent's access policy. Both paths need verbatim form
    // for tempfile's direct Win32 calls. No existing destination, including a symlink, is replaced.
    let mut file = tempfile::Builder::new()
        .prefix(".uv-resolver-capture-")
        .tempfile_in(verbatim_path(parent))?;
    file.write_all(bytes)?;
    file.flush()?;
    file.persist_noclobber(verbatim_path(destination))
        .map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_direct_user_error_is_eligible() {
        let direct = UvError::User(io::Error::other("direct").into());
        assert!(direct_user_error::<io::Error>(&direct).is_some());
        let nested =
            UvError::User(anyhow::Error::new(io::Error::other("nested")).context("metadata"));
        assert!(direct_user_error::<io::Error>(&nested).is_none());
        let unexpected = UvError::Unexpected(io::Error::other("unexpected").into());
        assert!(direct_user_error::<io::Error>(&unexpected).is_none());
        let argument = UvError::Argument(io::Error::other("argument").into());
        assert!(direct_user_error::<io::Error>(&argument).is_none());
    }

    #[test]
    fn publication_never_replaces_an_existing_entry() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let destination = directory.path().join("capture.json");
        publish_bytes(&destination, b"complete first envelope")?;
        assert_eq!(fs_err::read(&destination)?, b"complete first envelope");
        assert!(publish_bytes(&destination, b"replacement").is_err());
        assert_eq!(fs_err::read(&destination)?, b"complete first envelope");
        assert!(publish_bytes(&directory.path().join("missing/capture.json"), b"missing").is_err());
        assert!(!directory.path().join("missing").exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn publication_uses_private_permissions_and_rejects_symlinks() -> anyhow::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir()?;
        let destination = directory.path().join("capture.json");
        publish_bytes(&destination, b"complete envelope")?;
        assert_eq!(
            fs_err::metadata(&destination)?.permissions().mode() & 0o777,
            0o600
        );
        let link = directory.path().join("dangling.json");
        fs_err::os::unix::fs::symlink(directory.path().join("absent"), &link)?;
        assert!(publish_bytes(&link, b"replacement").is_err());
        assert!(fs_err::symlink_metadata(&link)?.file_type().is_symlink());
        Ok(())
    }

    #[test]
    fn request_requires_an_absolute_existing_parent_and_valid_token() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let destination = directory.path().join("capture.json").into_os_string();
        let request = OsString::from("0123456789abcdef0123456789abcdef");
        assert!(
            InvocationCapture::new(Some(destination.clone()), Some(request.clone()), 1).is_some()
        );
        assert!(
            InvocationCapture::new(Some(destination.clone()), Some(request.clone()), 0).is_none()
        );
        assert!(
            InvocationCapture::new(Some(destination), Some(OsString::from("invalid")), 1).is_none()
        );
        assert!(
            InvocationCapture::new(
                Some(OsString::from("relative.json")),
                Some(request.clone()),
                1
            )
            .is_none()
        );
        assert!(
            InvocationCapture::new(
                Some(
                    directory
                        .path()
                        .join("missing/capture.json")
                        .into_os_string()
                ),
                Some(request),
                1,
            )
            .is_none()
        );
        Ok(())
    }
}
