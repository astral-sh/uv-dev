use std::fmt;
use std::str::FromStr;

use crate::{HashDigest, HashError};

/// A declared hash whose validation is deferred until its requirement is applicable.
///
/// Requirements excluded by markers or ignored as resolution preferences do not need valid
/// hashes. Retain their original text for serialization and validate only when selecting digests.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct HashDigestInput(String);

impl HashDigestInput {
    /// Parse an applicable declaration into a validated digest.
    pub fn parse(&self) -> Result<HashDigest, HashError> {
        HashDigest::from_str(&self.0)
    }
}

impl From<String> for HashDigestInput {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for HashDigestInput {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl fmt::Display for HashDigestInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Debug for HashDigestInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::HashDigestInput;

    #[test]
    fn retains_raw_hash_declarations() -> Result<(), Box<dyn std::error::Error>> {
        let raw = format!("sha256:{}", "AB".repeat(32));
        let input = HashDigestInput::from(raw.clone());
        assert_eq!(serde_json::to_string(&input)?, serde_json::to_string(&raw)?);
        assert_eq!(input.parse()?.digest(), "ab".repeat(32));
        let malformed: HashDigestInput = serde_json::from_str(r#""not-a-hash""#)?;
        assert_eq!(malformed.to_string(), "not-a-hash");
        assert!(malformed.parse().is_err());
        Ok(())
    }
}
