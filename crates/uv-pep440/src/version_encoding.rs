//! Checked component encoding for internal version-range endpoints.

use std::fmt;
use std::marker::PhantomData;

use serde::de::{DeserializeSeed, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

use crate::{LocalSegment, LocalVersion, LocalVersionSlice, Prerelease, PrereleaseKind, Version};

const MAX_COMPONENTS: usize = 256;
const MAX_LOCAL_SEGMENT_BYTES: usize = 16_384;

/// A checked, component-preserving encoding of a version-range endpoint.
///
/// Unlike [`Version`]'s ordinary string serialization, this representation retains the internal
/// min, max, and local-max sentinels. Numeric components are serialized as canonical decimal
/// strings, including values that cannot be parsed as ordinary PEP 440 versions.
///
/// Only zero-valued min/max sentinels in the combinations emitted by the range constructors are
/// supported. Equality compares the encoded components, including release precision, rather than
/// the padded-release equality of [`Version`].
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct EncodedVersion(VersionWire);

impl EncodedVersion {
    /// Maximum number of release or local components in one encoded version.
    pub const MAX_COMPONENTS: usize = MAX_COMPONENTS;

    /// Maximum byte length of a string-valued local component.
    pub const MAX_LOCAL_SEGMENT_BYTES: usize = MAX_LOCAL_SEGMENT_BYTES;

    /// Reconstruct the checked native endpoint without parsing or successor arithmetic.
    pub fn into_version(self) -> Version {
        let VersionWire {
            epoch,
            release,
            pre,
            post,
            dev,
            local,
            min,
            max,
        } = self.0;
        let mut version = Version::new(release.0.into_iter().map(|value| value.0))
            .with_epoch(epoch.0)
            .with_pre(pre.map(EncodedPrerelease::into_native))
            .with_post(post.map(|value| value.0))
            .with_dev(dev.map(|value| value.0));
        version = version.with_local(match local {
            EncodedLocalVersion::Segments(segments) => LocalVersion::Segments(
                segments
                    .0
                    .into_iter()
                    .map(|segment| match segment {
                        EncodedLocalSegment::String(value) => LocalSegment::String(value.0),
                        EncodedLocalSegment::Number(value) => LocalSegment::Number(value.0),
                    })
                    .collect(),
            ),
            EncodedLocalVersion::Max => LocalVersion::Max,
        });
        if let Some(min) = min {
            version = version.with_min(Some(min.0));
        }
        if let Some(max) = max {
            version = version.with_max(Some(max.0));
        }
        version
    }

    /// Reconstruct the checked native endpoint without consuming the encoding.
    pub fn to_version(&self) -> Version {
        self.clone().into_version()
    }
}

impl TryFrom<&Version> for EncodedVersion {
    type Error = VersionEncodingError;

    fn try_from(version: &Version) -> Result<Self, Self::Error> {
        let release = version.release();
        if release.len() > MAX_COMPONENTS {
            return Err(VersionEncodingError::TooManyReleaseComponents);
        }
        let local = match version.local() {
            LocalVersionSlice::Segments(segments) => {
                if segments.len() > MAX_COMPONENTS {
                    return Err(VersionEncodingError::TooManyLocalComponents);
                }
                let mut encoded = Vec::with_capacity(segments.len());
                for segment in segments {
                    encoded.push(match segment {
                        LocalSegment::String(value) => {
                            validate_local_string(value)?;
                            EncodedLocalSegment::String(LocalString(value.clone()))
                        }
                        LocalSegment::Number(value) => EncodedLocalSegment::Number(Decimal(*value)),
                    });
                }
                EncodedLocalVersion::Segments(BoundedVec(encoded))
            }
            LocalVersionSlice::Max => EncodedLocalVersion::Max,
        };
        Self::try_from(VersionWire {
            epoch: Decimal(version.epoch()),
            release: BoundedVec(release.iter().copied().map(Decimal).collect()),
            pre: version.pre().map(EncodedPrerelease::from_native),
            post: version.post().map(Decimal),
            dev: version.dev().map(Decimal),
            local,
            min: Version::min(version).map(Decimal),
            max: Version::max(version).map(Decimal),
        })
    }
}

impl TryFrom<Version> for EncodedVersion {
    type Error = VersionEncodingError;

    fn try_from(version: Version) -> Result<Self, Self::Error> {
        Self::try_from(&version)
    }
}

impl TryFrom<VersionWire> for EncodedVersion {
    type Error = VersionEncodingError;

    fn try_from(wire: VersionWire) -> Result<Self, Self::Error> {
        if wire.release.0.is_empty() {
            return Err(VersionEncodingError::EmptyRelease);
        }
        match (wire.min, wire.max) {
            (None, None) => {}
            (Some(Decimal(0)), None)
                if wire.pre.is_none()
                    && wire.post.is_none()
                    && wire.dev.is_none()
                    && wire.local.is_empty() => {}
            (None, Some(Decimal(0)))
                if wire.post.is_none() && wire.dev.is_none() && wire.local.is_empty() => {}
            _ => return Err(VersionEncodingError::UnsupportedSentinel),
        }
        Ok(Self(wire))
    }
}

impl Serialize for EncodedVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for EncodedVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        VersionWire::deserialize(deserializer)
            .and_then(|wire| Self::try_from(wire).map_err(de::Error::custom))
    }
}

/// An endpoint cannot be represented by the checked component encoding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VersionEncodingError {
    /// A version must contain at least one release component.
    EmptyRelease,
    /// The release has more than [`EncodedVersion::MAX_COMPONENTS`] components.
    TooManyReleaseComponents,
    /// The local version has more than [`EncodedVersion::MAX_COMPONENTS`] components.
    TooManyLocalComponents,
    /// A string local component exceeds [`EncodedVersion::MAX_LOCAL_SEGMENT_BYTES`].
    LocalSegmentTooLong,
    /// A string local component is not in the normalized native representation.
    InvalidLocalSegment,
    /// The min/max components are not a supported zero-valued range sentinel.
    UnsupportedSentinel,
}

impl std::error::Error for VersionEncodingError {}

impl fmt::Display for VersionEncodingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyRelease => "version release is empty",
            Self::TooManyReleaseComponents => "too many version release components",
            Self::TooManyLocalComponents => "too many local version components",
            Self::LocalSegmentTooLong => "local version component is too long",
            Self::InvalidLocalSegment => "local version string component is not normalized",
            Self::UnsupportedSentinel => "unsupported internal version sentinel",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VersionWire {
    epoch: Decimal,
    release: BoundedVec<Decimal, MAX_COMPONENTS>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pre: Option<EncodedPrerelease>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    post: Option<Decimal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dev: Option<Decimal>,
    local: EncodedLocalVersion,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    min: Option<Decimal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max: Option<Decimal>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EncodedPrerelease {
    kind: EncodedPrereleaseKind,
    number: Decimal,
}

impl EncodedPrerelease {
    fn from_native(pre: Prerelease) -> Self {
        Self {
            kind: match pre.kind {
                PrereleaseKind::Alpha => EncodedPrereleaseKind::Alpha,
                PrereleaseKind::Beta => EncodedPrereleaseKind::Beta,
                PrereleaseKind::Rc => EncodedPrereleaseKind::Rc,
            },
            number: Decimal(pre.number),
        }
    }

    fn into_native(self) -> Prerelease {
        Prerelease {
            kind: match self.kind {
                EncodedPrereleaseKind::Alpha => PrereleaseKind::Alpha,
                EncodedPrereleaseKind::Beta => PrereleaseKind::Beta,
                EncodedPrereleaseKind::Rc => PrereleaseKind::Rc,
            },
            number: self.number.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum EncodedPrereleaseKind {
    Alpha,
    Beta,
    Rc,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize)]
#[serde(tag = "kind", content = "segments", rename_all = "snake_case")]
enum EncodedLocalVersion {
    Segments(BoundedVec<EncodedLocalSegment, MAX_COMPONENTS>),
    Max,
}

impl<'de> Deserialize<'de> for EncodedLocalVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "snake_case")]
        enum Kind {
            Segments,
            Max,
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            kind: Kind,
            #[serde(default)]
            segments: Field<BoundedVec<EncodedLocalSegment, MAX_COMPONENTS>>,
        }

        let Wire { kind, segments } = Wire::deserialize(deserializer)?;
        match (kind, segments) {
            (Kind::Segments, Field::Present(segments)) => Ok(Self::Segments(segments)),
            (Kind::Max, Field::Missing) => Ok(Self::Max),
            (Kind::Segments, Field::Missing) => Err(de::Error::missing_field("segments")),
            (Kind::Max, Field::Present(_)) => {
                Err(de::Error::custom("local-max cannot contain segments"))
            }
        }
    }
}

impl EncodedLocalVersion {
    fn is_empty(&self) -> bool {
        matches!(self, Self::Segments(segments) if segments.0.is_empty())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum EncodedLocalSegment {
    String(LocalString),
    Number(Decimal),
}

impl<'de> Deserialize<'de> for EncodedLocalSegment {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "snake_case")]
        enum Kind {
            String,
            Number,
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            kind: Kind,
            value: LocalAtom,
        }

        let Wire { kind, value } = Wire::deserialize(deserializer)?;
        match kind {
            Kind::String => {
                validate_local_string(&value.0).map_err(de::Error::custom)?;
                Ok(Self::String(LocalString(value.0)))
            }
            Kind::Number => Decimal::parse(&value.0)
                .map(Self::Number)
                .map_err(de::Error::custom),
        }
    }
}

/// The JSON representation of a numeric component is never a floating-point number.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
struct Decimal(u64);

impl Decimal {
    fn parse(value: &str) -> Result<Self, &'static str> {
        if value.is_empty()
            || value.len() > 20
            || (value.len() > 1 && value.starts_with('0'))
            || !value.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err("expected a canonical decimal u64 string");
        }
        value
            .parse::<u64>()
            .map(Self)
            .map_err(|_| "decimal version component overflows u64")
    }
}

impl Serialize for Decimal {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Decimal {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct DecimalVisitor;

        impl Visitor<'_> for DecimalVisitor {
            type Value = Decimal;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a canonical decimal u64 string")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Decimal::parse(value).map_err(E::custom)
            }
        }

        deserializer.deserialize_str(DecimalVisitor)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize)]
#[serde(transparent)]
struct LocalString(String);

/// The tag and value may appear in either order without buffering an unbounded enum payload.
struct LocalAtom(String);

impl<'de> Deserialize<'de> for LocalAtom {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct LocalAtomVisitor;

        impl Visitor<'_> for LocalAtomVisitor {
            type Value = LocalAtom;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a bounded local version component string")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                if value.len() > MAX_LOCAL_SEGMENT_BYTES {
                    return Err(E::custom(VersionEncodingError::LocalSegmentTooLong));
                }
                Ok(LocalAtom(value.to_owned()))
            }

            fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
                if value.len() > MAX_LOCAL_SEGMENT_BYTES {
                    return Err(E::custom(VersionEncodingError::LocalSegmentTooLong));
                }
                Ok(LocalAtom(value))
            }
        }

        deserializer.deserialize_str(LocalAtomVisitor)
    }
}

fn validate_local_string(value: &str) -> Result<(), VersionEncodingError> {
    if value.len() > MAX_LOCAL_SEGMENT_BYTES {
        return Err(VersionEncodingError::LocalSegmentTooLong);
    }
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        || value.parse::<u64>().is_ok()
    {
        return Err(VersionEncodingError::InvalidLocalSegment);
    }
    Ok(())
}

/// Unlike `Option<T>`, this distinguishes a missing field from a present JSON null.
#[derive(Default)]
pub(super) enum Field<T> {
    #[default]
    Missing,
    Present(T),
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Field<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        T::deserialize(deserializer).map(Self::Present)
    }
}

/// Reject an excess element before asking it to deserialize or allocating space for it.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize)]
#[serde(transparent)]
pub(super) struct BoundedVec<T, const MAX: usize>(pub(super) Vec<T>);

impl<'de, T: Deserialize<'de>, const MAX: usize> Deserialize<'de> for BoundedVec<T, MAX> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct BoundedVisitor<T, const MAX: usize>(PhantomData<fn() -> T>);
        struct Element<T, const MAX: usize> {
            allowed: bool,
            marker: PhantomData<fn() -> T>,
        }

        impl<'de, T: Deserialize<'de>, const MAX: usize> DeserializeSeed<'de> for Element<T, MAX> {
            type Value = T;

            fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
            where
                D: Deserializer<'de>,
            {
                if !self.allowed {
                    return Err(de::Error::custom(format_args!(
                        "sequence exceeds {MAX} elements"
                    )));
                }
                T::deserialize(deserializer)
            }
        }

        impl<'de, T: Deserialize<'de>, const MAX: usize> Visitor<'de> for BoundedVisitor<T, MAX> {
            type Value = BoundedVec<T, MAX>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "a sequence of at most {MAX} elements")
            }

            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = sequence.next_element_seed(Element::<T, MAX> {
                    allowed: values.len() < MAX,
                    marker: PhantomData,
                })? {
                    if values.len() == values.capacity() {
                        let capacity = values.capacity().saturating_mul(2).max(4).min(MAX);
                        values
                            .try_reserve_exact(capacity.saturating_sub(values.len()))
                            .map_err(|_| de::Error::custom("cannot allocate bounded sequence"))?;
                    }
                    values.push(value);
                }
                Ok(BoundedVec(values))
            }
        }

        deserializer.deserialize_seq(BoundedVisitor::<T, MAX>(PhantomData))
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde_json::{Value, json};

    use super::*;

    fn version(value: &str) -> Version {
        value.parse().expect("valid test version")
    }

    fn assert_components(left: &Version, right: &Version) {
        assert_eq!(left.epoch(), right.epoch());
        assert_eq!(&*left.release(), &*right.release());
        assert_eq!(left.pre(), right.pre());
        assert_eq!(left.post(), right.post());
        assert_eq!(left.dev(), right.dev());
        assert_eq!(left.local(), right.local());
        assert_eq!(Version::min(left), Version::min(right));
        assert_eq!(Version::max(left), Version::max(right));
    }

    fn round_trip(native: &Version) -> Result<EncodedVersion, Box<dyn Error>> {
        let encoded = EncodedVersion::try_from(native)?;
        let json = serde_json::to_vec(&encoded)?;
        let decoded: EncodedVersion = serde_json::from_slice(&json)?;
        assert_eq!(encoded, decoded);
        assert_components(native, &decoded.to_version());
        assert_eq!(encoded, EncodedVersion::try_from(decoded.into_version())?);
        Ok(encoded)
    }

    fn plain_wire() -> Value {
        json!({
            "epoch": "0",
            "release": ["1", "0"],
            "local": {"kind": "segments", "segments": []}
        })
    }

    #[test]
    fn ordinary_component_wire() -> Result<(), Box<dyn Error>> {
        let native = version("2!1.0rc3.post4.dev5+abc.7");
        assert_eq!(
            serde_json::to_value(round_trip(&native)?)?,
            json!({
                "epoch": "2",
                "release": ["1", "0"],
                "pre": {"kind": "rc", "number": "3"},
                "post": "4",
                "dev": "5",
                "local": {
                    "kind": "segments",
                    "segments": [
                        {"kind": "string", "value": "abc"},
                        {"kind": "number", "value": "7"}
                    ]
                }
            })
        );
        for value in [
            "0",
            "1",
            "1.0.0.0.0",
            "9!2.3a0",
            "1b2.post3",
            "1rc4.dev5",
            "1.post0.dev0",
            "1.dev0+abc123.4",
            "1+18446744073709551615",
            "1+18446744073709551616",
            "1+00018446744073709551616",
        ] {
            round_trip(&version(value))?;
        }
        assert_ne!(
            EncodedVersion::try_from(version("1"))?,
            EncodedVersion::try_from(version("1.0"))?
        );
        Ok(())
    }

    #[test]
    fn full_u64_components_and_numeric_overflow_local_strings() -> Result<(), Box<dyn Error>> {
        let native = Version::new([u64::MAX, 0])
            .with_epoch(u64::MAX)
            .with_pre(Some(Prerelease {
                kind: PrereleaseKind::Rc,
                number: u64::MAX,
            }))
            .with_post(Some(u64::MAX))
            .with_dev(Some(u64::MAX))
            .with_local_segments(vec![
                LocalSegment::Number(u64::MAX),
                LocalSegment::String("18446744073709551616".to_owned()),
            ]);
        assert!(native.to_string().parse::<Version>().is_err());
        let wire = serde_json::to_value(round_trip(&native)?)?;
        for value in [
            &wire["epoch"],
            &wire["release"][0],
            &wire["pre"]["number"],
            &wire["post"],
            &wire["dev"],
            &wire["local"]["segments"][0]["value"],
        ] {
            assert_eq!(value, "18446744073709551615");
        }
        assert_eq!(wire["local"]["segments"][1]["kind"], "string");
        assert_eq!(
            wire["local"]["segments"][1]["value"],
            "18446744073709551616"
        );
        Ok(())
    }

    #[test]
    fn supported_storage_and_ordering() -> Result<(), Box<dyn Error>> {
        let compact = Version::new([1]);
        let full = Version::new([1, 0, 0, 0, 0]).with_release([1]);
        let mut natives = Vec::new();
        for base in [compact, full] {
            natives.extend([
                base.clone(),
                base.clone().with_min(Some(0)),
                base.clone().with_max(Some(0)),
                base.clone().with_local(LocalVersion::Max),
                base.clone().with_dev(Some(0)),
                base.clone().with_post(Some(0)),
                base.clone().with_pre(Some(Prerelease {
                    kind: PrereleaseKind::Alpha,
                    number: 1,
                })),
                base.with_pre(Some(Prerelease {
                    kind: PrereleaseKind::Rc,
                    number: 1,
                }))
                .with_max(Some(0)),
            ]);
        }
        natives.extend([
            version("1.0"),
            version("1a1.post2.dev3+local.4"),
            version("1rc1.post2.dev3").with_local(LocalVersion::Max),
            Version::new([1, u64::MAX]),
            Version::new([1]).with_epoch(u64::MAX),
            Version::new([1]).with_pre(Some(Prerelease {
                kind: PrereleaseKind::Rc,
                number: u64::MAX,
            })),
            Version::new([1]).with_post(Some(u64::MAX)),
            Version::new([1]).with_dev(Some(u64::MAX)),
        ]);
        let decoded = natives
            .iter()
            .map(|native| round_trip(native).map(EncodedVersion::into_version))
            .collect::<Result<Vec<_>, _>>()?;
        for (left, decoded_left) in natives.iter().zip(&decoded) {
            for (right, decoded_right) in natives.iter().zip(&decoded) {
                assert_eq!(left.cmp(right), decoded_left.cmp(decoded_right));
                assert_eq!(left.cmp(right), left.cmp(decoded_right));
                assert_eq!(left.cmp(right), decoded_left.cmp(right));
            }
        }
        Ok(())
    }

    #[test]
    fn unsupported_native_sentinels_are_rejected() {
        let base = Version::new([1]);
        let pre = Prerelease {
            kind: PrereleaseKind::Alpha,
            number: 1,
        };
        for native in [
            base.clone().with_min(Some(1)),
            base.clone().with_max(Some(1)),
            Version::new([1, 0, 0, 0, 0])
                .with_release([1])
                .with_max(Some(2)),
            base.clone().with_min(Some(0)).with_max(Some(0)),
            base.clone().with_min(Some(0)).with_pre(Some(pre)),
            base.clone().with_min(Some(0)).with_post(Some(1)),
            base.clone().with_min(Some(0)).with_dev(Some(1)),
            base.clone().with_min(Some(0)).with_local(LocalVersion::Max),
            base.clone().with_max(Some(0)).with_post(Some(1)),
            base.clone().with_max(Some(0)).with_dev(Some(1)),
            base.with_max(Some(0))
                .with_local_segments(vec![LocalSegment::Number(1)]),
        ] {
            assert_eq!(
                EncodedVersion::try_from(native),
                Err(VersionEncodingError::UnsupportedSentinel)
            );
        }
    }

    #[test]
    fn malformed_component_wire_is_rejected() {
        let mut cases = Vec::new();
        for value in [
            json!(0),
            json!(null),
            json!(""),
            json!("00"),
            json!("+1"),
            json!("-1"),
            json!("1.0"),
            json!("18446744073709551616"),
            json!("\u{ff11}"),
        ] {
            let mut wire = plain_wire();
            wire["epoch"] = value;
            cases.push(wire);
        }
        for value in [json!([]), json!([1]), json!(["01"])] {
            let mut wire = plain_wire();
            wire["release"] = value;
            cases.push(wire);
        }
        for value in [
            json!({"kind": "candidate", "number": "1"}),
            json!({"kind": "rc", "number": "1", "unknown": true}),
            json!({"kind": "alpha", "number": 1}),
        ] {
            let mut wire = plain_wire();
            wire["pre"] = value;
            cases.push(wire);
        }
        for value in [
            json!({"kind": "string", "value": ""}),
            json!({"kind": "string", "value": "ABC"}),
            json!({"kind": "string", "value": "1"}),
            json!({"kind": "string", "value": "0001"}),
            json!({"kind": "string", "value": "a.b"}),
            json!({"kind": "number", "value": "01"}),
            json!({"kind": "number", "value": "18446744073709551616"}),
            json!({"kind": "number", "value": 1}),
            json!({"kind": "other", "value": "a"}),
            json!({"kind": "string", "value": "a", "unknown": true}),
        ] {
            let mut wire = plain_wire();
            wire["local"]["segments"] = json!([value]);
            cases.push(wire);
        }
        for value in [
            json!({"kind": "other"}),
            json!({"kind": "max", "segments": []}),
            json!({"kind": "max", "segments": null}),
            json!({"kind": "segments", "segments": [], "unknown": true}),
        ] {
            let mut wire = plain_wire();
            wire["local"] = value;
            cases.push(wire);
        }
        for (key, value) in [("min", "1"), ("max", "1")] {
            let mut wire = plain_wire();
            wire[key] = json!(value);
            cases.push(wire);
        }
        for (key, value) in [
            ("max", json!("0")),
            ("pre", json!({"kind": "rc", "number": "1"})),
            ("post", json!("1")),
            ("dev", json!("1")),
            ("local", json!({"kind": "max"})),
        ] {
            let mut wire = plain_wire();
            wire["min"] = json!("0");
            wire[key] = value;
            cases.push(wire);
        }
        for (key, value) in [
            ("post", json!("1")),
            ("dev", json!("1")),
            ("local", json!({"kind": "max"})),
        ] {
            let mut wire = plain_wire();
            wire["max"] = json!("0");
            wire[key] = value;
            cases.push(wire);
        }
        let mut unknown = plain_wire();
        unknown["unknown"] = json!(true);
        cases.push(unknown);
        for wire in cases {
            assert!(
                serde_json::from_value::<EncodedVersion>(wire.clone()).is_err(),
                "accepted malformed endpoint: {wire}"
            );
        }
        assert!(
            serde_json::from_str::<EncodedVersion>(
                r#"{"epoch":"0","epoch":"1","release":["1"],"local":{"kind":"segments","segments":[]}}"#
            )
            .is_err()
        );
    }

    #[test]
    fn component_limits_are_checked() -> Result<(), Box<dyn Error>> {
        round_trip(&Version::new(vec![1; MAX_COMPONENTS]))?;
        assert_eq!(
            EncodedVersion::try_from(Version::new(vec![1; MAX_COMPONENTS + 1])),
            Err(VersionEncodingError::TooManyReleaseComponents)
        );
        round_trip(
            &Version::new([1]).with_local_segments(vec![LocalSegment::Number(1); MAX_COMPONENTS]),
        )?;
        assert_eq!(
            EncodedVersion::try_from(
                Version::new([1])
                    .with_local_segments(vec![LocalSegment::Number(1); MAX_COMPONENTS + 1])
            ),
            Err(VersionEncodingError::TooManyLocalComponents)
        );
        round_trip(
            &Version::new([1]).with_local_segments(vec![LocalSegment::String(
                "a".repeat(MAX_LOCAL_SEGMENT_BYTES),
            )]),
        )?;
        assert_eq!(
            EncodedVersion::try_from(Version::new([1]).with_local_segments(vec![
                LocalSegment::String("a".repeat(MAX_LOCAL_SEGMENT_BYTES + 1))
            ])),
            Err(VersionEncodingError::LocalSegmentTooLong)
        );
        let mut release = plain_wire();
        release["release"] = json!(vec!["1"; MAX_COMPONENTS + 1]);
        assert!(serde_json::from_value::<EncodedVersion>(release).is_err());
        let mut local = plain_wire();
        local["local"]["segments"] = json!(vec![
            json!({"kind": "number", "value": "1"});
            MAX_COMPONENTS + 1
        ]);
        assert!(serde_json::from_value::<EncodedVersion>(local).is_err());
        let mut atom = plain_wire();
        atom["local"]["segments"] = json!([{
            "kind": "string", "value": "a".repeat(MAX_LOCAL_SEGMENT_BYTES + 1)
        }]);
        assert!(serde_json::from_value::<EncodedVersion>(atom).is_err());
        Ok(())
    }

    #[test]
    fn bounded_sequence_does_not_deserialize_an_excess_element() {
        static COUNT: AtomicUsize = AtomicUsize::new(0);

        #[derive(Debug)]
        struct Counted;

        impl<'de> Deserialize<'de> for Counted {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                COUNT.fetch_add(1, Ordering::Relaxed);
                u64::deserialize(deserializer)?;
                Ok(Self)
            }
        }

        let deserializer = serde::de::value::SeqDeserializer::<_, serde::de::value::Error>::new(
            [0_u64, 1, 2].into_iter(),
        );
        assert!(BoundedVec::<Counted, 2>::deserialize(deserializer).is_err());
        assert_eq!(COUNT.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn local_payload_fields_can_arrive_before_the_tag() -> Result<(), Box<dyn Error>> {
        let json = r#"{"local":{"segments":[{"value":"18446744073709551616","kind":"string"},{"value":"7","kind":"number"}],"kind":"segments"},"release":["1"],"epoch":"0"}"#;
        let encoded: EncodedVersion = serde_json::from_str(json)?;
        assert_components(
            &encoded.into_version(),
            &version("1+18446744073709551616.7"),
        );

        let segments = serde_json::to_string(&vec![
            json!({"kind": "number", "value": "1"});
            MAX_COMPONENTS + 1
        ])?;
        let json = format!(
            r#"{{"epoch":"0","release":["1"],"local":{{"segments":{segments},"kind":"segments"}}}}"#
        );
        let error = serde_json::from_str::<EncodedVersion>(&json)
            .expect_err("the local component count is bounded before reading its kind");
        assert!(error.to_string().contains("sequence exceeds 256 elements"));
        Ok(())
    }

    #[test]
    fn ordinary_version_serde_remains_text() -> Result<(), Box<dyn Error>> {
        assert_eq!(serde_json::to_string(&version("1.0+abc"))?, r#""1.0+abc""#);
        let sentinel = Version::new([1, 0]).with_min(Some(0));
        assert_eq!(serde_json::to_string(&sentinel)?, r#""1.0""#);
        assert_eq!(serde_json::to_value(round_trip(&sentinel)?)?["min"], "0");
        Ok(())
    }
}
