//! An in-memory credential used by platform-independent keyring tests.
//!
//! Each credential owns its secret and can return a configured error once.
use std::sync::Mutex;

use crate::credential::CredentialApi;
use crate::error::{Error, Result};

#[derive(Debug, Default)]
struct MockCredential {
    inner: Mutex<MockData>,
}

#[derive(Debug, Default)]
struct MockData {
    secret: Option<Vec<u8>>,
    error: Option<Error>,
}

impl MockData {
    /// Consume the error configured for the next credential operation, if any.
    fn check_error(&mut self) -> Result<()> {
        match self.error.take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

#[async_trait::async_trait]
impl CredentialApi for MockCredential {
    async fn set_secret(&self, secret: &[u8]) -> Result<()> {
        let mut data = self.inner.lock().expect("Can't access mock data for set");
        data.check_error()?;
        data.secret = Some(secret.to_vec());
        Ok(())
    }

    async fn get_secret(&self) -> Result<Vec<u8>> {
        let mut data = self.inner.lock().expect("Can't access mock data for get");
        data.check_error()?;
        data.secret.clone().ok_or(Error::NoEntry)
    }

    async fn delete_credential(&self) -> Result<()> {
        let mut data = self
            .inner
            .lock()
            .expect("Can't access mock data for delete");
        data.check_error()?;
        data.secret.take().ok_or(Error::NoEntry)?;
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn debug_fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
}

impl MockCredential {
    /// Set an error to be returned from this mock credential.
    ///
    /// Error returns always take precedence over the normal
    /// behavior of the mock.  But once an error has been
    /// returned it is removed, so the mock works thereafter.
    fn set_error(&self, err: Error) {
        let mut data = self
            .inner
            .lock()
            .expect("Can't access mock data for set_error");
        data.error = Some(err);
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::MockCredential;
    use crate::{Entry, Error, tests::generate_random_string};

    fn entry_new(_service: &str, _user: &str) -> Entry {
        Entry::new_with_credential(Box::<MockCredential>::default())
    }

    #[tokio::test]
    async fn test_missing_entry() {
        crate::tests::test_missing_entry(entry_new).await;
    }

    #[tokio::test]
    async fn test_empty_password() {
        crate::tests::test_empty_password(entry_new).await;
    }

    #[tokio::test]
    async fn test_round_trip_ascii_password() {
        crate::tests::test_round_trip_ascii_password(entry_new).await;
    }

    #[tokio::test]
    async fn test_round_trip_non_ascii_password() {
        crate::tests::test_round_trip_non_ascii_password(entry_new).await;
    }

    #[tokio::test]
    async fn test_round_trip_random_secret() {
        crate::tests::test_round_trip_random_secret(entry_new).await;
    }

    #[tokio::test]
    async fn test_update() {
        crate::tests::test_update(entry_new).await;
    }

    #[tokio::test]
    async fn test_get_update_attributes() {
        crate::tests::test_noop_get_update_attributes(entry_new).await;
    }

    #[tokio::test]
    async fn test_set_error() {
        let name = generate_random_string();
        let entry = entry_new(&name, &name);
        let password = "test ascii password";
        let mock: &MockCredential = entry
            .inner
            .as_any()
            .downcast_ref()
            .expect("Downcast failed");
        mock.set_error(Error::Invalid(
            "mock error".to_string(),
            "is an error".to_string(),
        ));
        assert_matches!(
            entry.set_password(password).await,
            Err(Error::Invalid(_, _)),
            "set: No error"
        );
        entry
            .set_password(password)
            .await
            .expect("set: Error not cleared");
        mock.set_error(Error::NoEntry);
        assert_matches!(
            entry.get_password().await,
            Err(Error::NoEntry),
            "get: No error"
        );
        let stored_password = entry.get_password().await.expect("get: Error not cleared");
        assert_eq!(
            stored_password, password,
            "Retrieved and set ascii passwords don't match"
        );
        mock.set_error(Error::TooLong("mock".to_string(), 3));
        assert_matches!(
            entry.delete_credential().await,
            Err(Error::TooLong(_, 3)),
            "delete: No error"
        );
        entry
            .delete_credential()
            .await
            .expect("delete: Error not cleared");
        assert_matches!(
            entry.get_password().await,
            Err(Error::NoEntry),
            "Able to read a deleted ascii password"
        );
    }
}
