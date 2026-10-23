/*!

# Platform-independent secure storage model

This module defines the [`CredentialApi`] trait for entries in platform-specific
credential stores. Implementations must be thread-safe, a requirement captured
in the [`Credential`] type that wraps the trait.
 */
use std::any::Any;
use std::collections::HashMap;

use crate::Result;

/// The API that [credentials](Credential) implement.
#[async_trait::async_trait]
pub trait CredentialApi {
    /// Set the credential's password (a string).
    ///
    /// This will persist the password in the underlying store.
    async fn set_password(&self, password: &str) -> Result<()> {
        self.set_secret(password.as_bytes()).await
    }

    /// Set the credential's secret (a byte array).
    ///
    /// This will persist the secret in the underlying store.
    async fn set_secret(&self, password: &[u8]) -> Result<()>;

    /// Retrieve the password (a string) from the underlying credential.
    ///
    /// This has no effect on the underlying store. If there is no credential
    /// for this entry, a [`NoEntry`](crate::Error::NoEntry) error is returned.
    async fn get_password(&self) -> Result<String> {
        let secret = self.get_secret().await?;
        crate::error::decode_password(secret)
    }

    /// Retrieve a secret (a byte array) from the credential.
    ///
    /// This has no effect on the underlying store. If there is no credential
    /// for this entry, a [NoEntry](crate::Error::NoEntry) error is returned.
    async fn get_secret(&self) -> Result<Vec<u8>>;

    /// Get the secure store attributes on this entry's credential.
    ///
    /// Each credential store may support reading and updating different
    /// named attributes; see the documentation on each of the stores
    /// for details. Note that the keyring itself uses some of these
    /// attributes to map entries to their underlying credential; these
    /// _controlled_ attributes are not available for reading or updating.
    ///
    /// We provide a default (no-op) implementation of this method
    /// for backward compatibility with stores that don't implement it.
    async fn get_attributes(&self) -> Result<HashMap<String, String>> {
        // this should err in the same cases as get_secret, so first call that for effect
        self.get_secret().await?;
        // if we got this far, return success with no attributes
        Ok(HashMap::new())
    }

    /// Update the secure store attributes on this entry's credential.
    ///
    /// Each credential store may support reading and updating different
    /// named attributes; see the documentation on each of the stores
    /// for details. The implementation will ignore any attribute names
    /// that you supply that are not available for update. Because the
    /// names used by the different stores tend to be distinct, you can
    /// write cross-platform code that will work correctly on each platform.
    ///
    /// We provide a default no-op implementation of this method
    /// for backward compatibility with stores that don't implement it.
    async fn update_attributes(&self, _: &HashMap<&str, &str>) -> Result<()> {
        // this should err in the same cases as get_secret, so first call that for effect
        self.get_secret().await?;
        // if we got this far, return success after setting no attributes
        Ok(())
    }

    /// Delete the underlying credential, if there is one.
    ///
    /// This is not idempotent if the credential existed!
    /// A second call to `delete_credential` will return
    /// a [`NoEntry`](crate::Error::NoEntry) error.
    async fn delete_credential(&self) -> Result<()>;

    /// Return the underlying concrete object cast to [Any].
    ///
    /// This allows clients
    /// to downcast the credential to its concrete type so they
    /// can do platform-specific things with it (e.g.,
    /// query its attributes in the underlying store).
    fn as_any(&self) -> &dyn Any;

    /// The `Debug` trait call for the object.
    ///
    /// This is used to implement the `Debug` trait on this type; it
    /// allows generic code to provide debug printing as provided by
    /// the underlying concrete object.
    ///
    /// We provide a (useless) default implementation for backward
    /// compatibility with existing implementors who may have not
    /// implemented the `Debug` trait for their credential objects
    fn debug_fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self.as_any(), f)
    }
}

/// A thread-safe implementation of the [Credential API](CredentialApi).
pub type Credential = dyn CredentialApi + Send + Sync;

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.debug_fmt(f)
    }
}
