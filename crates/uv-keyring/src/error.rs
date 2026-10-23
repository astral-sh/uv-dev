/*!

Platform-independent error model.

There is an escape hatch here for surfacing platform-specific
error information returned by the platform-specific storage provider,
but the concrete objects returned must be `Send` so they can be
moved from one thread to another. (Since most platform errors
are integer error codes, this requirement
is not much of a burden on the platform-specific store providers.)
 */

use crate::Credential;

#[derive(Debug, thiserror::Error)]
/// Each variant of the `Error` enum provides a summary of the error.
/// More details, if relevant, are contained in the associated value,
/// which may be platform-specific.
///
/// This enum is non-exhaustive so that more values can be added to it
/// without a `SemVer` break. Clients should always have default handling
/// for variants they don't understand.
#[non_exhaustive]
pub enum Error {
    /// This indicates runtime failure in the underlying
    /// platform storage system.  The details of the failure can
    /// be retrieved from the attached platform error.
    #[error("Platform secure storage failure")]
    PlatformFailure(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// This indicates that the underlying secure storage
    /// holding saved items could not be accessed.  Typically, this
    /// is because of access rules in the platform; for example, it
    /// might be that the credential store is locked.  The underlying
    /// platform error will typically give the reason.
    #[error("Couldn't access platform secure storage")]
    NoStorageAccess(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// This indicates that there is no underlying credential
    /// entry in the platform for this entry.  Either one was
    /// never set, or it was deleted.
    #[error("No matching entry found in secure storage")]
    NoEntry,
    /// This indicates that the retrieved password blob was not
    /// a UTF-8 string.  The underlying bytes are available
    /// for examination in the attached value.
    #[error("Data is not UTF-8 encoded")]
    BadEncoding(Vec<u8>),
    /// This indicates that one of the entry's credential
    /// attributes exceeded a
    /// length limit in the underlying platform.  The
    /// attached values give the name of the attribute and
    /// the platform length limit that was exceeded.
    #[error("Attribute '{0}' is longer than platform limit of {1} chars")]
    TooLong(String, u32),
    /// This indicates that one of the entry's required credential
    /// attributes was invalid.  The
    /// attached value gives the name of the attribute
    /// and the reason it's invalid.
    #[error("Attribute {0} is invalid: {1}")]
    Invalid(String, String),
    /// This indicates that there is more than one credential found in the store
    /// that matches the entry.  Its value is a vector of the matching credentials.
    #[error("Entry is matched by multiple credentials ({} matches)", .0.len())]
    Ambiguous(Vec<Box<Credential>>),
    /// This indicates that there was no default credential builder to use;
    /// the client must set one before creating entries.
    #[error("No default credential builder is available; set one before creating entries")]
    NoDefaultCredentialBuilder,
}

pub type Result<T> = std::result::Result<T, Error>;

/// Try to interpret a byte vector as a password string
pub(crate) fn decode_password(bytes: Vec<u8>) -> Result<String> {
    String::from_utf8(bytes).map_err(|err| Error::BadEncoding(err.into_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bad_password() {
        // malformed sequences here taken from:
        // https://www.cl.cam.ac.uk/~mgk25/ucs/examples/UTF-8-test.txt
        for bytes in [b"\x80".to_vec(), b"\xbf".to_vec(), b"\xed\xa0\xa0".to_vec()] {
            match decode_password(bytes.clone()) {
                Err(Error::BadEncoding(str)) => assert_eq!(str, bytes),
                Err(other) => panic!("Bad password ({bytes:?}) decode gave wrong error: {other}"),
                Ok(s) => panic!("Bad password ({bytes:?}) decode gave results: {s:?}"),
            }
        }
    }
}

#[cfg(all(
    test,
    any(
        all(
            any(target_os = "linux", target_os = "freebsd", target_os = "openbsd"),
            feature = "secret-service"
        ),
        all(target_os = "macos", feature = "apple-native"),
        all(target_os = "windows", feature = "windows-native"),
    )
))]
mod native_tests {
    use insta::assert_snapshot;
    use uv_errors::{ErrorOptions, Hints, write_error_chain_with_options};

    use crate::{Entry, Error};

    #[test]
    fn ambiguous_warning_redacts_credentials() -> anyhow::Result<()> {
        let service = "uv:https://synthetic-url-token@example.com/?sig=synthetic-signature";
        let username = "synthetic-username-token";
        // Constructing entries only records their attributes; it does not access the native store.
        let credentials = vec![
            Entry::new(service, username)?.inner,
            Entry::new(service, username)?.inner,
        ];
        let error = anyhow::Error::from(Error::Ambiguous(credentials)).context(
            "Unable to fetch credentials for https://****@example.com/?sig=**** from system keyring",
        );
        let mut output = String::new();
        temp_env::with_var("UV_NO_WRAP", Some("1"), || {
            write_error_chain_with_options(
                error.as_ref(),
                &Hints::none(),
                ErrorOptions::default()
                    .with_level("warning")
                    .with_stream(&mut output),
            )
        })?;
        assert_snapshot!(anstream::adapter::strip_str(&output), @"
        warning: Unable to fetch credentials for https://****@example.com/?sig=**** from system keyring
          cause: Entry is matched by multiple credentials (2 matches)
        ");

        let Some(Error::Ambiguous(credentials)) = error.downcast_ref::<Error>() else {
            anyhow::bail!("Expected the typed ambiguity error");
        };
        assert_eq!(credentials.len(), 2);
        Ok(())
    }
}
