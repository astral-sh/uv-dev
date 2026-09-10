use std::fmt::{Debug, Display, Formatter};
use std::str::{self, FromStr};

use thiserror::Error;

/// Unique identity of any Git object (commit, tree, blob, tag).
///
/// This type's `FromStr` implementation validates that it's exactly 40 hex characters, i.e. a
/// full-length git commit.
///
/// If Git's SHA-256 support becomes more widespread in the future (in particular if GitHub ever
/// adds support), we might need to make this an enum.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GitOid {
    bytes: [u8; 40],
}

impl GitOid {
    /// Return the string representation of an object ID.
    pub fn as_str(&self) -> &str {
        str::from_utf8(&self.bytes).unwrap()
    }

    /// Return a truncated representation, i.e., the first 16 characters of the SHA.
    pub fn as_short_str(&self) -> &str {
        &self.as_str()[..16]
    }

    /// Return a (very) truncated representation, i.e., the first 8 characters of the SHA.
    pub fn as_tiny_str(&self) -> &str {
        &self.as_str()[..8]
    }
}

impl Debug for GitOid {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "GitOid(\"{}\")", self.as_str())
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum OidParseError {
    #[error("Object ID cannot be parsed from empty string")]
    Empty,
    #[error("Object ID must be exactly 40 hex characters")]
    WrongLength,
    #[error("Object ID must be valid hex characters")]
    NotHex,
}

impl FromStr for GitOid {
    type Err = OidParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(OidParseError::Empty);
        }

        if s.len() != 40 {
            return Err(OidParseError::WrongLength);
        }

        if !s.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Err(OidParseError::NotHex);
        }

        let mut bytes = [0; 40];
        bytes.copy_from_slice(s.as_bytes());
        Ok(Self { bytes })
    }
}

impl Display for GitOid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl serde::Serialize for GitOid {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.as_str().serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for GitOid {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl serde::de::Visitor<'_> for Visitor {
            type Value = GitOid;

            fn expecting(&self, f: &mut Formatter) -> std::fmt::Result {
                f.write_str("a string")
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                GitOid::from_str(v).map_err(serde::de::Error::custom)
            }
        }

        deserializer.deserialize_str(Visitor)
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use serde::Deserialize;
    use serde::de::value::{
        BorrowedBytesDeserializer, BorrowedStrDeserializer, Error, StrDeserializer,
        StringDeserializer, U64Deserializer,
    };

    use super::{GitOid, OidParseError};

    fn deserialize_strings(input: &str) -> [Result<GitOid, Error>; 3] {
        [
            GitOid::deserialize(StrDeserializer::new(input)),
            GitOid::deserialize(BorrowedStrDeserializer::new(input)),
            GitOid::deserialize(StringDeserializer::new(input.to_owned())),
        ]
    }

    #[test]
    fn git_oid() {
        GitOid::from_str("4a23745badf5bf5ef7928f1e346e9986bd696d82").unwrap();
        GitOid::from_str("4A23745BADF5BF5EF7928F1E346E9986BD696D82").unwrap();

        assert_eq!(GitOid::from_str(""), Err(OidParseError::Empty));
        assert_eq!(
            GitOid::from_str(&str::repeat("a", 41)),
            Err(OidParseError::WrongLength)
        );
        assert_eq!(
            GitOid::from_str(&str::repeat("a", 39)),
            Err(OidParseError::WrongLength)
        );
        assert_eq!(
            GitOid::from_str(&str::repeat("x", 40)),
            Err(OidParseError::NotHex)
        );
    }

    #[test]
    fn git_oid_deserialize_preserves_spelling() {
        for input in [
            "4a23745badf5bf5ef7928f1e346e9986bd696d82",
            "4A23745BADF5BF5EF7928F1E346E9986BD696D82",
        ] {
            for oid in deserialize_strings(input) {
                let oid = oid.unwrap();
                assert_eq!(oid, input.parse::<GitOid>().unwrap());
                assert_eq!(oid.as_str(), input);
                assert_eq!(oid.as_short_str(), &input[..16]);
                assert_eq!(oid.as_tiny_str(), &input[..8]);
                assert_eq!(oid.to_string(), input);
                assert_eq!(format!("{oid:?}"), format!("GitOid({input:?})"));
            }
        }
    }

    #[test]
    fn git_oid_deserialize_rejects_invalid_oids() {
        for (input, expected) in [
            (String::new(), OidParseError::Empty),
            ("a".repeat(16), OidParseError::WrongLength),
            ("a".repeat(39), OidParseError::WrongLength),
            ("a".repeat(41), OidParseError::WrongLength),
            ("a".repeat(64), OidParseError::WrongLength),
            ("g".repeat(40), OidParseError::NotHex),
            ("é".repeat(20), OidParseError::NotHex),
        ] {
            for result in deserialize_strings(&input) {
                assert_eq!(result.unwrap_err().to_string(), expected.to_string());
            }
        }
    }

    #[test]
    fn git_oid_deserialize_requires_a_string() {
        let error = GitOid::deserialize(U64Deserializer::<Error>::new(42)).unwrap_err();
        assert_eq!(
            error.to_string(),
            "invalid type: integer `42`, expected a string"
        );

        let error = GitOid::deserialize(BorrowedBytesDeserializer::<Error>::new(
            b"4a23745badf5bf5ef7928f1e346e9986bd696d82",
        ))
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "invalid type: byte array, expected a string"
        );
    }
}
