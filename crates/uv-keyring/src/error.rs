/*!

Platform-independent error model.

There is an escape hatch here for surfacing platform-specific
error information returned by the platform-specific storage provider,
but the concrete objects returned must be `Send` so they can be
moved from one thread to another. (Since most platform errors
are integer error codes, this requirement
is not much of a burden on the platform-specific store providers.)
 */

use std::fmt;

use crate::Credential;

#[derive(thiserror::Error)]
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
    /// for examination in the attached value, but redacted from debug output.
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
    #[error("Entry is matched by multiple credentials: {0:?}")]
    Ambiguous(Vec<Box<Credential>>),
    /// This indicates that there was no default credential builder to use;
    /// the client must set one before creating entries.
    #[error("No default credential builder is available; set one before creating entries")]
    NoDefaultCredentialBuilder,
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PlatformFailure(error) => f.debug_tuple("PlatformFailure").field(error).finish(),
            Self::NoStorageAccess(error) => f.debug_tuple("NoStorageAccess").field(error).finish(),
            Self::NoEntry => f.write_str("NoEntry"),
            Self::BadEncoding(_) => f.write_str("BadEncoding(****)"),
            Self::TooLong(attribute, limit) => f
                .debug_tuple("TooLong")
                .field(attribute)
                .field(limit)
                .finish(),
            Self::Invalid(attribute, reason) => f
                .debug_tuple("Invalid")
                .field(attribute)
                .field(reason)
                .finish(),
            Self::Ambiguous(credentials) => f.debug_tuple("Ambiguous").field(credentials).finish(),
            Self::NoDefaultCredentialBuilder => f.write_str("NoDefaultCredentialBuilder"),
        }
    }
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
            let error = decode_password(bytes.clone()).unwrap_err();
            assert_eq!(format!("{error:?}"), "BadEncoding(****)");
            match error {
                Error::BadEncoding(str) => assert_eq!(str, bytes),
                other => panic!("Bad password ({bytes:?}) decode gave wrong error: {other}"),
            }
        }
    }
}
