//! Checked encoding of ordered, non-overlapping native version intervals.

use std::fmt;
use std::ops::Bound;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use version_ranges::Ranges;

use crate::version_encoding::{BoundedVec, Field};
use crate::{EncodedVersion, Version, VersionEncodingError};

const MAX_INTERVALS: usize = 131_072;

/// A checked, component-preserving encoding of native version intervals.
///
/// The intervals must already be ordered, nonempty, and separated by a gap. Deserialization
/// rejects malformed or noncanonical input instead of normalizing it into a different range.
/// Finite bounds use [`EncodedVersion`], not the lossy diagnostic form of a version specifier.
/// Membership comparisons are defined over the supported [`EncodedVersion`] domain, which
/// includes ordinary parsed PEP 440 candidates but excludes unsupported internal sentinel states.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct EncodedVersionRanges(BoundedVec<EncodedInterval, MAX_INTERVALS>);

impl EncodedVersionRanges {
    /// Maximum number of intervals in one encoded range.
    pub const MAX_INTERVALS: usize = MAX_INTERVALS;

    /// Reconstruct the checked native intervals without changing their endpoints.
    pub fn into_ranges(self) -> Ranges<Version> {
        self.0
            .0
            .into_iter()
            .map(|interval| (interval.lower.into_native(), interval.upper.into_native()))
            .collect()
    }

    /// Reconstruct the checked native intervals without consuming the encoding.
    pub fn to_ranges(&self) -> Ranges<Version> {
        self.clone().into_ranges()
    }

    fn validate(&self) -> Result<(), VersionRangesEncodingError> {
        let mut previous_upper = None;
        for (index, interval) in self.0.0.iter().enumerate() {
            let lower = interval.lower.clone().into_native();
            let upper = interval.upper.clone().into_native();
            if !nonempty_interval(&lower, &upper) {
                return Err(VersionRangesEncodingError::EmptyInterval { index });
            }
            if let Some(previous_upper) = &previous_upper
                && !separated_intervals(previous_upper, &lower)
            {
                return Err(VersionRangesEncodingError::NonCanonicalIntervals { index });
            }
            previous_upper = Some(upper);
        }
        Ok(())
    }
}

impl TryFrom<&Ranges<Version>> for EncodedVersionRanges {
    type Error = VersionRangesEncodingError;

    fn try_from(ranges: &Ranges<Version>) -> Result<Self, Self::Error> {
        let count = ranges.iter().take(MAX_INTERVALS + 1).count();
        if count > MAX_INTERVALS {
            return Err(VersionRangesEncodingError::TooManyIntervals);
        }
        let mut intervals = Vec::with_capacity(count);
        for (index, (lower, upper)) in ranges.iter().enumerate() {
            let encode = |bound| {
                EncodedBound::from_native(bound)
                    .map_err(|source| VersionRangesEncodingError::Version { index, source })
            };
            intervals.push(EncodedInterval {
                lower: encode(lower)?,
                upper: encode(upper)?,
            });
        }
        let encoded = Self(BoundedVec(intervals));
        encoded.validate()?;
        Ok(encoded)
    }
}

impl TryFrom<Ranges<Version>> for EncodedVersionRanges {
    type Error = VersionRangesEncodingError;

    fn try_from(ranges: Ranges<Version>) -> Result<Self, Self::Error> {
        Self::try_from(&ranges)
    }
}

impl Serialize for EncodedVersionRanges {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for EncodedVersionRanges {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = Self(BoundedVec::deserialize(deserializer)?);
        encoded.validate().map_err(de::Error::custom)?;
        Ok(encoded)
    }
}

/// A native range cannot be represented by the checked interval encoding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VersionRangesEncodingError {
    /// The range exceeds [`EncodedVersionRanges::MAX_INTERVALS`].
    TooManyIntervals,
    /// One finite endpoint is outside the supported version-encoding domain.
    Version {
        /// Zero-based interval index.
        index: usize,
        /// The endpoint's encoding error.
        source: VersionEncodingError,
    },
    /// The interval is empty or its endpoints are reversed.
    EmptyInterval {
        /// Zero-based interval index.
        index: usize,
    },
    /// The interval is not strictly separated from its predecessor.
    NonCanonicalIntervals {
        /// Zero-based interval index.
        index: usize,
    },
}

impl std::error::Error for VersionRangesEncodingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Version { source, .. } => Some(source),
            Self::TooManyIntervals
            | Self::EmptyInterval { .. }
            | Self::NonCanonicalIntervals { .. } => None,
        }
    }
}

impl fmt::Display for VersionRangesEncodingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyIntervals => formatter.write_str("too many version intervals"),
            Self::Version { index, source } => {
                write!(
                    formatter,
                    "unsupported endpoint in interval {index}: {source}"
                )
            }
            Self::EmptyInterval { index } => {
                write!(formatter, "version interval {index} is empty or reversed")
            }
            Self::NonCanonicalIntervals { index } => {
                write!(
                    formatter,
                    "version interval {index} is not separated from its predecessor"
                )
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EncodedInterval {
    lower: EncodedBound,
    upper: EncodedBound,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize)]
#[serde(tag = "kind", content = "version", rename_all = "snake_case")]
enum EncodedBound {
    Unbounded,
    Included(EncodedVersion),
    Excluded(EncodedVersion),
}

impl<'de> Deserialize<'de> for EncodedBound {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "snake_case")]
        enum Kind {
            Unbounded,
            Included,
            Excluded,
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            kind: Kind,
            #[serde(default)]
            version: Field<EncodedVersion>,
        }

        let Wire { kind, version } = Wire::deserialize(deserializer)?;
        match (kind, version) {
            (Kind::Unbounded, Field::Missing) => Ok(Self::Unbounded),
            (Kind::Included, Field::Present(version)) => Ok(Self::Included(version)),
            (Kind::Excluded, Field::Present(version)) => Ok(Self::Excluded(version)),
            (Kind::Unbounded, Field::Present(_)) => Err(de::Error::custom(
                "unbounded endpoint cannot contain a version",
            )),
            (Kind::Included | Kind::Excluded, Field::Missing) => {
                Err(de::Error::missing_field("version"))
            }
        }
    }
}

impl EncodedBound {
    fn from_native(bound: Bound<&Version>) -> Result<Self, VersionEncodingError> {
        Ok(match bound {
            Bound::Unbounded => Self::Unbounded,
            Bound::Included(version) => Self::Included(EncodedVersion::try_from(version)?),
            Bound::Excluded(version) => Self::Excluded(EncodedVersion::try_from(version)?),
        })
    }

    fn into_native(self) -> Bound<Version> {
        match self {
            Self::Unbounded => Bound::Unbounded,
            Self::Included(version) => Bound::Included(version.into_version()),
            Self::Excluded(version) => Bound::Excluded(version.into_version()),
        }
    }
}

fn nonempty_interval(lower: &Bound<Version>, upper: &Bound<Version>) -> bool {
    match (lower, upper) {
        (Bound::Included(lower), Bound::Included(upper)) => lower <= upper,
        (Bound::Included(lower) | Bound::Excluded(lower), Bound::Excluded(upper))
        | (Bound::Excluded(lower), Bound::Included(upper)) => lower < upper,
        (Bound::Unbounded, _) | (_, Bound::Unbounded) => true,
    }
}

fn separated_intervals(upper: &Bound<Version>, lower: &Bound<Version>) -> bool {
    match (upper, lower) {
        (Bound::Excluded(upper), Bound::Excluded(lower)) => upper <= lower,
        (Bound::Included(upper) | Bound::Excluded(upper), Bound::Included(lower))
        | (Bound::Included(upper), Bound::Excluded(lower)) => upper < lower,
        (Bound::Unbounded, _) | (_, Bound::Unbounded) => false,
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use serde_json::{Value, json};

    use crate::{Prerelease, PrereleaseKind, VersionSpecifier, release_specifier_to_range};

    use super::*;

    fn version(value: &str) -> Version {
        value.parse().expect("valid test version")
    }

    fn finite(kind: &str, value: &str) -> Value {
        json!({
            "kind": kind,
            "version": EncodedVersion::try_from(version(value)).expect("supported test version")
        })
    }

    fn unbounded() -> Value {
        json!({"kind": "unbounded"})
    }

    fn interval(lower: Value, upper: Value) -> Value {
        Value::Object(serde_json::Map::from_iter([
            ("lower".to_owned(), lower),
            ("upper".to_owned(), upper),
        ]))
    }

    fn round_trip(native: &Ranges<Version>, candidates: &[Version]) -> Result<(), Box<dyn Error>> {
        let encoded = EncodedVersionRanges::try_from(native)?;
        let bytes = serde_json::to_vec(&encoded)?;
        let decoded: EncodedVersionRanges = serde_json::from_slice(&bytes)?;
        assert_eq!(encoded, decoded);
        let ranges = decoded.into_ranges();
        assert_eq!(encoded, EncodedVersionRanges::try_from(&ranges)?);
        for candidate in candidates {
            assert_eq!(
                native.contains(candidate),
                ranges.contains(candidate),
                "membership changed for {candidate} in {native:?}"
            );
        }
        Ok(())
    }

    fn ordinary_candidates() -> Vec<Version> {
        [
            "0",
            "0.dev0",
            "1.dev0",
            "1a0",
            "1a1",
            "1a1.post0",
            "1b0",
            "1rc1",
            "1",
            "1.0.0",
            "1+abc",
            "1+1",
            "1+18446744073709551616",
            "1.post0.dev0",
            "1.post0",
            "1.post1",
            "1.1",
            "2",
            "2!1",
        ]
        .into_iter()
        .map(version)
        .collect()
    }

    #[test]
    fn every_bound_kind_and_union_round_trips() -> Result<(), Box<dyn Error>> {
        let candidates = ordinary_candidates();
        round_trip(&Ranges::empty(), &candidates)?;
        round_trip(&Ranges::full(), &candidates)?;
        for lower in [
            Bound::Unbounded,
            Bound::Included(version("1")),
            Bound::Excluded(version("1")),
        ] {
            for upper in [
                Bound::Unbounded,
                Bound::Included(version("2")),
                Bound::Excluded(version("2")),
            ] {
                round_trip(
                    &Ranges::from_range_bounds((lower.clone(), upper)),
                    &candidates,
                )?;
            }
        }
        round_trip(
            &Ranges::from_range_bounds((
                Bound::Included(version("1.0")),
                Bound::Included(version("1.0.0")),
            )),
            &candidates,
        )?;
        round_trip(
            &Ranges::strictly_lower_than(version("1"))
                .union(&Ranges::strictly_higher_than(version("1"))),
            &candidates,
        )?;
        round_trip(
            &Ranges::between(version("0"), version("1"))
                .union(&Ranges::singleton(version("1.1")))
                .union(&Ranges::higher_than(version("2"))),
            &candidates,
        )?;
        Ok(())
    }

    #[test]
    fn specifier_sentinels_retain_membership() -> Result<(), Box<dyn Error>> {
        let candidates = ordinary_candidates();
        for specifier in [
            "==1.0",
            "==1.0+abc",
            "!=1.0",
            "===1.0",
            "<1.0",
            "<1.0a1",
            "<1.0.post1",
            "<1.0.dev1",
            "<=1.0",
            "<=1.0a1.post2.dev3",
            ">1.0",
            ">1.0a1",
            ">1.0.post1",
            ">1.0.dev1",
            ">=1.0",
            "~=1.0",
            "==1.0.*",
            "!=1.0.*",
            "<=2!1.0rc1.post2.dev3",
        ] {
            round_trip(
                &Ranges::from(specifier.parse::<VersionSpecifier>()?),
                &candidates,
            )?;
        }
        Ok(())
    }

    #[test]
    fn checked_upper_bound_maxima_retain_membership() -> Result<(), Box<dyn Error>> {
        let largest_input = u64::MAX - 1;
        let specifiers = [
            VersionSpecifier::equals_star_version(Version::new([1, largest_input])),
            VersionSpecifier::equals_star_version(Version::new([1]).with_pre(Some(Prerelease {
                kind: PrereleaseKind::Rc,
                number: largest_input,
            }))),
            VersionSpecifier::equals_star_version(Version::new([1]).with_post(Some(largest_input))),
            VersionSpecifier::greater_than_version(Version::new([1]).with_dev(Some(largest_input))),
        ];
        let mut candidates = ordinary_candidates();
        for number in [u64::MAX - 2, largest_input, u64::MAX] {
            candidates.extend([
                Version::new([1, number]),
                Version::new([1, number]).with_dev(Some(0)),
                Version::new([1]).with_pre(Some(Prerelease {
                    kind: PrereleaseKind::Rc,
                    number,
                })),
                Version::new([1])
                    .with_pre(Some(Prerelease {
                        kind: PrereleaseKind::Rc,
                        number,
                    }))
                    .with_dev(Some(0)),
                Version::new([1]).with_post(Some(number)),
                Version::new([1]).with_post(Some(number)).with_dev(Some(0)),
                Version::new([1]).with_dev(Some(number)),
            ]);
        }
        for specifier in specifiers {
            round_trip(&Ranges::from(specifier), &candidates)?;
        }
        round_trip(
            &release_specifier_to_range(
                VersionSpecifier::equals_star_version(Version::new([1, largest_input])),
                false,
            ),
            &candidates,
        )?;
        Ok(())
    }

    #[test]
    fn malformed_or_noncanonical_intervals_are_rejected() {
        let mut cases = vec![
            json!([interval(finite("included", "2"), finite("included", "1"))]),
            json!([interval(finite("included", "1"), finite("excluded", "1"))]),
            json!([interval(finite("excluded", "1"), finite("included", "1"))]),
            json!([interval(finite("excluded", "1"), finite("excluded", "1"))]),
            json!([
                interval(finite("included", "1"), finite("included", "3")),
                interval(finite("included", "2"), finite("included", "4"))
            ]),
            json!([
                interval(finite("included", "3"), finite("included", "4")),
                interval(finite("included", "1"), finite("included", "2"))
            ]),
            json!([
                interval(unbounded(), unbounded()),
                interval(finite("included", "1"), finite("included", "2"))
            ]),
            json!([
                interval(finite("included", "1"), finite("included", "2")),
                interval(unbounded(), finite("included", "3"))
            ]),
            json!([{"lower": unbounded()}]),
            json!([{"lower": unbounded(), "upper": unbounded(), "unknown": true}]),
            json!([interval(json!({"kind": "other"}), unbounded())]),
            json!([interval(json!({"kind": "included"}), unbounded())]),
            json!([interval(
                json!({"kind": "included", "version": null}),
                unbounded()
            )]),
            json!([interval(
                json!({"kind": "unbounded", "version": null}),
                unbounded()
            )]),
            json!([interval(finite("unbounded", "1"), unbounded())]),
        ];
        for (upper, lower) in [
            ("included", "included"),
            ("included", "excluded"),
            ("excluded", "included"),
        ] {
            cases.push(json!([
                interval(finite("included", "1"), finite(upper, "2")),
                interval(finite(lower, "2"), finite("included", "3"))
            ]));
        }
        let mut unsupported = finite("included", "1");
        unsupported["version"]["max"] = json!("1");
        cases.push(json!([interval(unsupported, unbounded())]));
        for wire in cases {
            assert!(
                serde_json::from_value::<EncodedVersionRanges>(wire.clone()).is_err(),
                "accepted malformed intervals: {wire}"
            );
        }
    }

    #[test]
    fn excluded_touching_bounds_have_a_real_gap() -> Result<(), Box<dyn Error>> {
        let wire = json!([
            interval(unbounded(), finite("excluded", "1.0")),
            interval(finite("excluded", "1.0.0"), unbounded())
        ]);
        let encoded: EncodedVersionRanges = serde_json::from_value(wire)?;
        let ranges = encoded.to_ranges();
        assert!(!ranges.contains(&version("1")));
        assert!(ranges.contains(&version("1+local")));
        assert_eq!(encoded, EncodedVersionRanges::try_from(&ranges)?);
        Ok(())
    }

    #[test]
    fn unsupported_native_endpoint_is_rejected() {
        let ranges = Ranges::singleton(Version::new([1]).with_max(Some(1)));
        assert_eq!(
            EncodedVersionRanges::try_from(&ranges),
            Err(VersionRangesEncodingError::Version {
                index: 0,
                source: VersionEncodingError::UnsupportedSentinel,
            })
        );
    }

    #[test]
    fn interval_count_is_checked_before_encoding_endpoints() {
        let ranges: Ranges<Version> = (0..=MAX_INTERVALS)
            .map(|index| {
                let value = u64::try_from(index).expect("bounded interval index") * 2;
                let version = Version::new([value]);
                (Bound::Included(version.clone()), Bound::Included(version))
            })
            .collect();
        assert_eq!(ranges.iter().count(), MAX_INTERVALS + 1);
        assert_eq!(
            EncodedVersionRanges::try_from(&ranges),
            Err(VersionRangesEncodingError::TooManyIntervals)
        );
    }

    #[test]
    fn finite_bound_fields_can_arrive_before_the_tag() -> Result<(), Box<dyn Error>> {
        let json = r#"[{"lower":{"version":{"release":["1"],"local":{"segments":[],"kind":"segments"},"epoch":"0"},"kind":"included"},"upper":{"kind":"unbounded"}}]"#;
        let encoded: EncodedVersionRanges = serde_json::from_str(json)?;
        assert_eq!(encoded.to_ranges(), Ranges::higher_than(version("1")));
        Ok(())
    }
}
