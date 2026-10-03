use std::error::Error as StdError;

use uv_keyring::Error;

const MESSAGE: &str = "No built-in credential store is enabled for this platform";

#[test]
fn no_default_store_message() {
    let error = Error::NoDefaultCredentialBuilder;
    assert_eq!(error.to_string(), MESSAGE);
    assert!(StdError::source(&error).is_none());
}

#[cfg(not(any(
    all(target_os = "linux", feature = "secret-service"),
    all(target_os = "freebsd", feature = "secret-service"),
    all(target_os = "openbsd", feature = "secret-service"),
    all(target_os = "macos", feature = "apple-native"),
    all(target_os = "windows", feature = "windows-native"),
)))]
mod no_native_store {
    use std::assert_matches;

    use uv_keyring::{Entry, Error};

    use super::MESSAGE;

    #[test]
    fn entry_reports_unavailable_store() {
        let error = Entry::new("uv-no-store-test", "authored")
            .expect_err("an entry cannot be created without a credential store");
        assert_matches!(&error, Error::NoDefaultCredentialBuilder);
        assert_eq!(error.to_string(), MESSAGE);
    }
}
