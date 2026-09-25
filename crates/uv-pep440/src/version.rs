use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::fmt::Formatter;
use std::num::NonZero;
use std::ops::Deref;
use std::sync::LazyLock;
use std::{
    borrow::Borrow,
    cmp::Ordering,
    hash::{Hash, Hasher},
    str::FromStr,
    sync::Arc,
};
use uv_cache_key::{CacheKey, CacheKeyHasher};

/// A version comparison operator, such as `~=`, `==`, `!=`, `<=`, `>=`, `<`, `>`, or `===`.
#[derive(Eq, Ord, PartialEq, PartialOrd, Debug, Hash, Clone, Copy)]
#[cfg_attr(
    feature = "rkyv",
    derive(rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)
)]
#[cfg_attr(feature = "rkyv", rkyv(derive(Debug, Eq, PartialEq, PartialOrd, Ord)))]
pub enum Operator {
    /// `== 1.2.3`
    Equal,
    /// `== 1.2.*`
    EqualStar,
    /// `===` (discouraged)
    ///
    /// <https://peps.python.org/pep-0440/#arbitrary-equality>
    ///
    /// "Use of this operator is heavily discouraged and tooling MAY display a warning when it is used"
    // Clippy rejects this: #[deprecated = "Use of this operator is heavily discouraged"]
    ExactEqual,
    /// `!= 1.2.3`
    NotEqual,
    /// `!= 1.2.*`
    NotEqualStar,
    /// `~=`
    ///
    /// Invariant: With `~=`, there are always at least 2 release segments.
    TildeEqual,
    /// `<`
    LessThan,
    /// `<=`
    LessThanEqual,
    /// `>`
    GreaterThan,
    /// `>=`
    GreaterThanEqual,
}

impl Operator {
    /// Returns the negation of this operator, if one exists.
    ///
    /// Returns `None` for `~=`, which has no single negated operator. Callers must handle that
    /// negation at a higher level. For example, split a compatible-version constraint into its
    /// component constraints and combine their negations with a disjunction.
    ///
    /// Negation is not always reversible. For example, `Operator::ExactEqual` negates to
    /// `Operator::NotEqual`, which negates to `Operator::Equal`.
    pub fn negate(self) -> Option<Self> {
        Some(match self {
            Self::Equal => Self::NotEqual,
            Self::EqualStar => Self::NotEqualStar,
            Self::ExactEqual => Self::NotEqual,
            Self::NotEqual => Self::Equal,
            Self::NotEqualStar => Self::EqualStar,
            Self::TildeEqual => return None,
            Self::LessThan => Self::GreaterThanEqual,
            Self::LessThanEqual => Self::GreaterThan,
            Self::GreaterThan => Self::LessThanEqual,
            Self::GreaterThanEqual => Self::LessThan,
        })
    }

    /// Returns `true` if this operator accepts a version with a non-empty local segment.
    ///
    /// This follows the version specifier [spec]: "Local version identifiers are
    /// NOT permitted in this version specifier."
    ///
    /// [spec]: https://packaging.python.org/en/latest/specifications/version-specifiers/
    pub(crate) fn is_local_compatible(self) -> bool {
        !matches!(
            self,
            Self::GreaterThan
                | Self::GreaterThanEqual
                | Self::LessThan
                | Self::LessThanEqual
                | Self::TildeEqual
                | Self::EqualStar
                | Self::NotEqualStar
        )
    }

    /// Returns the wildcard form of this operator, if one exists.
    ///
    /// Returns `None` when this operator has no wildcard form.
    pub(crate) fn to_star(self) -> Option<Self> {
        match self {
            Self::Equal => Some(Self::EqualStar),
            Self::NotEqual => Some(Self::NotEqualStar),
            _ => None,
        }
    }

    /// Returns `true` if this operator represents a wildcard.
    pub fn is_star(self) -> bool {
        matches!(self, Self::EqualStar | Self::NotEqualStar)
    }

    /// Returns the string representation of this operator.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Equal => "==",
            // The operator does not include the wildcard.
            Self::EqualStar => "==",
            #[allow(deprecated)]
            Self::ExactEqual => "===",
            Self::NotEqual => "!=",
            Self::NotEqualStar => "!=",
            Self::TildeEqual => "~=",
            Self::LessThan => "<",
            Self::LessThanEqual => "<=",
            Self::GreaterThan => ">",
            Self::GreaterThanEqual => ">=",
        }
    }
}

impl FromStr for Operator {
    type Err = OperatorParseError;

    /// Parses the base operator without any version wildcard.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let operator = match s {
            "==" => Self::Equal,
            "===" => {
                #[cfg(feature = "tracing")]
                {
                    tracing::warn!("Using arbitrary equality (`===`) is discouraged");
                }
                #[allow(deprecated)]
                Self::ExactEqual
            }
            "!=" => Self::NotEqual,
            "~=" => Self::TildeEqual,
            "<" => Self::LessThan,
            "<=" => Self::LessThanEqual,
            ">" => Self::GreaterThan,
            ">=" => Self::GreaterThanEqual,
            other => {
                return Err(OperatorParseError {
                    got: other.to_string(),
                });
            }
        };
        Ok(operator)
    }
}

impl std::fmt::Display for Operator {
    /// Formats `EqualStar` as `==`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let operator = self.as_str();
        write!(f, "{operator}")
    }
}

/// An error that occurs when parsing an invalid version specifier operator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperatorParseError {
    pub(crate) got: String,
}

impl std::error::Error for OperatorParseError {}

impl std::fmt::Display for OperatorParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "no such comparison operator {:?}, must be one of ~= == != <= >= < > ===",
            self.got
        )
    }
}

// NOTE: Measure common version formats to optimize their representation. Use a more complete
// representation for versions outside the common case.
//
// The experiment downloaded PyPI distribution metadata from Google BigQuery and counted versions
// with each property:
//
//     total: 11264078
//     release counts:
//         01: 51204 (0.45%)
//         02: 754520 (6.70%)
//         03: 9757602 (86.63%)
//         04: 527403 (4.68%)
//         05: 77994 (0.69%)
//         06: 91346 (0.81%)
//         07: 1421 (0.01%)
//         08: 205 (0.00%)
//         09: 72 (0.00%)
//         10: 2297 (0.02%)
//         11: 5 (0.00%)
//         12: 2 (0.00%)
//         13: 4 (0.00%)
//         20: 2 (0.00%)
//         39: 1 (0.00%)
//     JUST release counts:
//         01: 48297 (0.43%)
//         02: 604692 (5.37%)
//         03: 8460917 (75.11%)
//         04: 465354 (4.13%)
//         05: 49293 (0.44%)
//         06: 25909 (0.23%)
//         07: 1413 (0.01%)
//         08: 192 (0.00%)
//         09: 72 (0.00%)
//         10: 2292 (0.02%)
//         11: 5 (0.00%)
//         12: 2 (0.00%)
//         13: 4 (0.00%)
//         20: 2 (0.00%)
//         39: 1 (0.00%)
//     non-zero epochs: 1902 (0.02%)
//     pre-releases: 752184 (6.68%)
//     post-releases: 134383 (1.19%)
//     dev-releases: 765099 (6.79%)
//     locals: 1 (0.00%)
//     fitsu8: 10388430 (92.23%)
//     sweetspot: 10236089 (90.87%)
//
// "JUST release counts" includes versions with only a release component. "fitsu8" means every
// number except a local numeric segment fits in `u8`. "sweetspot" means the version has no local
// component, has at most four release segments, and every number fits in `u8`.
//
// Most versions, 75%, use exactly three release components in the `x.y.z` format.
//
// ---AG

/// A version number such as `1.2.3` or `4!5.6.7-a8.post9.dev0`.
///
/// The [`Ord`] and [`Eq`] implementations do not always match PEP 440 comparison operators. A
/// Rust `>` comparison can differ from a [`crate::VersionSpecifier`] that uses `>`.
///
/// Parse with [`Version::from_str`]:
///
/// ```rust
/// use std::str::FromStr;
/// use uv_pep440::Version;
///
/// let version = Version::from_str("1.19").unwrap();
/// ```
#[derive(Clone)]
#[cfg_attr(
    feature = "rkyv",
    derive(rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)
)]
#[cfg_attr(feature = "rkyv", rkyv(derive(Debug, Eq, PartialEq, PartialOrd, Ord)))]
pub struct Version {
    inner: VersionInner,
}

#[derive(Clone, Debug)]
#[cfg_attr(
    feature = "rkyv",
    derive(rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)
)]
#[cfg_attr(feature = "rkyv", rkyv(derive(Debug, Eq, PartialEq, PartialOrd, Ord)))]
enum VersionInner {
    Small { small: VersionSmall },
    Full { full: Arc<VersionFull> },
}

impl Version {
    /// Creates a version from an iterator of release segments.
    ///
    /// # Panics
    ///
    /// When the iterator yields no elements.
    #[inline]
    pub fn new<I, R>(release_numbers: I) -> Self
    where
        I: IntoIterator<Item = R>,
        R: Borrow<u64>,
    {
        Self {
            inner: VersionInner::Small {
                small: VersionSmall::new(),
            },
        }
        .with_release(release_numbers)
    }

    /// Returns `true` for an alpha, beta, release-candidate, or development version.
    #[inline]
    pub fn any_prerelease(&self) -> bool {
        self.is_pre() || self.is_dev()
    }

    /// Returns `true` if this is neither a pre-release nor a development version.
    #[inline]
    pub fn is_stable(&self) -> bool {
        !self.is_pre() && !self.is_dev()
    }

    /// Returns `true` for an alpha, beta, or release-candidate version.
    #[inline]
    pub fn is_pre(&self) -> bool {
        self.pre().is_some()
    }

    /// Returns `true` for a development version.
    #[inline]
    pub fn is_dev(&self) -> bool {
        self.dev().is_some()
    }

    /// Returns `true` for a post-release version.
    #[inline]
    pub fn is_post(&self) -> bool {
        self.post().is_some()
    }

    /// Returns `true` for a local version, such as `1.2.3+localsuffixesareweird`.
    ///
    /// A `true` result guarantees that [`Version::local`] returns a non-empty slice.
    #[inline]
    pub fn is_local(&self) -> bool {
        !self.local().is_empty()
    }

    /// Returns the epoch of this version.
    #[inline]
    pub fn epoch(&self) -> u64 {
        match self.inner {
            VersionInner::Small { ref small } => small.epoch(),
            VersionInner::Full { ref full } => full.epoch,
        }
    }

    /// Returns the release number part of the version.
    #[inline]
    pub fn release(&self) -> Release<'_> {
        let inner = match &self.inner {
            VersionInner::Small { small } => {
                // Extract the version digits.
                // * Bytes 6 and 7 correspond to the first release segment as a `u16`.
                // * Bytes 5, 4 and 3 correspond to the second, third and fourth release
                //   segments, respectively.
                match small.len {
                    0 => ReleaseInner::Small0([]),
                    1 => ReleaseInner::Small1([(small.repr >> 0o60) & 0xFFFF]),
                    2 => ReleaseInner::Small2([
                        (small.repr >> 0o60) & 0xFFFF,
                        (small.repr >> 0o50) & 0xFF,
                    ]),
                    3 => ReleaseInner::Small3([
                        (small.repr >> 0o60) & 0xFFFF,
                        (small.repr >> 0o50) & 0xFF,
                        (small.repr >> 0o40) & 0xFF,
                    ]),
                    4 => ReleaseInner::Small4([
                        (small.repr >> 0o60) & 0xFFFF,
                        (small.repr >> 0o50) & 0xFF,
                        (small.repr >> 0o40) & 0xFF,
                        (small.repr >> 0o30) & 0xFF,
                    ]),
                    _ => unreachable!("{}", small.len),
                }
            }
            VersionInner::Full { full } => ReleaseInner::Full(&full.release),
        };

        Release { inner }
    }

    /// Returns the pre-release part of this version, if it exists.
    #[inline]
    pub fn pre(&self) -> Option<Prerelease> {
        match self.inner {
            VersionInner::Small { ref small } => small.pre(),
            VersionInner::Full { ref full } => full.pre,
        }
    }

    /// Returns the post-release part of this version, if it exists.
    #[inline]
    pub fn post(&self) -> Option<u64> {
        match self.inner {
            VersionInner::Small { ref small } => small.post(),
            VersionInner::Full { ref full } => full.post,
        }
    }

    /// Returns the dev-release part of this version, if it exists.
    #[inline]
    pub fn dev(&self) -> Option<u64> {
        match self.inner {
            VersionInner::Small { ref small } => small.dev(),
            VersionInner::Full { ref full } => full.dev,
        }
    }

    /// Returns the local segments in this version, if any exist.
    #[inline]
    pub fn local(&self) -> LocalVersionSlice<'_> {
        match self.inner {
            VersionInner::Small { ref small } => small.local_slice(),
            VersionInner::Full { ref full } => full.local.as_slice(),
        }
    }

    /// Returns the min-release part of this version, if it exists.
    ///
    /// The internal `min` component does not exist in PEP 440. For example, `1.0min0` sorts before
    /// every other `1.0` version, including `1.0a1` and `1.0dev0`.
    #[inline]
    pub(crate) fn min(&self) -> Option<u64> {
        match self.inner {
            VersionInner::Small { ref small } => small.min(),
            VersionInner::Full { ref full } => full.min,
        }
    }

    /// Returns the max-release part of this version, if it exists.
    ///
    /// The internal `max` component does not exist in PEP 440. For example, `1.0max0` sorts after
    /// every other `1.0` version, including `1.0.post1` and `1.0+local`.
    #[inline]
    pub(crate) fn max(&self) -> Option<u64> {
        match self.inner {
            VersionInner::Small { ref small } => small.max(),
            VersionInner::Full { ref full } => full.max,
        }
    }

    /// Sets the release numbers and returns the updated version.
    ///
    /// Unlike [`Version::new`], this preserves the other version components.
    ///
    /// # Panics
    ///
    /// When the iterator yields no elements.
    #[inline]
    #[must_use]
    pub fn with_release<I, R>(mut self, release_numbers: I) -> Self
    where
        I: IntoIterator<Item = R>,
        R: Borrow<u64>,
    {
        self.clear_release();
        for n in release_numbers {
            self.push_release(*n.borrow());
        }
        assert!(
            !self.release().is_empty(),
            "release must have non-zero size"
        );
        self
    }

    /// Returns the release component at the given precision.
    ///
    /// Preserves the epoch, pads missing release segments with zeros, and removes every other
    /// component. Returns `None` when the precision is zero.
    #[inline]
    #[must_use]
    pub fn only_release_at_precision(&self, precision: usize) -> Option<Self> {
        let release = self
            .release()
            .iter()
            .copied()
            .chain(std::iter::repeat(0))
            .take(precision)
            .collect::<Vec<_>>();
        (!release.is_empty()).then(|| Self::new(release).with_epoch(self.epoch()))
    }

    /// Appends the given number to the release component.
    #[inline]
    fn push_release(&mut self, n: u64) {
        if let VersionInner::Small { small } = &mut self.inner {
            if small.push_release(n) {
                return;
            }
        }
        self.make_full().release.push(n);
    }

    /// Removes every number from the release component.
    ///
    /// Do not expose this empty state because valid versions require at least one release number.
    #[inline]
    fn clear_release(&mut self) {
        match &mut self.inner {
            VersionInner::Small { small } => small.clear_release(),
            VersionInner::Full { full } => {
                Arc::make_mut(full).release.clear();
            }
        }
    }

    /// Sets the epoch and returns the updated version.
    #[inline]
    #[must_use]
    pub(crate) fn with_epoch(mut self, value: u64) -> Self {
        if let VersionInner::Small { small } = &mut self.inner {
            if small.set_epoch(value) {
                return self;
            }
        }
        self.make_full().epoch = value;
        self
    }

    /// Sets the pre-release component and returns the updated version.
    #[inline]
    #[must_use]
    pub fn with_pre(mut self, value: Option<Prerelease>) -> Self {
        if let VersionInner::Small { small } = &mut self.inner {
            if small.set_pre(value) {
                return self;
            }
        }
        self.make_full().pre = value;
        self
    }

    /// Sets the post-release component and returns the updated version.
    #[inline]
    #[must_use]
    pub fn with_post(mut self, value: Option<u64>) -> Self {
        if let VersionInner::Small { small } = &mut self.inner {
            if small.set_post(value) {
                return self;
            }
        }
        self.make_full().post = value;
        self
    }

    /// Sets the development-release component and returns the updated version.
    #[inline]
    #[must_use]
    pub(crate) fn with_dev(mut self, value: Option<u64>) -> Self {
        if let VersionInner::Small { small } = &mut self.inner {
            if small.set_dev(value) {
                return self;
            }
        }
        self.make_full().dev = value;
        self
    }

    /// Sets the local segments and returns the updated version.
    #[inline]
    #[must_use]
    pub(crate) fn with_local_segments(mut self, value: Vec<LocalSegment>) -> Self {
        if value.is_empty() {
            self.without_local()
        } else {
            self.make_full().local = LocalVersion::Segments(value);
            self
        }
    }

    /// Sets the local version and returns the updated version.
    #[inline]
    #[must_use]
    pub(crate) fn with_local(mut self, value: LocalVersion) -> Self {
        match value {
            LocalVersion::Segments(segments) => self.with_local_segments(segments),
            LocalVersion::Max => {
                if let VersionInner::Small { small } = &mut self.inner {
                    if small.set_local(LocalVersion::Max) {
                        return self;
                    }
                }
                self.make_full().local = value;
                self
            }
        }
    }

    /// For PEP 440 specifier matching: "Except where specifically noted below,
    /// local version identifiers MUST NOT be permitted in version specifiers,
    /// and local version labels MUST be ignored entirely when checking if
    /// candidate versions match a given version specifier."
    #[inline]
    #[must_use]
    pub fn without_local(mut self) -> Self {
        if let VersionInner::Small { small } = &mut self.inner {
            if small.set_local(LocalVersion::empty()) {
                return self;
            }
        }
        self.make_full().local = LocalVersion::empty();
        self
    }

    /// Returns the version with only its release component.
    #[inline]
    #[must_use]
    pub fn only_release(&self) -> Self {
        Self::new(self.release().iter().copied())
    }

    /// Returns the version with only its major and minor release segments.
    #[inline]
    #[must_use]
    pub(crate) fn only_minor_release(&self) -> Self {
        Self::new(self.release().iter().take(2).copied())
    }

    /// Returns the release component without trailing zeroes or other version components.
    #[inline]
    #[must_use]
    pub fn only_release_trimmed(&self) -> Self {
        if let Some(last_non_zero) = self.release().iter().rposition(|segment| *segment != 0) {
            if last_non_zero + 1 == self.release().len()
                && self.epoch() == 0
                && self.pre().is_none()
                && self.post().is_none()
                && self.dev().is_none()
                && self.local().is_empty()
                && self.min().is_none()
                && self.max().is_none()
            {
                // Already a trimmed release-only version.
                self.clone()
            } else {
                Self::new(self.release().iter().take(last_non_zero + 1).copied())
            }
        } else {
            // `0` is a valid version.
            Self::new([0])
        }
    }

    /// Returns the version without trailing `.0` release segments.
    ///
    /// # Panics
    ///
    /// When the release is all zero segments.
    #[inline]
    #[must_use]
    pub fn without_trailing_zeros(self) -> Self {
        let mut release = self.release().to_vec();
        while let Some(0) = release.last() {
            release.pop();
        }
        self.with_release(release)
    }

    /// Updates a version component with the given operation.
    pub fn bump(&mut self, bump: BumpCommand) {
        // Version components use this hierarchy:
        //
        //   major > minor > patch > stable > pre > post > dev
        //
        // Updating one component clears every lower component. For example:
        //
        // if you bump `minor`, then clear: patch, pre, post, dev
        // if you bump `pre`, then clear: post, dev
        //
        // Incrementing a missing component sets it to `1`.
        //
        // The `stable` operation has no value. It clears `pre`, `post`, and `dev`.
        let full = self.make_full();

        match bump {
            BumpCommand::BumpRelease { index, value } => {
                // Clear every component below the release.
                full.pre = None;
                full.post = None;
                full.dev = None;

                // Use `max` so `0.2` becomes `0.3`, not `0.3.0`.
                let old_parts = &full.release;
                let len = old_parts.len().max(index + 1);
                let new_release_vec = (0..len)
                    .map(|i| match i.cmp(&index) {
                        // Preserve earlier values or use an implicit `0`.
                        Ordering::Less => old_parts.get(i).copied().unwrap_or(0),
                        // Increment the selected value or an implicit `0`.
                        Ordering::Equal => {
                            value.unwrap_or_else(|| old_parts.get(i).copied().unwrap_or(0) + 1)
                        }
                        // Reset every later value to `0`.
                        Ordering::Greater => 0,
                    })
                    .collect::<Vec<u64>>();
                full.release = new_release_vec;
            }
            BumpCommand::MakeStable => {
                // Clear every component below the release.
                full.pre = None;
                full.post = None;
                full.dev = None;
            }
            BumpCommand::BumpPrerelease { kind, value } => {
                // Clear every component below the pre-release.
                full.post = None;
                full.dev = None;
                if let Some(value) = value {
                    full.pre = Some(Prerelease {
                        kind,
                        number: value,
                    });
                } else {
                    // Increment the matching kind or set it to `1`.
                    if let Some(prerelease) = &mut full.pre
                        && prerelease.kind == kind
                    {
                        prerelease.number += 1;
                        return;
                    }
                    full.pre = Some(Prerelease { kind, number: 1 });
                }
            }
            BumpCommand::BumpPost { value } => {
                // Clear every component below the post-release.
                full.dev = None;
                if let Some(value) = value {
                    full.post = Some(value);
                } else {
                    // Increment the value or set it to `1`.
                    if let Some(post) = &mut full.post {
                        *post += 1;
                    } else {
                        full.post = Some(1);
                    }
                }
            }
            BumpCommand::BumpDev { value } => {
                if let Some(value) = value {
                    full.dev = Some(value);
                } else {
                    // Increment the value or set it to `1`.
                    if let Some(dev) = &mut full.dev {
                        *dev += 1;
                    } else {
                        full.dev = Some(1);
                    }
                }
            }
        }
    }

    /// Sets the minimum-release component and returns the updated version.
    ///
    /// The internal `min` component does not exist in PEP 440. For example, `1.0min0` sorts before
    /// every other `1.0` version, including `1.0a1` and `1.0dev0`.
    #[inline]
    #[must_use]
    pub fn with_min(mut self, value: Option<u64>) -> Self {
        debug_assert!(!self.is_pre(), "min is not allowed on pre-release versions");
        debug_assert!(!self.is_dev(), "min is not allowed on dev versions");
        if let VersionInner::Small { small } = &mut self.inner {
            if small.set_min(value) {
                return self;
            }
        }
        self.make_full().min = value;
        self
    }

    /// Sets the maximum-release component and returns the updated version.
    ///
    /// The internal `max` component does not exist in PEP 440. For example, `1.0max0` sorts after
    /// every other `1.0` version, including `1.0.post1` and `1.0+local`.
    #[inline]
    #[must_use]
    pub fn with_max(mut self, value: Option<u64>) -> Self {
        debug_assert!(
            !self.is_post(),
            "max is not allowed on post-release versions"
        );
        debug_assert!(!self.is_dev(), "max is not allowed on dev versions");
        if let VersionInner::Small { small } = &mut self.inner {
            if small.set_max(value) {
                return self;
            }
        }
        self.make_full().max = value;
        self
    }

    /// Converts this version to its full representation and returns a mutable reference.
    fn make_full(&mut self) -> &mut VersionFull {
        if let VersionInner::Small { ref small } = self.inner {
            let full = VersionFull {
                epoch: small.epoch(),
                release: self.release().to_vec(),
                min: small.min(),
                max: small.max(),
                pre: small.pre(),
                post: small.post(),
                dev: small.dev(),
                local: small.local(),
            };
            *self = Self {
                inner: VersionInner::Full {
                    full: Arc::new(full),
                },
            };
        }
        match &mut self.inner {
            VersionInner::Full { full } => Arc::make_mut(full),
            VersionInner::Small { .. } => unreachable!(),
        }
    }

    /// Compares two versions without relying on their internal representations.
    ///
    /// Uses the public [`Version`] API. Use this slower comparison when either version does not
    /// use the small representation.
    #[cold]
    #[inline(never)]
    fn cmp_slow(&self, other: &Self) -> Ordering {
        match self.epoch().cmp(&other.epoch()) {
            Ordering::Less => {
                return Ordering::Less;
            }
            Ordering::Equal => {}
            Ordering::Greater => {
                return Ordering::Greater;
            }
        }

        match compare_release(&self.release(), &other.release()) {
            Ordering::Less => {
                return Ordering::Less;
            }
            Ordering::Equal => {}
            Ordering::Greater => {
                return Ordering::Greater;
            }
        }

        // The release components match, so compare the remaining components.
        sortable_tuple(self).cmp(&sortable_tuple(other))
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Visitor;

        impl de::Visitor<'_> for Visitor {
            type Value = Version;

            fn expecting(&self, f: &mut Formatter) -> std::fmt::Result {
                f.write_str("a string")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Version::from_str(v).map_err(de::Error::custom)
            }
        }

        deserializer.deserialize_str(Visitor)
    }
}

/// <https://github.com/serde-rs/serde/issues/1316#issue-332908452>
impl Serialize for Version {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

/// Displays the normalized version.
impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.epoch() != 0 {
            write!(f, "{}!", self.epoch())?;
        }
        let release = self.release();
        let mut release_iter = release.iter();
        if let Some(first) = release_iter.next() {
            write!(f, "{first}")?;
            for n in release_iter {
                write!(f, ".{n}")?;
            }
        }

        if let Some(Prerelease { kind, number }) = self.pre() {
            write!(f, "{kind}{number}")?;
        }
        if let Some(post) = self.post() {
            write!(f, ".post{post}")?;
        }
        if let Some(dev) = self.dev() {
            write!(f, ".dev{dev}")?;
        }
        if !self.local().is_empty() {
            match self.local() {
                LocalVersionSlice::Segments(_) => {
                    write!(f, "+{}", self.local())?;
                }
                LocalVersionSlice::Max => {
                    write!(f, "+")?;
                }
            }
        }
        Ok(())
    }
}

impl std::fmt::Debug for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "\"{self}\"")
    }
}

impl PartialEq<Self> for Version {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Version {}

impl Hash for Version {
    /// Ignores trailing zeroes because [`PartialEq`] pads release segments with zeroes.
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.epoch().hash(state);
        // Skip trailing zeroes.
        for i in self.release().iter().rev().skip_while(|x| **x == 0) {
            i.hash(state);
        }
        self.pre().hash(state);
        self.dev().hash(state);
        self.post().hash(state);
        self.local().hash(state);
    }
}

impl CacheKey for Version {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        self.epoch().cache_key(state);

        let release = self.release();
        release.len().cache_key(state);
        for segment in release.iter() {
            segment.cache_key(state);
        }

        if let Some(pre) = self.pre() {
            1u8.cache_key(state);
            match pre.kind {
                PrereleaseKind::Alpha => 0u8.cache_key(state),
                PrereleaseKind::Beta => 1u8.cache_key(state),
                PrereleaseKind::Rc => 2u8.cache_key(state),
            }
            pre.number.cache_key(state);
        } else {
            0u8.cache_key(state);
        }

        if let Some(post) = self.post() {
            1u8.cache_key(state);
            post.cache_key(state);
        } else {
            0u8.cache_key(state);
        }

        if let Some(dev) = self.dev() {
            1u8.cache_key(state);
            dev.cache_key(state);
        } else {
            0u8.cache_key(state);
        }

        self.local().cache_key(state);
    }
}

impl PartialOrd<Self> for Version {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    /// 1.0.dev456 < 1.0a1 < 1.0a2.dev456 < 1.0a12.dev456 < 1.0a12 < 1.0b1.dev456 < 1.0b2
    /// < 1.0b2.post345.dev456 < 1.0b2.post345 < 1.0b2-346 < 1.0c1.dev456 < 1.0c1 < 1.0rc2 < 1.0c3
    /// < 1.0 < 1.0.post456.dev34 < 1.0.post456
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        match (&self.inner, &other.inner) {
            (VersionInner::Small { small: small1 }, VersionInner::Small { small: small2 }) => {
                small1.repr.cmp(&small2.repr)
            }
            _ => self.cmp_slow(other),
        }
    }
}

impl FromStr for Version {
    type Err = VersionParseError;

    /// Parses a version such as `1.19`, `1.0a1`, `1.0+abc.5`, or `1!2012.2`.
    ///
    /// Does not allow wildcard versions.
    fn from_str(version: &str) -> Result<Self, Self::Err> {
        Parser::new(version.as_bytes()).parse()
    }
}

/// An operation that updates a version component.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum BumpCommand {
    /// Increments or sets a release component.
    BumpRelease {
        /// The release component: `0` for major, `1` for minor, or `2` for patch.
        index: usize,
        /// An explicit value. If absent, increments the component.
        value: Option<u64>,
    },
    /// Increments or sets the pre-release component.
    BumpPrerelease {
        /// The pre-release component to update.
        kind: PrereleaseKind,
        /// An explicit value. If absent, increments the component.
        value: Option<u64>,
    },
    /// Updates the version to its stable release.
    MakeStable,
    /// Increments or sets the post-release component.
    BumpPost {
        /// An explicit value. If absent, increments the component.
        value: Option<u64>,
    },
    /// Increments or sets the development component.
    BumpDev {
        /// An explicit value. If absent, increments the component.
        value: Option<u64>,
    },
}

/// A small representation of a version.
///
/// Stores common versions with small numeric components and no local component. The compact
/// layout lets two small versions compare with a simple `memcmp`.
///
/// Setters return `false` when a value does not fit this representation. In that case, convert the
/// version to its full representation before setting the value.
///
/// # Representation
///
/// This representation supports versions that meet every condition below:
///
/// * The epoch must be `0`.
/// * The release must have at most four segments.
/// * The first release segment must fit in a `u16`; every other segment must fit in a `u8`. This
///   supports calendar versions such as `2023.03`.
/// * The version can have *at most* one pre-release, development, or post-release component.
/// * A pre-release value must be less than 64.
/// * A development or post-release value must be less than `u8::MAX`.
/// * The version must have no local segments.
///
/// These constraints balance a compact representation against support for common versions.
/// Resolution compares versions frequently, so this representation uses `u64::cmp` to keep each
/// comparison inexpensive.
///
/// Versions that meet these constraints fit in a `u64` that preserves PEP 440 ordering:
///
/// * Bytes 6 and 7 correspond to the first release segment as a `u16`.
/// * Bytes 5, 4 and 3 correspond to the second, third and fourth release
///   segments, respectively.
/// * Bytes 2, 1 and 0 represent *one* of the following:
///   `min, .devN, aN, bN, rcN, <no suffix>, local, .postN, max`.
///   * The four most significant bits of byte 2 contain a value from 0 through 8. These values
///     represent min, dev, pre-a, pre-b, pre-rc, no suffix, local, post, and max, respectively.
///     The internal `min` value sorts before every development, pre-release, post-release, and
///     final release. The internal `max` value sorts after every post-release and local release.
///     Neither value exists in PEP 440.
///   * The four remaining bits of byte 2 and all bits in bytes 1 and 0 contain the suffix release
///     number. These bits are `0` when no suffix exists.
///
/// Encoding order matters. Suffixes use less significant bits than release numbers, so
/// `1.2.3 < 1.2.3.post4`.
///
/// An earlier representation stored suffixes in separate locations to support versions such as
/// `1.2.3.dev2.post3`. Preserving the correct order was difficult, so this representation stores
/// only one suffix. A previous encoding also incorrectly produced `1.0dev1 > 1.0a1` because it
/// treated the pre-release as absent. Development releases must sort before pre-releases.
///
/// Almost all versions have at most one pre-release, development, or post-release component.
#[derive(Clone, Debug)]
#[cfg_attr(
    feature = "rkyv",
    derive(rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)
)]
#[cfg_attr(feature = "rkyv", rkyv(derive(Debug, Eq, PartialEq, PartialOrd, Ord)))]
struct VersionSmall {
    /// The packed representation described above.
    repr: u64,
    /// The number of segments in the release component.
    ///
    /// PEP 440 considers `1.2` equivalent to `1.2.0.0`. Preserve trailing zeroes when converting
    /// to and from strings, as the full representation does.
    len: u8,
    /// Adds a niche to the aligned type so [`Version`] uses two words instead of three.
    _force_niche: NonZero<u8>,
}

impl VersionSmall {
    // Constants for each suffix kind.
    //
    // These values define suffix ordering. For example, no suffix sorts after a development
    // suffix but before a post-release suffix.
    //
    // `SUFFIX_KIND_MASK` is the maximum suffix value. Adding a suffix beyond this value requires
    // another mask bit, usually taken from the suffix version.
    //
    // NOTE: Changing this bit format requires a cache-version update for every rkyv cache that
    // contains [`Version`], including *at least* the "simple" cache.
    const SUFFIX_MIN: u64 = 0;
    const SUFFIX_DEV: u64 = 1;
    const SUFFIX_PRE_ALPHA: u64 = 2;
    const SUFFIX_PRE_BETA: u64 = 3;
    const SUFFIX_PRE_RC: u64 = 4;
    const SUFFIX_NONE: u64 = 5;
    const SUFFIX_LOCAL: u64 = 6;
    const SUFFIX_POST: u64 = 7;
    const SUFFIX_MAX: u64 = 8;

    // The mask for the release segment bits.
    //
    // NOTE: Changing the number of release mask bits also requires changes to `push_release` and
    // `Parser::parse_fast`.
    const SUFFIX_RELEASE_MASK: u64 = 0xFFFF_FFFF_FF00_0000;
    // The mask for the version suffix.
    const SUFFIX_VERSION_MASK: u64 = 0x000F_FFFF;
    // The number of version suffix bits. Shifting `repr` right by this number moves the suffix
    // kind into the least significant bits.
    const SUFFIX_VERSION_BIT_LEN: u64 = 20;
    // The mask for the suffix kind after shifting past the version bits. Adding a bit usually
    // requires taking one from the suffix version and updating its mask and bit length.
    const SUFFIX_KIND_MASK: u64 = 0b1111;

    #[inline]
    fn new() -> Self {
        Self {
            _force_niche: NonZero::<u8>::MIN,
            repr: Self::SUFFIX_NONE << Self::SUFFIX_VERSION_BIT_LEN,
            len: 0,
        }
    }

    #[inline]
    #[expect(clippy::unused_self)]
    fn epoch(&self) -> u64 {
        0
    }

    #[inline]
    #[expect(clippy::unused_self)]
    fn set_epoch(&mut self, value: u64) -> bool {
        if value != 0 {
            return false;
        }
        true
    }

    #[inline]
    fn clear_release(&mut self) {
        self.repr &= !Self::SUFFIX_RELEASE_MASK;
        self.len = 0;
    }

    #[inline]
    fn push_release(&mut self, n: u64) -> bool {
        if self.len == 0 {
            if n > u64::from(u16::MAX) {
                return false;
            }
            self.repr |= n << 48;
            self.len = 1;
            true
        } else {
            if n > u64::from(u8::MAX) {
                return false;
            }
            if self.len >= 4 {
                return false;
            }
            let shift = 48 - (usize::from(self.len) * 8);
            self.repr |= n << shift;
            self.len += 1;
            true
        }
    }

    #[inline]
    fn post(&self) -> Option<u64> {
        if self.suffix_kind() == Self::SUFFIX_POST {
            Some(self.suffix_version())
        } else {
            None
        }
    }

    #[inline]
    fn set_post(&mut self, value: Option<u64>) -> bool {
        let suffix_kind = self.suffix_kind();
        if !(suffix_kind == Self::SUFFIX_NONE || suffix_kind == Self::SUFFIX_POST) {
            return value.is_none();
        }
        match value {
            None => {
                self.set_suffix_kind(Self::SUFFIX_NONE);
            }
            Some(number) => {
                if number > Self::SUFFIX_VERSION_MASK {
                    return false;
                }
                self.set_suffix_kind(Self::SUFFIX_POST);
                self.set_suffix_version(number);
            }
        }
        true
    }

    #[inline]
    fn pre(&self) -> Option<Prerelease> {
        let (kind, number) = (self.suffix_kind(), self.suffix_version());
        if kind == Self::SUFFIX_PRE_ALPHA {
            Some(Prerelease {
                kind: PrereleaseKind::Alpha,
                number,
            })
        } else if kind == Self::SUFFIX_PRE_BETA {
            Some(Prerelease {
                kind: PrereleaseKind::Beta,
                number,
            })
        } else if kind == Self::SUFFIX_PRE_RC {
            Some(Prerelease {
                kind: PrereleaseKind::Rc,
                number,
            })
        } else {
            None
        }
    }

    #[inline]
    fn set_pre(&mut self, value: Option<Prerelease>) -> bool {
        let suffix_kind = self.suffix_kind();
        if !(suffix_kind == Self::SUFFIX_NONE
            || suffix_kind == Self::SUFFIX_PRE_ALPHA
            || suffix_kind == Self::SUFFIX_PRE_BETA
            || suffix_kind == Self::SUFFIX_PRE_RC)
        {
            return value.is_none();
        }
        match value {
            None => {
                self.set_suffix_kind(Self::SUFFIX_NONE);
            }
            Some(Prerelease { kind, number }) => {
                if number > Self::SUFFIX_VERSION_MASK {
                    return false;
                }
                match kind {
                    PrereleaseKind::Alpha => {
                        self.set_suffix_kind(Self::SUFFIX_PRE_ALPHA);
                    }
                    PrereleaseKind::Beta => {
                        self.set_suffix_kind(Self::SUFFIX_PRE_BETA);
                    }
                    PrereleaseKind::Rc => {
                        self.set_suffix_kind(Self::SUFFIX_PRE_RC);
                    }
                }
                self.set_suffix_version(number);
            }
        }
        true
    }

    #[inline]
    fn dev(&self) -> Option<u64> {
        if self.suffix_kind() == Self::SUFFIX_DEV {
            Some(self.suffix_version())
        } else {
            None
        }
    }

    #[inline]
    fn set_dev(&mut self, value: Option<u64>) -> bool {
        let suffix_kind = self.suffix_kind();
        if !(suffix_kind == Self::SUFFIX_NONE || suffix_kind == Self::SUFFIX_DEV) {
            return value.is_none();
        }
        match value {
            None => {
                self.set_suffix_kind(Self::SUFFIX_NONE);
            }
            Some(number) => {
                if number > Self::SUFFIX_VERSION_MASK {
                    return false;
                }
                self.set_suffix_kind(Self::SUFFIX_DEV);
                self.set_suffix_version(number);
            }
        }
        true
    }

    #[inline]
    fn min(&self) -> Option<u64> {
        if self.suffix_kind() == Self::SUFFIX_MIN {
            Some(self.suffix_version())
        } else {
            None
        }
    }

    #[inline]
    fn set_min(&mut self, value: Option<u64>) -> bool {
        let suffix_kind = self.suffix_kind();
        if !(suffix_kind == Self::SUFFIX_NONE || suffix_kind == Self::SUFFIX_MIN) {
            return value.is_none();
        }
        match value {
            None => {
                self.set_suffix_kind(Self::SUFFIX_NONE);
            }
            Some(number) => {
                if number > Self::SUFFIX_VERSION_MASK {
                    return false;
                }
                self.set_suffix_kind(Self::SUFFIX_MIN);
                self.set_suffix_version(number);
            }
        }
        true
    }

    #[inline]
    fn max(&self) -> Option<u64> {
        if self.suffix_kind() == Self::SUFFIX_MAX {
            Some(self.suffix_version())
        } else {
            None
        }
    }

    #[inline]
    fn set_max(&mut self, value: Option<u64>) -> bool {
        let suffix_kind = self.suffix_kind();
        if !(suffix_kind == Self::SUFFIX_NONE || suffix_kind == Self::SUFFIX_MAX) {
            return value.is_none();
        }
        match value {
            None => {
                self.set_suffix_kind(Self::SUFFIX_NONE);
            }
            Some(number) => {
                if number > Self::SUFFIX_VERSION_MASK {
                    return false;
                }
                self.set_suffix_kind(Self::SUFFIX_MAX);
                self.set_suffix_version(number);
            }
        }
        true
    }

    #[inline]
    fn local(&self) -> LocalVersion {
        if self.suffix_kind() == Self::SUFFIX_LOCAL {
            LocalVersion::Max
        } else {
            LocalVersion::empty()
        }
    }

    #[inline]
    fn local_slice(&self) -> LocalVersionSlice<'_> {
        if self.suffix_kind() == Self::SUFFIX_LOCAL {
            LocalVersionSlice::Max
        } else {
            LocalVersionSlice::empty()
        }
    }

    #[inline]
    fn set_local(&mut self, value: LocalVersion) -> bool {
        let suffix_kind = self.suffix_kind();
        if !(suffix_kind == Self::SUFFIX_NONE || suffix_kind == Self::SUFFIX_LOCAL) {
            return value.is_empty();
        }
        match value {
            LocalVersion::Max => {
                self.set_suffix_kind(Self::SUFFIX_LOCAL);
                true
            }
            LocalVersion::Segments(segments) if segments.is_empty() => {
                self.set_suffix_kind(Self::SUFFIX_NONE);
                true
            }
            LocalVersion::Segments(_) => false,
        }
    }

    #[inline]
    fn suffix_kind(&self) -> u64 {
        let kind = (self.repr >> Self::SUFFIX_VERSION_BIT_LEN) & Self::SUFFIX_KIND_MASK;
        debug_assert!(kind <= Self::SUFFIX_MAX);
        kind
    }

    #[inline]
    fn set_suffix_kind(&mut self, kind: u64) {
        debug_assert!(kind <= Self::SUFFIX_MAX);
        self.repr &= !(Self::SUFFIX_KIND_MASK << Self::SUFFIX_VERSION_BIT_LEN);
        self.repr |= kind << Self::SUFFIX_VERSION_BIT_LEN;
        if kind == Self::SUFFIX_NONE || kind == Self::SUFFIX_LOCAL {
            self.set_suffix_version(0);
        }
    }

    #[inline]
    fn suffix_version(&self) -> u64 {
        self.repr & Self::SUFFIX_VERSION_MASK
    }

    #[inline]
    fn set_suffix_version(&mut self, value: u64) {
        debug_assert!(value <= Self::SUFFIX_VERSION_MASK);
        self.repr &= !Self::SUFFIX_VERSION_MASK;
        self.repr |= value;
    }
}

/// The full representation of a version.
///
/// Supports every possible version. Variable-length data, such as release numbers and local
/// segments, requires additional storage and indirection.
///
/// Most versions fit in the small representation and do not require this form.
#[derive(Clone, Debug)]
#[cfg_attr(
    feature = "rkyv",
    derive(rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)
)]
#[cfg_attr(feature = "rkyv", rkyv(derive(Debug, Eq, PartialEq, PartialOrd, Ord)))]
struct VersionFull {
    /// The [versioning
    /// epoch](https://peps.python.org/pep-0440/#version-epochs). Usually `0`, but can increase
    /// after a versioning scheme changes.
    epoch: u64,
    /// The normal number part of the version (["final
    /// release"](https://peps.python.org/pep-0440/#final-releases)), such as `1.2.3` in
    /// `4!1.2.3-a8.post9.dev1`.
    ///
    /// The [`Operator`] stores any `*` placeholder.
    release: Vec<u64>,
    /// The [prerelease](https://peps.python.org/pep-0440/#pre-releases),
    /// such as an alpha, beta, or release candidate with a number.
    ///
    /// Its presence affects version-range matching because matching usually excludes pre-releases.
    pre: Option<Prerelease>,
    /// The [Post release
    /// version](https://peps.python.org/pep-0440/#post-releases). Higher post-release values sort
    /// after lower values and versions without a post-release.
    post: Option<u64>,
    /// The [developmental
    /// release](https://peps.python.org/pep-0440/#developmental-releases),
    /// if present.
    dev: Option<u64>,
    /// A [local version
    /// identifier](https://peps.python.org/pep-0440/#local-version-identifiers)
    /// such as `+deadbeef` in `1.2.3+deadbeef`.
    ///
    /// > They consist of a normal public version identifier (as defined
    /// > in the previous section), along with an arbitrary “local version
    /// > label”, separated from the public version identifier by a plus.
    /// > Local version labels have no specific semantics assigned, but
    /// > some syntactic restrictions are imposed.
    ///
    /// Local versions can contain period-separated segments, such as `deadbeef.1.2.3`. See
    /// [`LocalSegment`] for their semantics.
    local: LocalVersion,
    /// An internal segment that sorts before every development, pre-release, post-release, and
    /// final release. PEP 440 does not define this segment.
    min: Option<u64>,
    /// An internal segment that sorts after every post-release and local release. PEP 440 does not
    /// define this segment.
    max: Option<u64>,
}

/// A version number pattern.
///
/// A version pattern appears in a [`VersionSpecifier`](crate::VersionSpecifier). Unlike a version,
/// it can end with a `*` wildcard. The wildcard matches every version with the same prefix.
///
/// A [`VersionPattern`] cannot match versions by itself. Combine it with an [`Operator`] to create
/// a [`VersionSpecifier`](crate::VersionSpecifier).
///
/// Examples:
///
/// * `1.2.3` -> verbatim pattern
/// * `1.2.3.*` -> wildcard pattern
/// * `1.2.*.4` -> invalid
/// * `1.0-dev1.*` -> invalid
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct VersionPattern {
    version: Version,
    wildcard: bool,
}

impl VersionPattern {
    /// Creates a verbatim pattern that matches the given version exactly.
    #[inline]
    pub fn verbatim(version: Version) -> Self {
        Self {
            version,
            wildcard: false,
        }
    }

    /// Creates a wildcard pattern that matches every version with the given prefix.
    #[inline]
    pub fn wildcard(version: Version) -> Self {
        Self {
            version,
            wildcard: true,
        }
    }

    /// Returns the underlying version.
    #[inline]
    pub fn version(&self) -> &Version {
        &self.version
    }

    /// Consumes this pattern and returns ownership of the underlying version.
    #[inline]
    pub(crate) fn into_version(self) -> Version {
        self.version
    }

    /// Returns `true` if this pattern contains a wildcard.
    #[inline]
    pub(crate) fn is_wildcard(&self) -> bool {
        self.wildcard
    }
}

impl FromStr for VersionPattern {
    type Err = VersionPatternParseError;

    fn from_str(version: &str) -> Result<Self, VersionPatternParseError> {
        Parser::new(version.as_bytes()).parse_pattern()
    }
}

/// Release digits of a [`Version`].
///
/// Provides `&[u64]` access even when the release digits use a compressed representation.
pub struct Release<'a> {
    inner: ReleaseInner<'a>,
}

enum ReleaseInner<'a> {
    // Unpack small versions into at most four `u64` values on the stack. This avoids a heap
    // allocation during the release call.
    Small0([u64; 0]),
    Small1([u64; 1]),
    Small2([u64; 2]),
    Small3([u64; 3]),
    Small4([u64; 4]),
    Full(&'a [u64]),
}

impl Deref for Release<'_> {
    type Target = [u64];

    fn deref(&self) -> &Self::Target {
        match &self.inner {
            ReleaseInner::Small0(v) => v,
            ReleaseInner::Small1(v) => v,
            ReleaseInner::Small2(v) => v,
            ReleaseInner::Small3(v) => v,
            ReleaseInner::Small4(v) => v,
            ReleaseInner::Full(v) => v,
        }
    }
}

/// An optional pre-release modifier and number applied to a version.
#[derive(PartialEq, Eq, Debug, Hash, Clone, Copy, Ord, PartialOrd)]
#[cfg_attr(
    feature = "rkyv",
    derive(rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)
)]
#[cfg_attr(feature = "rkyv", rkyv(derive(Debug, Eq, PartialEq, PartialOrd, Ord)))]
pub struct Prerelease {
    /// The kind of pre-release.
    pub kind: PrereleaseKind,
    /// The number associated with the pre-release.
    pub number: u64,
}

/// A pre-release modifier: alpha, beta, or release candidate.
///
/// <https://peps.python.org/pep-0440/#pre-releases>
#[derive(PartialEq, Eq, Debug, Hash, Clone, Copy, Ord, PartialOrd)]
#[cfg_attr(
    feature = "rkyv",
    derive(rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)
)]
#[cfg_attr(feature = "rkyv", rkyv(derive(Debug, Eq, PartialEq, PartialOrd, Ord)))]
pub enum PrereleaseKind {
    /// An alpha pre-release.
    Alpha,
    /// A beta pre-release.
    Beta,
    /// A release-candidate pre-release.
    Rc,
}

impl std::fmt::Display for PrereleaseKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Alpha => write!(f, "a"),
            Self::Beta => write!(f, "b"),
            Self::Rc => write!(f, "rc"),
        }
    }
}

impl std::fmt::Display for Prerelease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", self.kind, self.number)
    }
}

/// Either local version segments or [`LocalVersion::Max`], an internal value that sorts after
/// every other local version.
#[derive(Eq, PartialEq, Debug, Clone, Hash)]
#[cfg_attr(
    feature = "rkyv",
    derive(rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)
)]
#[cfg_attr(feature = "rkyv", rkyv(derive(Debug, Eq, PartialEq, PartialOrd, Ord)))]
pub enum LocalVersion {
    /// A sequence of local segments.
    Segments(Vec<LocalSegment>),
    /// An internal value that sorts after every other local version.
    Max,
}

/// A [`LocalVersion`] that stores its segments as a slice.
#[derive(Eq, PartialEq, Debug, Clone, Hash)]
pub enum LocalVersionSlice<'a> {
    /// The slice form of [`LocalVersion::Segments`].
    Segments(&'a [LocalSegment]),
    /// The slice form of [`LocalVersion::Max`].
    Max,
}

impl LocalVersion {
    /// Returns an empty local version.
    fn empty() -> Self {
        Self::Segments(Vec::new())
    }

    /// Returns `true` if the local version is empty.
    fn is_empty(&self) -> bool {
        match self {
            Self::Segments(segments) => segments.is_empty(),
            Self::Max => false,
        }
    }

    /// Converts the local version segments into a slice.
    fn as_slice(&self) -> LocalVersionSlice<'_> {
        match self {
            Self::Segments(segments) => LocalVersionSlice::Segments(segments),
            Self::Max => LocalVersionSlice::Max,
        }
    }
}

/// Displays the local version identifier.
///
/// [`LocalVersionSlice::Max`] maps to `"[max]"`. A valid local version cannot contain `[` or `]`.
impl std::fmt::Display for LocalVersionSlice<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Segments(segments) => {
                for (i, segment) in segments.iter().enumerate() {
                    if i > 0 {
                        write!(f, ".")?;
                    }
                    write!(f, "{segment}")?;
                }
                Ok(())
            }
            Self::Max => write!(f, "[max]"),
        }
    }
}

impl CacheKey for LocalVersionSlice<'_> {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        match self {
            Self::Segments(segments) => {
                0u8.cache_key(state);
                segments.len().cache_key(state);
                for segment in *segments {
                    segment.cache_key(state);
                }
            }
            Self::Max => {
                1u8.cache_key(state);
            }
        }
    }
}

impl PartialOrd for LocalVersionSlice<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for LocalVersionSlice<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (LocalVersionSlice::Segments(lv1), LocalVersionSlice::Segments(lv2)) => lv1.cmp(lv2),
            (LocalVersionSlice::Segments(_), LocalVersionSlice::Max) => Ordering::Less,
            (LocalVersionSlice::Max, LocalVersionSlice::Segments(_)) => Ordering::Greater,
            (LocalVersionSlice::Max, LocalVersionSlice::Max) => Ordering::Equal,
        }
    }
}

impl LocalVersionSlice<'_> {
    /// Returns an empty local version.
    const fn empty() -> Self {
        Self::Segments(&[])
    }

    /// Returns `true` if the local version is empty.
    pub fn is_empty(&self) -> bool {
        matches!(self, &Self::Segments(&[]))
    }
}

/// A segment of a [local version identifier](<https://peps.python.org/pep-0440/#local-version-identifiers>).
///
/// PEP 440 defines local version ordering as follows:
///
/// > Comparison and ordering of local versions considers each segment of the local version
/// > (divided by a .) separately. If a segment consists entirely of ASCII digits then that section
/// > should be considered an integer for comparison purposes and if a segment contains any ASCII
/// > letters then that segment is compared lexicographically with case insensitivity. When
/// > comparing a numeric and lexicographic segment, the numeric section always compares as greater
/// > than the lexicographic segment. Additionally, a local version with a great number of segments
/// > will always compare as greater than a local version with fewer segments, as long as the
/// > shorter local version’s segments match the beginning of the longer local version’s segments
/// > exactly.
///
/// The default [`Ord`] implementation for `Vec<LocalSegment>` matches these PEP 440 rules.
#[derive(Eq, PartialEq, Debug, Clone, Hash)]
#[cfg_attr(
    feature = "rkyv",
    derive(rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)
)]
#[cfg_attr(feature = "rkyv", rkyv(derive(Debug, Eq, PartialEq, PartialOrd, Ord)))]
pub enum LocalSegment {
    /// A local version segment that cannot be parsed as an integer.
    String(String),
    /// A local version segment parsed as an integer.
    Number(u64),
}

impl std::fmt::Display for LocalSegment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::String(string) => write!(f, "{string}"),
            Self::Number(number) => write!(f, "{number}"),
        }
    }
}

impl CacheKey for LocalSegment {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        match self {
            Self::String(string) => {
                0u8.cache_key(state);
                string.cache_key(state);
            }
            Self::Number(number) => {
                1u8.cache_key(state);
                number.cache_key(state);
            }
        }
    }
}

impl PartialOrd for LocalSegment {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for LocalSegment {
    fn cmp(&self, other: &Self) -> Ordering {
        // <https://peps.python.org/pep-0440/#local-version-identifiers>
        match (self, other) {
            (Self::Number(n1), Self::Number(n2)) => n1.cmp(n2),
            (Self::String(s1), Self::String(s2)) => s1.cmp(s2),
            (Self::Number(_), Self::String(_)) => Ordering::Greater,
            (Self::String(_), Self::Number(_)) => Ordering::Less,
        }
    }
}

/// The state for [parsing a version][pep440].
///
/// Accepts the flexible version format from the PEP 440 normalization rules.
///
/// Also parses version patterns with a trailing wildcard, such as `1.2.*`.
///
/// [pep440]: https://packaging.python.org/en/latest/specifications/version-specifiers/
#[derive(Debug)]
struct Parser<'a> {
    /// The version string to parse.
    v: &'a [u8],
    /// The current position of the parser.
    i: usize,
    /// The epoch extracted from the version.
    epoch: u64,
    /// The release numbers extracted from the version.
    release: ReleaseNumbers,
    /// The pre-release version, if any.
    pre: Option<Prerelease>,
    /// The post-release version, if any.
    post: Option<u64>,
    /// The development release, if any.
    dev: Option<u64>,
    /// The local segments, if any.
    local: Vec<LocalSegment>,
    /// Whether the version ends with a wildcard.
    ///
    /// Valid only while parsing a version pattern.
    wildcard: bool,
}

impl<'a> Parser<'a> {
    /// Separators allowed in multiple version components.
    #[expect(clippy::byte_char_slices)]
    const SEPARATOR: ByteSet = ByteSet::new(&[b'.', b'_', b'-']);

    /// Creates a [`Parser`] for the given version byte string.
    fn new(version: &'a [u8]) -> Self {
        Parser {
            v: version,
            i: 0,
            epoch: 0,
            release: ReleaseNumbers::new(),
            pre: None,
            post: None,
            dev: None,
            local: vec![],
            wildcard: false,
        }
    }

    /// Parses a verbatim version.
    ///
    /// Returns an error for a version pattern.
    fn parse(self) -> Result<Version, VersionParseError> {
        match self.parse_pattern() {
            Ok(vpat) => {
                if vpat.is_wildcard() {
                    Err(ErrorKind::Wildcard.into())
                } else {
                    Ok(vpat.into_version())
                }
            }
            // Preserve version parsing errors. Convert pattern-specific errors to the generic
            // wildcard error because this method expects a verbatim version.
            Err(err) => match *err.kind {
                PatternErrorKind::Version(err) => Err(err),
                PatternErrorKind::WildcardNotTrailing => Err(ErrorKind::Wildcard.into()),
            },
        }
    }

    /// Parses a version pattern, which can also be a verbatim version.
    fn parse_pattern(mut self) -> Result<VersionPattern, VersionPatternParseError> {
        if let Some(vpat) = self.parse_fast() {
            return Ok(vpat);
        }
        self.bump_while(|byte| byte.is_ascii_whitespace());
        self.bump_if("v");
        self.parse_epoch_and_initial_release()?;
        self.parse_rest_of_release()?;
        if self.parse_wildcard()? {
            return Ok(self.into_pattern());
        }
        self.parse_pre()?;
        self.parse_post()?;
        self.parse_dev()?;
        self.parse_local()?;
        self.bump_while(|byte| byte.is_ascii_whitespace());
        if !self.is_done() {
            let version = String::from_utf8_lossy(&self.v[..self.i]).into_owned();
            let remaining = String::from_utf8_lossy(&self.v[self.i..]).into_owned();
            return Err(ErrorKind::UnexpectedEnd { version, remaining }.into());
        }
        Ok(self.into_pattern())
    }

    /// Attempts to parse a common numeric version without the general parser.
    ///
    /// Parses versions in the `w[.x[.y[.z]]]` format. Most version strings use this format, which
    /// avoids most of the work in the general parser.
    ///
    /// Returns `None` when the version does not match that format.
    fn parse_fast(&self) -> Option<VersionPattern> {
        if let [major, b'.', minor, b'.', patch] = self.v {
            let major = major.wrapping_sub(b'0');
            let minor = minor.wrapping_sub(b'0');
            let patch = patch.wrapping_sub(b'0');
            if major <= 9 && minor <= 9 && patch <= 9 {
                return Some(Self::from_fast_release([major, minor, patch, 0], 3));
            }
        }

        let (mut prev_digit, mut cur, mut release, mut len) = (false, 0u8, [0u8; 4], 0u8);
        for &byte in self.v {
            if byte == b'.' {
                if !prev_digit {
                    return None;
                }
                prev_digit = false;
                *release.get_mut(usize::from(len))? = cur;
                len += 1;
                cur = 0;
            } else {
                let digit = byte.checked_sub(b'0')?;
                if digit > 9 {
                    return None;
                }
                prev_digit = true;
                cur = cur.checked_mul(10)?.checked_add(digit)?;
            }
        }
        if !prev_digit {
            return None;
        }
        *release.get_mut(usize::from(len))? = cur;
        len += 1;
        Some(Self::from_fast_release(release, len))
    }

    /// Builds the packed representation used by the numeric fast parser.
    fn from_fast_release(release: [u8; 4], len: u8) -> VersionPattern {
        let small = VersionSmall {
            _force_niche: NonZero::<u8>::MIN,
            repr: (u64::from(release[0]) << 48)
                | (u64::from(release[1]) << 40)
                | (u64::from(release[2]) << 32)
                | (u64::from(release[3]) << 24)
                | (VersionSmall::SUFFIX_NONE << VersionSmall::SUFFIX_VERSION_BIT_LEN),

            len,
        };
        let inner = VersionInner::Small { small };
        let version = Version { inner };
        VersionPattern {
            version,
            wildcard: false,
        }
    }

    /// Parses an optional epoch and the first release component.
    ///
    /// Returns an error if the version does not start with a number. On success, the release
    /// contains one number, and the parser points to the next component or the end of input.
    fn parse_epoch_and_initial_release(&mut self) -> Result<(), VersionPatternParseError> {
        let first_number = self.parse_number()?.ok_or(ErrorKind::NoLeadingNumber)?;
        let first_release_number = if self.bump_if("!") {
            self.epoch = first_number;
            self.parse_number()?
                .ok_or(ErrorKind::NoLeadingReleaseNumber)?
        } else {
            first_number
        };
        self.release.push(first_release_number);
        Ok(())
    }

    /// Parses the remaining release numbers.
    ///
    /// Stops after the last release number. The next character can be a component separator, such
    /// as the second dot in `1.2.*`, `1.2.a5`, or `1.2.dev5`, or the end of input.
    ///
    /// Call this after parsing the optional epoch and first release number.
    fn parse_rest_of_release(&mut self) -> Result<(), VersionPatternParseError> {
        while self.bump_if(".") {
            let Some(n) = self.parse_number()? else {
                self.unbump();
                break;
            };
            self.release.push(n);
        }
        Ok(())
    }

    /// Parses an optional trailing wildcard after the release numbers.
    ///
    /// Returns `true` when the input ends with `.*`. Returns `false` without moving the parser
    /// when no wildcard exists. Returns an error if input follows the wildcard.
    ///
    /// Call this immediately after parsing every release number.
    fn parse_wildcard(&mut self) -> Result<bool, VersionPatternParseError> {
        if !self.bump_if(".*") {
            return Ok(false);
        }
        if !self.is_done() {
            return Err(PatternErrorKind::WildcardNotTrailing.into());
        }
        self.wildcard = true;
        Ok(true)
    }

    /// Parses the pre-release component of a version.
    ///
    /// If present, sets `self.pre` and advances past the pre-release. Otherwise, leaves the parser
    /// unchanged.
    fn parse_pre(&mut self) -> Result<(), VersionPatternParseError> {
        // `SPELLINGS` and `MAP` share the same order. Use the matching spelling index to find the
        // pre-release kind.
        //
        // Spelling order matters because strings match in sequence. For example, `preview` must
        // appear before `pre`.
        const SPELLINGS: StringSet =
            StringSet::new(&["alpha", "beta", "preview", "pre", "rc", "a", "b", "c"]);
        const MAP: &[PrereleaseKind] = &[
            PrereleaseKind::Alpha,
            PrereleaseKind::Beta,
            PrereleaseKind::Rc,
            PrereleaseKind::Rc,
            PrereleaseKind::Rc,
            PrereleaseKind::Alpha,
            PrereleaseKind::Beta,
            PrereleaseKind::Rc,
        ];

        let oldpos = self.i;
        self.bump_if_byte_set(&Parser::SEPARATOR);
        let Some(spelling) = self.bump_if_string_set(&SPELLINGS) else {
            // An optional separator can precede a different component. Restore the parser so the
            // caller can try the next component.
            self.reset(oldpos);
            return Ok(());
        };
        let kind = MAP[spelling];
        self.bump_if_byte_set(&Parser::SEPARATOR);
        // Normalization defaults a missing pre-release number to `0`.
        let number = self.parse_number()?.unwrap_or(0);
        self.pre = Some(Prerelease { kind, number });
        Ok(())
    }

    /// Parses the post-release component of a version.
    ///
    /// If present, sets `self.post` and advances past the post-release. Otherwise, leaves the
    /// parser unchanged.
    fn parse_post(&mut self) -> Result<(), VersionPatternParseError> {
        const SPELLINGS: StringSet = StringSet::new(&["post", "rev", "r"]);

        let oldpos = self.i;
        if self.bump_if("-") {
            if let Some(n) = self.parse_number()? {
                self.post = Some(n);
                return Ok(());
            }
            self.reset(oldpos);
        }
        self.bump_if_byte_set(&Parser::SEPARATOR);
        if self.bump_if_string_set(&SPELLINGS).is_none() {
            // Post-releases are optional. Restore the parser when no post-release spelling matches.
            self.reset(oldpos);
            return Ok(());
        }
        self.bump_if_byte_set(&Parser::SEPARATOR);
        // Normalization defaults a missing post-release number to `0`.
        self.post = Some(self.parse_number()?.unwrap_or(0));
        Ok(())
    }

    /// Parses the development-release component of a version.
    ///
    /// If present, sets `self.dev` and advances past the development release. Otherwise, leaves
    /// the parser unchanged.
    fn parse_dev(&mut self) -> Result<(), VersionPatternParseError> {
        let oldpos = self.i;
        self.bump_if_byte_set(&Parser::SEPARATOR);
        if !self.bump_if("dev") {
            // Development releases are optional. Restore the parser when `dev` does not match.
            self.reset(oldpos);
            return Ok(());
        }
        self.bump_if_byte_set(&Parser::SEPARATOR);
        // Normalization defaults a missing development-release number to `0`.
        self.dev = Some(self.parse_number()?.unwrap_or(0));
        Ok(())
    }

    /// Parses the local component of a version.
    ///
    /// If present, updates `self.local` and advances past the local component. Otherwise, leaves
    /// the parser unchanged. A local component must be the final version component.
    fn parse_local(&mut self) -> Result<(), VersionPatternParseError> {
        if !self.bump_if("+") {
            return Ok(());
        }
        let mut precursor = '+';
        loop {
            let first = self.bump_while(|byte| byte.is_ascii_alphanumeric());
            if first.is_empty() {
                return Err(ErrorKind::LocalEmpty { precursor }.into());
            }
            self.local.push(if let Ok(number) = parse_u64(first) {
                LocalSegment::Number(number)
            } else {
                let string = String::from_utf8(first.to_ascii_lowercase())
                    .expect("ASCII alphanumerics are always valid UTF-8");
                LocalSegment::String(string)
            });
            let Some(byte) = self.bump_if_byte_set(&Parser::SEPARATOR) else {
                break;
            };
            precursor = char::from(byte);
        }
        Ok(())
    }

    /// Consumes consecutive ASCII digits and parses them as a decimal number.
    ///
    /// Returns `Ok(None)` when no digits exist. Returns an error when the number does not fit in a
    /// `u64`.
    fn parse_number(&mut self) -> Result<Option<u64>, VersionPatternParseError> {
        let digits = self.bump_while(|ch| ch.is_ascii_digit());
        if digits.is_empty() {
            return Ok(None);
        }
        let n = parse_u64(digits)?;
        // Reject `u64::MAX` to prevent overflow when downstream code computes `segment + 1`, such
        // as `~=` upper bounds, `==*` upper bounds, and `python_version` marker algebra. This
        // applies to release, epoch, and pre/post/dev segments, but not local segments.
        if n == u64::MAX {
            return Err(ErrorKind::NumberTooBig {
                bytes: digits.to_vec(),
            }
            .into());
        }
        Ok(Some(n))
    }

    /// Converts the current parser state into a [`VersionPattern`].
    ///
    /// # Panics
    ///
    /// When `self.release` is empty. A valid version requires at least one release component.
    fn into_pattern(self) -> VersionPattern {
        assert!(
            self.release.len() > 0,
            "version with no release numbers is invalid"
        );
        let version = Version::new(self.release.as_slice())
            .with_epoch(self.epoch)
            .with_pre(self.pre)
            .with_post(self.post)
            .with_dev(self.dev)
            .with_local(LocalVersion::Segments(self.local));
        VersionPattern {
            version,
            wildcard: self.wildcard,
        }
    }

    /// Consumes and returns input while the given predicate returns `true`.
    ///
    /// Stops at the first byte that fails the predicate or at the end of the input.
    fn bump_while(&mut self, mut predicate: impl FnMut(u8) -> bool) -> &'a [u8] {
        let start = self.i;
        while !self.is_done() && predicate(self.byte()) {
            self.i = self.i.saturating_add(1);
        }
        &self.v[start..self.i]
    }

    /// Consumes the given string if it matches the input at the current position.
    ///
    /// Returns `true` when the string matches. Otherwise, leaves the parser unchanged.
    fn bump_if(&mut self, string: &str) -> bool {
        if self.is_done() {
            return false;
        }
        if starts_with_ignore_ascii_case(string.as_bytes(), &self.v[self.i..]) {
            self.i = self
                .i
                .checked_add(string.len())
                .expect("valid offset because of prefix");
            true
        } else {
            false
        }
    }

    /// Consumes the first matching string from the ordered set and returns its index.
    fn bump_if_string_set(&mut self, set: &StringSet) -> Option<usize> {
        let index = set.starts_with(&self.v[self.i..])?;
        let found = &set.strings[index];
        self.i = self
            .i
            .checked_add(found.len())
            .expect("valid offset because of prefix");
        Some(index)
    }

    /// Consumes and returns the current byte if it belongs to the given set.
    fn bump_if_byte_set(&mut self, set: &ByteSet) -> Option<u8> {
        let found = set.starts_with(&self.v[self.i..])?;
        self.i = self
            .i
            .checked_add(1)
            .expect("valid offset because of prefix");
        Some(found)
    }

    /// Moves the parser back by one byte.
    ///
    /// Use this when a parsing routine advances past the intended position.
    ///
    /// # Panics
    ///
    /// When the parser is already positioned at the beginning.
    fn unbump(&mut self) {
        self.i = self.i.checked_sub(1).expect("not at beginning of input");
    }

    /// Resets the parser to the given position.
    ///
    /// # Panics
    ///
    /// When `offset` is greater than `self.v.len()`.
    fn reset(&mut self, offset: usize) {
        assert!(offset <= self.v.len());
        self.i = offset;
    }

    /// Returns the byte at the current position of the parser.
    ///
    /// # Panics
    ///
    /// When `Parser::is_done` returns `true`.
    fn byte(&self) -> u8 {
        self.v[self.i]
    }

    /// Returns `true` if no input remains.
    fn is_done(&self) -> bool {
        self.i >= self.v.len()
    }
}

/// Stores the release numbers of a version.
///
/// Avoids heap allocation for more than 90% of parsed versions.
#[derive(Debug)]
enum ReleaseNumbers {
    Inline { numbers: [u64; 4], len: usize },
    Vec(Vec<u64>),
}

impl ReleaseNumbers {
    /// Creates an empty set of release numbers.
    fn new() -> Self {
        Self::Inline {
            numbers: [0; 4],
            len: 0,
        }
    }

    /// Adds a release number and switches to heap storage when the inline capacity is full.
    fn push(&mut self, n: u64) {
        match *self {
            Self::Inline {
                ref mut numbers,
                ref mut len,
            } => {
                assert!(*len <= 4);
                if *len == 4 {
                    let mut numbers = numbers.to_vec();
                    numbers.push(n);
                    *self = Self::Vec(numbers);
                } else {
                    numbers[*len] = n;
                    *len += 1;
                }
            }
            Self::Vec(ref mut numbers) => {
                numbers.push(n);
            }
        }
    }

    /// Returns the number of components in this release component.
    fn len(&self) -> usize {
        self.as_slice().len()
    }

    /// Returns the release components as a slice.
    fn as_slice(&self) -> &[u64] {
        match self {
            Self::Inline { numbers, len } => &numbers[..*len],
            Self::Vec(vec) => vec,
        }
    }
}

/// A set of strings for prefix searches.
///
/// Supports constant construction and case-insensitive ASCII matching.
struct StringSet {
    /// The first byte of each string. Rejects inputs that cannot match any prefix.
    first_byte: ByteSet,
    /// The strings in this set. They are matched in order.
    strings: &'static [&'static str],
}

impl StringSet {
    /// Creates a prefix-search set from the given strings.
    ///
    /// # Panics
    ///
    /// When the number of strings is too big.
    const fn new(strings: &'static [&'static str]) -> Self {
        assert!(
            strings.len() <= 20,
            "only a small number of strings are supported"
        );
        let (mut firsts, mut firsts_len) = ([0u8; 20], 0);
        let mut i = 0;
        while i < strings.len() {
            assert!(
                !strings[i].is_empty(),
                "every string in set should be non-empty",
            );
            firsts[firsts_len] = strings[i].as_bytes()[0];
            firsts_len += 1;
            i += 1;
        }
        let first_byte = ByteSet::new(&firsts);
        Self {
            first_byte,
            strings,
        }
    }

    /// Returns the index of the first string that matches the given input prefix.
    fn starts_with(&self, haystack: &[u8]) -> Option<usize> {
        let first_byte = self.first_byte.starts_with(haystack)?;
        for (i, &string) in self.strings.iter().enumerate() {
            let bytes = string.as_bytes();
            if bytes[0].eq_ignore_ascii_case(&first_byte)
                && starts_with_ignore_ascii_case(bytes, haystack)
            {
                return Some(i);
            }
        }
        None
    }
}

/// A byte set for case-insensitive ASCII searches.
struct ByteSet {
    set: [bool; 256],
}

impl ByteSet {
    /// Creates a search set from the given bytes.
    const fn new(bytes: &[u8]) -> Self {
        let mut set = [false; 256];
        let mut i = 0;
        while i < bytes.len() {
            set[bytes[i].to_ascii_uppercase() as usize] = true;
            set[bytes[i].to_ascii_lowercase() as usize] = true;
            i += 1;
        }
        Self { set }
    }

    /// Returns the first input byte if it belongs to this case-insensitive ASCII set.
    fn starts_with(&self, haystack: &[u8]) -> Option<u8> {
        let byte = *haystack.first()?;
        if self.contains(byte) {
            Some(byte)
        } else {
            None
        }
    }

    /// Returns `true` if the given byte belongs to this set.
    fn contains(&self, byte: u8) -> bool {
        self.set[usize::from(byte)]
    }
}

impl std::fmt::Debug for ByteSet {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let mut set = f.debug_set();
        for byte in 0..=255 {
            if self.contains(byte) {
                set.entry(&char::from(byte));
            }
        }
        set.finish()
    }
}

/// An error that occurs when parsing a [`Version`] string fails.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VersionParseError {
    kind: Box<ErrorKind>,
}

impl std::error::Error for VersionParseError {}

impl std::fmt::Display for VersionParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match *self.kind {
            ErrorKind::Wildcard => write!(f, "wildcards are not allowed in a version"),
            ErrorKind::InvalidDigit { got } if got.is_ascii() => {
                write!(f, "expected ASCII digit, but found {:?}", char::from(got))
            }
            ErrorKind::InvalidDigit { got } => {
                write!(
                    f,
                    "expected ASCII digit, but found non-ASCII byte \\x{got:02X}"
                )
            }
            ErrorKind::NumberTooBig { ref bytes } => {
                let string = match std::str::from_utf8(bytes) {
                    Ok(v) => v,
                    Err(err) => {
                        std::str::from_utf8(&bytes[..err.valid_up_to()]).expect("valid UTF-8")
                    }
                };
                write!(
                    f,
                    "expected number less than or equal to {}, \
                     but number found in {string:?} exceeds it",
                    u64::MAX - 1,
                )
            }
            ErrorKind::NoLeadingNumber => {
                write!(
                    f,
                    "expected version to start with a number, \
                     but no leading ASCII digits were found"
                )
            }
            ErrorKind::NoLeadingReleaseNumber => {
                write!(
                    f,
                    "expected version to have a non-empty release component after an epoch, \
                     but no ASCII digits after the epoch were found"
                )
            }
            ErrorKind::LocalEmpty { precursor } => {
                write!(
                    f,
                    "found a `{precursor}` indicating the start of a local \
                     component in a version, but did not find any alphanumeric \
                     ASCII segment following the `{precursor}`",
                )
            }
            ErrorKind::UnexpectedEnd {
                ref version,
                ref remaining,
            } => {
                write!(
                    f,
                    "after parsing `{version}`, found `{remaining}`, \
                     which is not part of a valid version",
                )
            }
        }
    }
}

/// An error that can occur while parsing a [`Version`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ErrorKind {
    /// A wildcard pattern appears where a verbatim version is required.
    Wildcard,
    /// A non-digit appears where an ASCII digit is required.
    InvalidDigit {
        /// The unexpected byte, which can be non-ASCII.
        got: u8,
    },
    /// A number exceeds the range of a `u64`.
    NumberTooBig {
        /// The number bytes, which can contain invalid digits or invalid UTF-8.
        bytes: Vec<u8>,
    },
    /// A version does not start with a number.
    NoLeadingNumber,
    /// An epoch has no release number after `!`.
    NoLeadingReleaseNumber,
    /// A local version separator has no following alphanumeric ASCII segment.
    LocalEmpty {
        /// The `+` or `[-_.]` separator that requires a non-empty local segment.
        precursor: char,
    },
    /// Unexpected input follows an otherwise valid version.
    UnexpectedEnd {
        /// The parsed version.
        version: String,
        /// The remaining unparsed input.
        remaining: String,
    },
}

impl From<ErrorKind> for VersionParseError {
    fn from(kind: ErrorKind) -> Self {
        Self {
            kind: Box::new(kind),
        }
    }
}

/// An error that occurs when parsing a [`VersionPattern`] string fails.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VersionPatternParseError {
    kind: Box<PatternErrorKind>,
}

impl std::error::Error for VersionPatternParseError {}

impl std::fmt::Display for VersionPatternParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match *self.kind {
            PatternErrorKind::Version(ref err) => err.fmt(f),
            PatternErrorKind::WildcardNotTrailing => {
                write!(f, "wildcards in versions must be at the end")
            }
        }
    }
}

/// An error that can occur while parsing a [`VersionPattern`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PatternErrorKind {
    Version(VersionParseError),
    WildcardNotTrailing,
}

impl From<PatternErrorKind> for VersionPatternParseError {
    fn from(kind: PatternErrorKind) -> Self {
        Self {
            kind: Box::new(kind),
        }
    }
}

impl From<ErrorKind> for VersionPatternParseError {
    fn from(kind: ErrorKind) -> Self {
        Self::from(VersionParseError::from(kind))
    }
}

impl From<VersionParseError> for VersionPatternParseError {
    fn from(err: VersionParseError) -> Self {
        Self {
            kind: Box::new(PatternErrorKind::Version(err)),
        }
    }
}

/// Compares release components, such as `4.3.1 > 4.2`, `1.1.0 == 1.1`, and `1.16 < 1.19`.
pub(crate) fn compare_release(this: &[u64], other: &[u64]) -> Ordering {
    if this.len() == other.len() {
        return this.cmp(other);
    }
    // "When comparing release segments with different numbers of components, the shorter segment
    // is padded out with additional zeros as necessary"
    for (this, other) in this.iter().chain(std::iter::repeat(&0)).zip(
        other
            .iter()
            .chain(std::iter::repeat(&0))
            .take(this.len().max(other.len())),
    ) {
        match this.cmp(other) {
            Ordering::Less => {
                return Ordering::Less;
            }
            Ordering::Equal => {}
            Ordering::Greater => {
                return Ordering::Greater;
            }
        }
    }
    Ordering::Equal
}

/// Orders suffixes when two versions have the same release component.
///
/// The [PEP 440 suffix ordering][pep440-suffix-ordering] is `.devN`, `aN`, `bN`, `rcN`, no
/// suffix, and `.postN`. Development and post-release suffixes can also occur on pre-releases.
/// Represent this with the tuple `({min: 0, dev: 1, a: 2, b: 3, rc: 4, (): 5, post: 6}, <preN>,
/// <postN or None as smallest>, <devN or Max as largest>, <local>)`.
///
/// A post-release number sorts after no post-release. A missing development number sorts after
/// every development number. The default [`Ord`] implementation already orders local segments
/// correctly.
///
/// [pep440-suffix-ordering]: https://peps.python.org/pep-0440/#summary-of-permitted-suffixes-and-relative-ordering
fn sortable_tuple(version: &Version) -> (u64, u64, Option<u64>, u64, LocalVersionSlice<'_>) {
    // For a `max` version, use a post-release value larger than every valid post-release.
    let post = if version.max().is_some() {
        Some(u64::MAX)
    } else {
        version.post()
    };
    match (version.pre(), post, version.dev(), version.min()) {
        // Minimum release.
        (_pre, post, _dev, Some(n)) => (0, 0, post, n, version.local()),
        // Development release.
        (None, None, Some(n), None) => (1, 0, None, n, version.local()),
        // Alpha release.
        (
            Some(Prerelease {
                kind: PrereleaseKind::Alpha,
                number: n,
            }),
            post,
            dev,
            None,
        ) => (2, n, post, dev.unwrap_or(u64::MAX), version.local()),
        // Beta release.
        (
            Some(Prerelease {
                kind: PrereleaseKind::Beta,
                number: n,
            }),
            post,
            dev,
            None,
        ) => (3, n, post, dev.unwrap_or(u64::MAX), version.local()),
        // Release candidate.
        (
            Some(Prerelease {
                kind: PrereleaseKind::Rc,
                number: n,
            }),
            post,
            dev,
            None,
        ) => (4, n, post, dev.unwrap_or(u64::MAX), version.local()),
        // Final release.
        (None, None, None, None) => (5, 0, None, 0, version.local()),
        // Post-release.
        (None, Some(post), dev, None) => {
            (6, 0, Some(post), dev.unwrap_or(u64::MAX), version.local())
        }
    }
}

/// Returns `true` if `needle` is a prefix of `haystack`, ignoring ASCII case.
fn starts_with_ignore_ascii_case(needle: &[u8], haystack: &[u8]) -> bool {
    needle.len() <= haystack.len()
        && std::iter::zip(needle, haystack).all(|(b1, b2)| b1.eq_ignore_ascii_case(b2))
}

/// Parses a `u64` from ASCII digits.
///
/// Returns an error if any byte is not an ASCII digit or the number does not fit in a `u64`.
///
/// # Motivation
///
/// The standard integer parser requires UTF-8 validation and accepts a leading `+`. Version
/// parsing needs neither behavior because it accepts only unsigned ASCII digits.
fn parse_u64(bytes: &[u8]) -> Result<u64, VersionParseError> {
    let mut n: u64 = 0;
    for &byte in bytes {
        let digit = match byte.checked_sub(b'0') {
            None => return Err(ErrorKind::InvalidDigit { got: byte }.into()),
            Some(digit) if digit > 9 => return Err(ErrorKind::InvalidDigit { got: byte }.into()),
            Some(digit) => {
                debug_assert!((0..=9).contains(&digit));
                u64::from(digit)
            }
        };
        n = n
            .checked_mul(10)
            .and_then(|n| n.checked_add(digit))
            .ok_or_else(|| ErrorKind::NumberTooBig {
                bytes: bytes.to_vec(),
            })?;
    }
    Ok(n)
}

/// The minimum version that can be represented by a [`Version`]: `0a0.dev0`.
pub static MIN_VERSION: LazyLock<Version> =
    LazyLock::new(|| Version::from_str("0a0.dev0").unwrap());

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use crate::VersionSpecifier;

    use super::*;

    /// <https://github.com/pypa/packaging/blob/237ff3aa348486cf835a980592af3a59fccd6101/tests/test_version.py#L24-L81>
    #[test]
    fn test_packaging_versions() {
        let versions = [
            // Implicit epoch of 0
            ("1.0.dev456", Version::new([1, 0]).with_dev(Some(456))),
            (
                "1.0a1",
                Version::new([1, 0]).with_pre(Some(Prerelease {
                    kind: PrereleaseKind::Alpha,
                    number: 1,
                })),
            ),
            (
                "1.0a2.dev456",
                Version::new([1, 0])
                    .with_pre(Some(Prerelease {
                        kind: PrereleaseKind::Alpha,
                        number: 2,
                    }))
                    .with_dev(Some(456)),
            ),
            (
                "1.0a12.dev456",
                Version::new([1, 0])
                    .with_pre(Some(Prerelease {
                        kind: PrereleaseKind::Alpha,
                        number: 12,
                    }))
                    .with_dev(Some(456)),
            ),
            (
                "1.0a12",
                Version::new([1, 0]).with_pre(Some(Prerelease {
                    kind: PrereleaseKind::Alpha,
                    number: 12,
                })),
            ),
            (
                "1.0b1.dev456",
                Version::new([1, 0])
                    .with_pre(Some(Prerelease {
                        kind: PrereleaseKind::Beta,
                        number: 1,
                    }))
                    .with_dev(Some(456)),
            ),
            (
                "1.0b2",
                Version::new([1, 0]).with_pre(Some(Prerelease {
                    kind: PrereleaseKind::Beta,
                    number: 2,
                })),
            ),
            (
                "1.0b2.post345.dev456",
                Version::new([1, 0])
                    .with_pre(Some(Prerelease {
                        kind: PrereleaseKind::Beta,
                        number: 2,
                    }))
                    .with_dev(Some(456))
                    .with_post(Some(345)),
            ),
            (
                "1.0b2.post345",
                Version::new([1, 0])
                    .with_pre(Some(Prerelease {
                        kind: PrereleaseKind::Beta,
                        number: 2,
                    }))
                    .with_post(Some(345)),
            ),
            (
                "1.0b2-346",
                Version::new([1, 0])
                    .with_pre(Some(Prerelease {
                        kind: PrereleaseKind::Beta,
                        number: 2,
                    }))
                    .with_post(Some(346)),
            ),
            (
                "1.0c1.dev456",
                Version::new([1, 0])
                    .with_pre(Some(Prerelease {
                        kind: PrereleaseKind::Rc,
                        number: 1,
                    }))
                    .with_dev(Some(456)),
            ),
            (
                "1.0c1",
                Version::new([1, 0]).with_pre(Some(Prerelease {
                    kind: PrereleaseKind::Rc,
                    number: 1,
                })),
            ),
            (
                "1.0rc2",
                Version::new([1, 0]).with_pre(Some(Prerelease {
                    kind: PrereleaseKind::Rc,
                    number: 2,
                })),
            ),
            (
                "1.0c3",
                Version::new([1, 0]).with_pre(Some(Prerelease {
                    kind: PrereleaseKind::Rc,
                    number: 3,
                })),
            ),
            ("1.0", Version::new([1, 0])),
            (
                "1.0.post456.dev34",
                Version::new([1, 0]).with_post(Some(456)).with_dev(Some(34)),
            ),
            ("1.0.post456", Version::new([1, 0]).with_post(Some(456))),
            ("1.1.dev1", Version::new([1, 1]).with_dev(Some(1))),
            (
                "1.2+123abc",
                Version::new([1, 2])
                    .with_local_segments(vec![LocalSegment::String("123abc".to_string())]),
            ),
            (
                "1.2+123abc456",
                Version::new([1, 2])
                    .with_local_segments(vec![LocalSegment::String("123abc456".to_string())]),
            ),
            (
                "1.2+abc",
                Version::new([1, 2])
                    .with_local_segments(vec![LocalSegment::String("abc".to_string())]),
            ),
            (
                "1.2+abc123",
                Version::new([1, 2])
                    .with_local_segments(vec![LocalSegment::String("abc123".to_string())]),
            ),
            (
                "1.2+abc123def",
                Version::new([1, 2])
                    .with_local_segments(vec![LocalSegment::String("abc123def".to_string())]),
            ),
            (
                "1.2+1234.abc",
                Version::new([1, 2]).with_local_segments(vec![
                    LocalSegment::Number(1234),
                    LocalSegment::String("abc".to_string()),
                ]),
            ),
            (
                "1.2+123456",
                Version::new([1, 2]).with_local_segments(vec![LocalSegment::Number(123_456)]),
            ),
            (
                "1.2.r32+123456",
                Version::new([1, 2])
                    .with_post(Some(32))
                    .with_local_segments(vec![LocalSegment::Number(123_456)]),
            ),
            (
                "1.2.rev33+123456",
                Version::new([1, 2])
                    .with_post(Some(33))
                    .with_local_segments(vec![LocalSegment::Number(123_456)]),
            ),
            // Explicit epoch of 1
            (
                "1!1.0.dev456",
                Version::new([1, 0]).with_epoch(1).with_dev(Some(456)),
            ),
            (
                "1!1.0a1",
                Version::new([1, 0])
                    .with_epoch(1)
                    .with_pre(Some(Prerelease {
                        kind: PrereleaseKind::Alpha,
                        number: 1,
                    })),
            ),
            (
                "1!1.0a2.dev456",
                Version::new([1, 0])
                    .with_epoch(1)
                    .with_pre(Some(Prerelease {
                        kind: PrereleaseKind::Alpha,
                        number: 2,
                    }))
                    .with_dev(Some(456)),
            ),
            (
                "1!1.0a12.dev456",
                Version::new([1, 0])
                    .with_epoch(1)
                    .with_pre(Some(Prerelease {
                        kind: PrereleaseKind::Alpha,
                        number: 12,
                    }))
                    .with_dev(Some(456)),
            ),
            (
                "1!1.0a12",
                Version::new([1, 0])
                    .with_epoch(1)
                    .with_pre(Some(Prerelease {
                        kind: PrereleaseKind::Alpha,
                        number: 12,
                    })),
            ),
            (
                "1!1.0b1.dev456",
                Version::new([1, 0])
                    .with_epoch(1)
                    .with_pre(Some(Prerelease {
                        kind: PrereleaseKind::Beta,
                        number: 1,
                    }))
                    .with_dev(Some(456)),
            ),
            (
                "1!1.0b2",
                Version::new([1, 0])
                    .with_epoch(1)
                    .with_pre(Some(Prerelease {
                        kind: PrereleaseKind::Beta,
                        number: 2,
                    })),
            ),
            (
                "1!1.0b2.post345.dev456",
                Version::new([1, 0])
                    .with_epoch(1)
                    .with_pre(Some(Prerelease {
                        kind: PrereleaseKind::Beta,
                        number: 2,
                    }))
                    .with_post(Some(345))
                    .with_dev(Some(456)),
            ),
            (
                "1!1.0b2.post345",
                Version::new([1, 0])
                    .with_epoch(1)
                    .with_pre(Some(Prerelease {
                        kind: PrereleaseKind::Beta,
                        number: 2,
                    }))
                    .with_post(Some(345)),
            ),
            (
                "1!1.0b2-346",
                Version::new([1, 0])
                    .with_epoch(1)
                    .with_pre(Some(Prerelease {
                        kind: PrereleaseKind::Beta,
                        number: 2,
                    }))
                    .with_post(Some(346)),
            ),
            (
                "1!1.0c1.dev456",
                Version::new([1, 0])
                    .with_epoch(1)
                    .with_pre(Some(Prerelease {
                        kind: PrereleaseKind::Rc,
                        number: 1,
                    }))
                    .with_dev(Some(456)),
            ),
            (
                "1!1.0c1",
                Version::new([1, 0])
                    .with_epoch(1)
                    .with_pre(Some(Prerelease {
                        kind: PrereleaseKind::Rc,
                        number: 1,
                    })),
            ),
            (
                "1!1.0rc2",
                Version::new([1, 0])
                    .with_epoch(1)
                    .with_pre(Some(Prerelease {
                        kind: PrereleaseKind::Rc,
                        number: 2,
                    })),
            ),
            (
                "1!1.0c3",
                Version::new([1, 0])
                    .with_epoch(1)
                    .with_pre(Some(Prerelease {
                        kind: PrereleaseKind::Rc,
                        number: 3,
                    })),
            ),
            ("1!1.0", Version::new([1, 0]).with_epoch(1)),
            (
                "1!1.0.post456.dev34",
                Version::new([1, 0])
                    .with_epoch(1)
                    .with_post(Some(456))
                    .with_dev(Some(34)),
            ),
            (
                "1!1.0.post456",
                Version::new([1, 0]).with_epoch(1).with_post(Some(456)),
            ),
            (
                "1!1.1.dev1",
                Version::new([1, 1]).with_epoch(1).with_dev(Some(1)),
            ),
            (
                "1!1.2+123abc",
                Version::new([1, 2])
                    .with_epoch(1)
                    .with_local_segments(vec![LocalSegment::String("123abc".to_string())]),
            ),
            (
                "1!1.2+123abc456",
                Version::new([1, 2])
                    .with_epoch(1)
                    .with_local_segments(vec![LocalSegment::String("123abc456".to_string())]),
            ),
            (
                "1!1.2+abc",
                Version::new([1, 2])
                    .with_epoch(1)
                    .with_local_segments(vec![LocalSegment::String("abc".to_string())]),
            ),
            (
                "1!1.2+abc123",
                Version::new([1, 2])
                    .with_epoch(1)
                    .with_local_segments(vec![LocalSegment::String("abc123".to_string())]),
            ),
            (
                "1!1.2+abc123def",
                Version::new([1, 2])
                    .with_epoch(1)
                    .with_local_segments(vec![LocalSegment::String("abc123def".to_string())]),
            ),
            (
                "1!1.2+1234.abc",
                Version::new([1, 2]).with_epoch(1).with_local_segments(vec![
                    LocalSegment::Number(1234),
                    LocalSegment::String("abc".to_string()),
                ]),
            ),
            (
                "1!1.2+123456",
                Version::new([1, 2])
                    .with_epoch(1)
                    .with_local_segments(vec![LocalSegment::Number(123_456)]),
            ),
            (
                "1!1.2.r32+123456",
                Version::new([1, 2])
                    .with_epoch(1)
                    .with_post(Some(32))
                    .with_local_segments(vec![LocalSegment::Number(123_456)]),
            ),
            (
                "1!1.2.rev33+123456",
                Version::new([1, 2])
                    .with_epoch(1)
                    .with_post(Some(33))
                    .with_local_segments(vec![LocalSegment::Number(123_456)]),
            ),
            (
                "98765!1.2.rev33+123456",
                Version::new([1, 2])
                    .with_epoch(98765)
                    .with_post(Some(33))
                    .with_local_segments(vec![LocalSegment::Number(123_456)]),
            ),
        ];
        for (string, structured) in versions {
            match Version::from_str(string) {
                Err(err) => {
                    unreachable!(
                        "expected {string:?} to parse as {structured:?}, but got {err:?}",
                        structured = structured.as_bloated_debug(),
                    )
                }
                Ok(v) => assert!(
                    v == structured,
                    "for {string:?}, expected {structured:?} but got {v:?}",
                    structured = structured.as_bloated_debug(),
                    v = v.as_bloated_debug(),
                ),
            }
            let spec = format!("=={string}");
            match VersionSpecifier::from_str(&spec) {
                Err(err) => {
                    unreachable!(
                        "expected version in {spec:?} to parse as {structured:?}, but got {err:?}",
                        structured = structured.as_bloated_debug(),
                    )
                }
                Ok(v) => assert!(
                    v.version() == &structured,
                    "for {string:?}, expected {structured:?} but got {v:?}",
                    structured = structured.as_bloated_debug(),
                    v = v.version.as_bloated_debug(),
                ),
            }
        }
    }

    /// <https://github.com/pypa/packaging/blob/237ff3aa348486cf835a980592af3a59fccd6101/tests/test_version.py#L91-L100>
    #[test]
    fn test_packaging_failures() {
        let versions = [
            // Versions with invalid local versions
            "1.0+a+",
            "1.0++",
            "1.0+_foobar",
            "1.0+foo&asd",
            "1.0+1+1",
            // Nonsensical versions should also be invalid
            "french toast",
            "==french toast",
        ];
        for version in versions {
            assert!(Version::from_str(version).is_err());
            assert!(VersionSpecifier::from_str(&format!("=={version}")).is_err());
        }
    }

    #[test]
    fn test_equality_and_normalization() {
        let versions = [
            // Various development release incarnations
            ("1.0dev", "1.0.dev0"),
            ("1.0.dev", "1.0.dev0"),
            ("1.0dev1", "1.0.dev1"),
            ("1.0dev", "1.0.dev0"),
            ("1.0-dev", "1.0.dev0"),
            ("1.0-dev1", "1.0.dev1"),
            ("1.0DEV", "1.0.dev0"),
            ("1.0.DEV", "1.0.dev0"),
            ("1.0DEV1", "1.0.dev1"),
            ("1.0DEV", "1.0.dev0"),
            ("1.0.DEV1", "1.0.dev1"),
            ("1.0-DEV", "1.0.dev0"),
            ("1.0-DEV1", "1.0.dev1"),
            // Various alpha incarnations
            ("1.0a", "1.0a0"),
            ("1.0.a", "1.0a0"),
            ("1.0.a1", "1.0a1"),
            ("1.0-a", "1.0a0"),
            ("1.0-a1", "1.0a1"),
            ("1.0alpha", "1.0a0"),
            ("1.0.alpha", "1.0a0"),
            ("1.0.alpha1", "1.0a1"),
            ("1.0-alpha", "1.0a0"),
            ("1.0-alpha1", "1.0a1"),
            ("1.0A", "1.0a0"),
            ("1.0.A", "1.0a0"),
            ("1.0.A1", "1.0a1"),
            ("1.0-A", "1.0a0"),
            ("1.0-A1", "1.0a1"),
            ("1.0ALPHA", "1.0a0"),
            ("1.0.ALPHA", "1.0a0"),
            ("1.0.ALPHA1", "1.0a1"),
            ("1.0-ALPHA", "1.0a0"),
            ("1.0-ALPHA1", "1.0a1"),
            // Various beta incarnations
            ("1.0b", "1.0b0"),
            ("1.0.b", "1.0b0"),
            ("1.0.b1", "1.0b1"),
            ("1.0-b", "1.0b0"),
            ("1.0-b1", "1.0b1"),
            ("1.0beta", "1.0b0"),
            ("1.0.beta", "1.0b0"),
            ("1.0.beta1", "1.0b1"),
            ("1.0-beta", "1.0b0"),
            ("1.0-beta1", "1.0b1"),
            ("1.0B", "1.0b0"),
            ("1.0.B", "1.0b0"),
            ("1.0.B1", "1.0b1"),
            ("1.0-B", "1.0b0"),
            ("1.0-B1", "1.0b1"),
            ("1.0BETA", "1.0b0"),
            ("1.0.BETA", "1.0b0"),
            ("1.0.BETA1", "1.0b1"),
            ("1.0-BETA", "1.0b0"),
            ("1.0-BETA1", "1.0b1"),
            // Various release candidate incarnations
            ("1.0c", "1.0rc0"),
            ("1.0.c", "1.0rc0"),
            ("1.0.c1", "1.0rc1"),
            ("1.0-c", "1.0rc0"),
            ("1.0-c1", "1.0rc1"),
            ("1.0rc", "1.0rc0"),
            ("1.0.rc", "1.0rc0"),
            ("1.0.rc1", "1.0rc1"),
            ("1.0-rc", "1.0rc0"),
            ("1.0-rc1", "1.0rc1"),
            ("1.0C", "1.0rc0"),
            ("1.0.C", "1.0rc0"),
            ("1.0.C1", "1.0rc1"),
            ("1.0-C", "1.0rc0"),
            ("1.0-C1", "1.0rc1"),
            ("1.0RC", "1.0rc0"),
            ("1.0.RC", "1.0rc0"),
            ("1.0.RC1", "1.0rc1"),
            ("1.0-RC", "1.0rc0"),
            ("1.0-RC1", "1.0rc1"),
            // Various post release incarnations
            ("1.0post", "1.0.post0"),
            ("1.0.post", "1.0.post0"),
            ("1.0post1", "1.0.post1"),
            ("1.0post", "1.0.post0"),
            ("1.0-post", "1.0.post0"),
            ("1.0-post1", "1.0.post1"),
            ("1.0POST", "1.0.post0"),
            ("1.0.POST", "1.0.post0"),
            ("1.0POST1", "1.0.post1"),
            ("1.0POST", "1.0.post0"),
            ("1.0r", "1.0.post0"),
            ("1.0rev", "1.0.post0"),
            ("1.0.POST1", "1.0.post1"),
            ("1.0.r1", "1.0.post1"),
            ("1.0.rev1", "1.0.post1"),
            ("1.0-POST", "1.0.post0"),
            ("1.0-POST1", "1.0.post1"),
            ("1.0-5", "1.0.post5"),
            ("1.0-r5", "1.0.post5"),
            ("1.0-rev5", "1.0.post5"),
            // Local version case insensitivity
            ("1.0+AbC", "1.0+abc"),
            // Integer Normalization
            ("1.01", "1.1"),
            ("1.0a05", "1.0a5"),
            ("1.0b07", "1.0b7"),
            ("1.0c056", "1.0rc56"),
            ("1.0rc09", "1.0rc9"),
            ("1.0.post000", "1.0.post0"),
            ("1.1.dev09000", "1.1.dev9000"),
            ("00!1.2", "1.2"),
            ("0100!0.0", "100!0.0"),
            // Various other normalizations
            ("v1.0", "1.0"),
            ("   v1.0\t\n", "1.0"),
        ];
        for (version_str, normalized_str) in versions {
            let version = Version::from_str(version_str).unwrap();
            let normalized = Version::from_str(normalized_str).unwrap();
            // Just test version parsing again
            assert_eq!(version, normalized, "{version_str} {normalized_str}");
            // Test version normalization
            assert_eq!(
                version.to_string(),
                normalized.to_string(),
                "{version_str} {normalized_str}"
            );
        }
    }

    /// <https://github.com/pypa/packaging/blob/237ff3aa348486cf835a980592af3a59fccd6101/tests/test_version.py#L229-L277>
    #[test]
    fn test_equality_and_normalization2() {
        let versions = [
            ("1.0.dev456", "1.0.dev456"),
            ("1.0a1", "1.0a1"),
            ("1.0a2.dev456", "1.0a2.dev456"),
            ("1.0a12.dev456", "1.0a12.dev456"),
            ("1.0a12", "1.0a12"),
            ("1.0b1.dev456", "1.0b1.dev456"),
            ("1.0b2", "1.0b2"),
            ("1.0b2.post345.dev456", "1.0b2.post345.dev456"),
            ("1.0b2.post345", "1.0b2.post345"),
            ("1.0rc1.dev456", "1.0rc1.dev456"),
            ("1.0rc1", "1.0rc1"),
            ("1.0", "1.0"),
            ("1.0.post456.dev34", "1.0.post456.dev34"),
            ("1.0.post456", "1.0.post456"),
            ("1.0.1", "1.0.1"),
            ("0!1.0.2", "1.0.2"),
            ("1.0.3+7", "1.0.3+7"),
            ("0!1.0.4+8.0", "1.0.4+8.0"),
            ("1.0.5+9.5", "1.0.5+9.5"),
            ("1.2+1234.abc", "1.2+1234.abc"),
            ("1.2+123456", "1.2+123456"),
            ("1.2+123abc", "1.2+123abc"),
            ("1.2+123abc456", "1.2+123abc456"),
            ("1.2+abc", "1.2+abc"),
            ("1.2+abc123", "1.2+abc123"),
            ("1.2+abc123def", "1.2+abc123def"),
            ("1.1.dev1", "1.1.dev1"),
            ("7!1.0.dev456", "7!1.0.dev456"),
            ("7!1.0a1", "7!1.0a1"),
            ("7!1.0a2.dev456", "7!1.0a2.dev456"),
            ("7!1.0a12.dev456", "7!1.0a12.dev456"),
            ("7!1.0a12", "7!1.0a12"),
            ("7!1.0b1.dev456", "7!1.0b1.dev456"),
            ("7!1.0b2", "7!1.0b2"),
            ("7!1.0b2.post345.dev456", "7!1.0b2.post345.dev456"),
            ("7!1.0b2.post345", "7!1.0b2.post345"),
            ("7!1.0rc1.dev456", "7!1.0rc1.dev456"),
            ("7!1.0rc1", "7!1.0rc1"),
            ("7!1.0", "7!1.0"),
            ("7!1.0.post456.dev34", "7!1.0.post456.dev34"),
            ("7!1.0.post456", "7!1.0.post456"),
            ("7!1.0.1", "7!1.0.1"),
            ("7!1.0.2", "7!1.0.2"),
            ("7!1.0.3+7", "7!1.0.3+7"),
            ("7!1.0.4+8.0", "7!1.0.4+8.0"),
            ("7!1.0.5+9.5", "7!1.0.5+9.5"),
            ("7!1.1.dev1", "7!1.1.dev1"),
        ];
        for (version_str, normalized_str) in versions {
            let version = Version::from_str(version_str).unwrap();
            let normalized = Version::from_str(normalized_str).unwrap();
            assert_eq!(version, normalized, "{version_str} {normalized_str}");
            // Test version normalization
            assert_eq!(
                version.to_string(),
                normalized_str,
                "{version_str} {normalized_str}"
            );
            // Since we're already at it
            assert_eq!(
                version.to_string(),
                normalized.to_string(),
                "{version_str} {normalized_str}"
            );
        }
    }

    #[test]
    fn test_star_fixed_version() {
        let result = Version::from_str("0.9.1.*");
        assert_eq!(result.unwrap_err(), ErrorKind::Wildcard.into());
    }

    #[test]
    fn test_invalid_word() {
        let result = Version::from_str("blergh");
        assert_eq!(result.unwrap_err(), ErrorKind::NoLeadingNumber.into());
    }

    #[test]
    fn test_from_version_star() {
        let p = |s: &str| -> Result<VersionPattern, _> { s.parse() };
        assert!(!p("1.2.3").unwrap().is_wildcard());
        assert!(p("1.2.3.*").unwrap().is_wildcard());
        assert_eq!(
            p("1.2.*.4.*").unwrap_err(),
            PatternErrorKind::WildcardNotTrailing.into(),
        );
        assert_eq!(
            p("1.0-dev1.*").unwrap_err(),
            ErrorKind::UnexpectedEnd {
                version: "1.0-dev1".to_string(),
                remaining: ".*".to_string()
            }
            .into(),
        );
        assert_eq!(
            p("1.0a1.*").unwrap_err(),
            ErrorKind::UnexpectedEnd {
                version: "1.0a1".to_string(),
                remaining: ".*".to_string()
            }
            .into(),
        );
        assert_eq!(
            p("1.0.post1.*").unwrap_err(),
            ErrorKind::UnexpectedEnd {
                version: "1.0.post1".to_string(),
                remaining: ".*".to_string()
            }
            .into(),
        );
        assert_eq!(
            p("1.0+lolwat.*").unwrap_err(),
            ErrorKind::LocalEmpty { precursor: '.' }.into(),
        );
    }

    // Tests the valid cases of our version parser. These were written
    // in tandem with the parser.
    //
    // They are meant to be additional (but in some cases likely redundant)
    // with some of the above tests.
    #[test]
    fn parse_version_valid() {
        let p = |s: &str| match Parser::new(s.as_bytes()).parse() {
            Ok(v) => v,
            Err(err) => unreachable!("expected valid version, but got error: {err:?}"),
        };

        // release-only tests
        assert_eq!(p("5"), Version::new([5]));
        assert_eq!(p("5.6"), Version::new([5, 6]));
        assert_eq!(p("5.6.7"), Version::new([5, 6, 7]));
        assert_eq!(p("512.623.734"), Version::new([512, 623, 734]));
        assert_eq!(p("1.2.3.4"), Version::new([1, 2, 3, 4]));
        assert_eq!(p("1.2.3.4.5"), Version::new([1, 2, 3, 4, 5]));

        // epoch tests
        assert_eq!(p("4!5"), Version::new([5]).with_epoch(4));
        assert_eq!(p("4!5.6"), Version::new([5, 6]).with_epoch(4));

        // pre-release tests
        assert_eq!(
            p("5a1"),
            Version::new([5]).with_pre(Some(Prerelease {
                kind: PrereleaseKind::Alpha,
                number: 1
            }))
        );
        assert_eq!(
            p("5alpha1"),
            Version::new([5]).with_pre(Some(Prerelease {
                kind: PrereleaseKind::Alpha,
                number: 1
            }))
        );
        assert_eq!(
            p("5b1"),
            Version::new([5]).with_pre(Some(Prerelease {
                kind: PrereleaseKind::Beta,
                number: 1
            }))
        );
        assert_eq!(
            p("5beta1"),
            Version::new([5]).with_pre(Some(Prerelease {
                kind: PrereleaseKind::Beta,
                number: 1
            }))
        );
        assert_eq!(
            p("5rc1"),
            Version::new([5]).with_pre(Some(Prerelease {
                kind: PrereleaseKind::Rc,
                number: 1
            }))
        );
        assert_eq!(
            p("5c1"),
            Version::new([5]).with_pre(Some(Prerelease {
                kind: PrereleaseKind::Rc,
                number: 1
            }))
        );
        assert_eq!(
            p("5preview1"),
            Version::new([5]).with_pre(Some(Prerelease {
                kind: PrereleaseKind::Rc,
                number: 1
            }))
        );
        assert_eq!(
            p("5pre1"),
            Version::new([5]).with_pre(Some(Prerelease {
                kind: PrereleaseKind::Rc,
                number: 1
            }))
        );
        assert_eq!(
            p("5.6.7pre1"),
            Version::new([5, 6, 7]).with_pre(Some(Prerelease {
                kind: PrereleaseKind::Rc,
                number: 1
            }))
        );
        assert_eq!(
            p("5alpha789"),
            Version::new([5]).with_pre(Some(Prerelease {
                kind: PrereleaseKind::Alpha,
                number: 789
            }))
        );
        assert_eq!(
            p("5.alpha789"),
            Version::new([5]).with_pre(Some(Prerelease {
                kind: PrereleaseKind::Alpha,
                number: 789
            }))
        );
        assert_eq!(
            p("5-alpha789"),
            Version::new([5]).with_pre(Some(Prerelease {
                kind: PrereleaseKind::Alpha,
                number: 789
            }))
        );
        assert_eq!(
            p("5_alpha789"),
            Version::new([5]).with_pre(Some(Prerelease {
                kind: PrereleaseKind::Alpha,
                number: 789
            }))
        );
        assert_eq!(
            p("5alpha.789"),
            Version::new([5]).with_pre(Some(Prerelease {
                kind: PrereleaseKind::Alpha,
                number: 789
            }))
        );
        assert_eq!(
            p("5alpha-789"),
            Version::new([5]).with_pre(Some(Prerelease {
                kind: PrereleaseKind::Alpha,
                number: 789
            }))
        );
        assert_eq!(
            p("5alpha_789"),
            Version::new([5]).with_pre(Some(Prerelease {
                kind: PrereleaseKind::Alpha,
                number: 789
            }))
        );
        assert_eq!(
            p("5ALPHA789"),
            Version::new([5]).with_pre(Some(Prerelease {
                kind: PrereleaseKind::Alpha,
                number: 789
            }))
        );
        assert_eq!(
            p("5aLpHa789"),
            Version::new([5]).with_pre(Some(Prerelease {
                kind: PrereleaseKind::Alpha,
                number: 789
            }))
        );
        assert_eq!(
            p("5alpha"),
            Version::new([5]).with_pre(Some(Prerelease {
                kind: PrereleaseKind::Alpha,
                number: 0
            }))
        );

        // post-release tests
        assert_eq!(p("5post2"), Version::new([5]).with_post(Some(2)));
        assert_eq!(p("5rev2"), Version::new([5]).with_post(Some(2)));
        assert_eq!(p("5r2"), Version::new([5]).with_post(Some(2)));
        assert_eq!(p("5.post2"), Version::new([5]).with_post(Some(2)));
        assert_eq!(p("5-post2"), Version::new([5]).with_post(Some(2)));
        assert_eq!(p("5_post2"), Version::new([5]).with_post(Some(2)));
        assert_eq!(p("5.post.2"), Version::new([5]).with_post(Some(2)));
        assert_eq!(p("5.post-2"), Version::new([5]).with_post(Some(2)));
        assert_eq!(p("5.post_2"), Version::new([5]).with_post(Some(2)));
        assert_eq!(
            p("5.6.7.post_2"),
            Version::new([5, 6, 7]).with_post(Some(2))
        );
        assert_eq!(p("5-2"), Version::new([5]).with_post(Some(2)));
        assert_eq!(p("5.6.7-2"), Version::new([5, 6, 7]).with_post(Some(2)));
        assert_eq!(p("5POST2"), Version::new([5]).with_post(Some(2)));
        assert_eq!(p("5PoSt2"), Version::new([5]).with_post(Some(2)));
        assert_eq!(p("5post"), Version::new([5]).with_post(Some(0)));

        // dev-release tests
        assert_eq!(p("5dev2"), Version::new([5]).with_dev(Some(2)));
        assert_eq!(p("5.dev2"), Version::new([5]).with_dev(Some(2)));
        assert_eq!(p("5-dev2"), Version::new([5]).with_dev(Some(2)));
        assert_eq!(p("5_dev2"), Version::new([5]).with_dev(Some(2)));
        assert_eq!(p("5.dev.2"), Version::new([5]).with_dev(Some(2)));
        assert_eq!(p("5.dev-2"), Version::new([5]).with_dev(Some(2)));
        assert_eq!(p("5.dev_2"), Version::new([5]).with_dev(Some(2)));
        assert_eq!(p("5.6.7.dev_2"), Version::new([5, 6, 7]).with_dev(Some(2)));
        assert_eq!(p("5DEV2"), Version::new([5]).with_dev(Some(2)));
        assert_eq!(p("5dEv2"), Version::new([5]).with_dev(Some(2)));
        assert_eq!(p("5DeV2"), Version::new([5]).with_dev(Some(2)));
        assert_eq!(p("5dev"), Version::new([5]).with_dev(Some(0)));

        // local tests
        assert_eq!(
            p("5+2"),
            Version::new([5]).with_local_segments(vec![LocalSegment::Number(2)])
        );
        assert_eq!(
            p("5+a"),
            Version::new([5]).with_local_segments(vec![LocalSegment::String("a".to_string())])
        );
        assert_eq!(
            p("5+abc.123"),
            Version::new([5]).with_local_segments(vec![
                LocalSegment::String("abc".to_string()),
                LocalSegment::Number(123),
            ])
        );
        assert_eq!(
            p("5+123.abc"),
            Version::new([5]).with_local_segments(vec![
                LocalSegment::Number(123),
                LocalSegment::String("abc".to_string()),
            ])
        );
        assert_eq!(
            p("5+18446744073709551615.abc"),
            Version::new([5]).with_local_segments(vec![
                LocalSegment::Number(18_446_744_073_709_551_615),
                LocalSegment::String("abc".to_string()),
            ])
        );
        assert_eq!(
            p("5+18446744073709551616.abc"),
            Version::new([5]).with_local_segments(vec![
                LocalSegment::String("18446744073709551616".to_string()),
                LocalSegment::String("abc".to_string()),
            ])
        );
        assert_eq!(
            p("5+ABC.123"),
            Version::new([5]).with_local_segments(vec![
                LocalSegment::String("abc".to_string()),
                LocalSegment::Number(123),
            ])
        );
        assert_eq!(
            p("5+ABC-123.4_5_xyz-MNO"),
            Version::new([5]).with_local_segments(vec![
                LocalSegment::String("abc".to_string()),
                LocalSegment::Number(123),
                LocalSegment::Number(4),
                LocalSegment::Number(5),
                LocalSegment::String("xyz".to_string()),
                LocalSegment::String("mno".to_string()),
            ])
        );
        assert_eq!(
            p("5.6.7+abc-00123"),
            Version::new([5, 6, 7]).with_local_segments(vec![
                LocalSegment::String("abc".to_string()),
                LocalSegment::Number(123),
            ])
        );
        assert_eq!(
            p("5.6.7+abc-foo00123"),
            Version::new([5, 6, 7]).with_local_segments(vec![
                LocalSegment::String("abc".to_string()),
                LocalSegment::String("foo00123".to_string()),
            ])
        );
        assert_eq!(
            p("5.6.7+abc-00123a"),
            Version::new([5, 6, 7]).with_local_segments(vec![
                LocalSegment::String("abc".to_string()),
                LocalSegment::String("00123a".to_string()),
            ])
        );

        // {pre-release, post-release} tests
        assert_eq!(
            p("5a2post3"),
            Version::new([5])
                .with_pre(Some(Prerelease {
                    kind: PrereleaseKind::Alpha,
                    number: 2
                }))
                .with_post(Some(3))
        );
        assert_eq!(
            p("5.a-2_post-3"),
            Version::new([5])
                .with_pre(Some(Prerelease {
                    kind: PrereleaseKind::Alpha,
                    number: 2
                }))
                .with_post(Some(3))
        );
        assert_eq!(
            p("5a2-3"),
            Version::new([5])
                .with_pre(Some(Prerelease {
                    kind: PrereleaseKind::Alpha,
                    number: 2
                }))
                .with_post(Some(3))
        );

        // Ignoring a no-op 'v' prefix.
        assert_eq!(p("v5"), Version::new([5]));
        assert_eq!(p("V5"), Version::new([5]));
        assert_eq!(p("v5.6.7"), Version::new([5, 6, 7]));

        // Ignoring leading and trailing whitespace.
        assert_eq!(p("  v5  "), Version::new([5]));
        assert_eq!(p("  5  "), Version::new([5]));
        assert_eq!(
            p("  5.6.7+abc.123.xyz  "),
            Version::new([5, 6, 7]).with_local_segments(vec![
                LocalSegment::String("abc".to_string()),
                LocalSegment::Number(123),
                LocalSegment::String("xyz".to_string())
            ])
        );
        assert_eq!(p("  \n5\n \t"), Version::new([5]));

        // min tests
        assert!(Parser::new("1.min0".as_bytes()).parse().is_err());
    }

    // Tests the error cases of our version parser.
    //
    // I wrote these with the intent to cover every possible error
    // case.
    //
    // They are meant to be additional (but in some cases likely redundant)
    // with some of the above tests.
    #[test]
    fn parse_version_invalid() {
        let p = |s: &str| match Parser::new(s.as_bytes()).parse() {
            Err(err) => err,
            Ok(v) => unreachable!(
                "expected version parser error, but got: {v:?}",
                v = v.as_bloated_debug()
            ),
        };

        assert_eq!(p(""), ErrorKind::NoLeadingNumber.into());
        assert_eq!(p("a"), ErrorKind::NoLeadingNumber.into());
        assert_eq!(p("v 5"), ErrorKind::NoLeadingNumber.into());
        assert_eq!(p("V 5"), ErrorKind::NoLeadingNumber.into());
        assert_eq!(p("x 5"), ErrorKind::NoLeadingNumber.into());
        assert_eq!(
            p("18446744073709551616"),
            ErrorKind::NumberTooBig {
                bytes: b"18446744073709551616".to_vec()
            }
            .into()
        );
        assert_eq!(p("5!"), ErrorKind::NoLeadingReleaseNumber.into());
        assert_eq!(
            p("5.6./"),
            ErrorKind::UnexpectedEnd {
                version: "5.6".to_string(),
                remaining: "./".to_string()
            }
            .into()
        );
        assert_eq!(
            p("5.6.-alpha2"),
            ErrorKind::UnexpectedEnd {
                version: "5.6".to_string(),
                remaining: ".-alpha2".to_string()
            }
            .into()
        );
        assert_eq!(
            p("1.2.3a18446744073709551616"),
            ErrorKind::NumberTooBig {
                bytes: b"18446744073709551616".to_vec()
            }
            .into()
        );
        assert_eq!(p("5+"), ErrorKind::LocalEmpty { precursor: '+' }.into());
        assert_eq!(p("5+ "), ErrorKind::LocalEmpty { precursor: '+' }.into());
        assert_eq!(p("5+abc."), ErrorKind::LocalEmpty { precursor: '.' }.into());
        assert_eq!(p("5+abc-"), ErrorKind::LocalEmpty { precursor: '-' }.into());
        assert_eq!(p("5+abc_"), ErrorKind::LocalEmpty { precursor: '_' }.into());
        assert_eq!(
            p("5+abc. "),
            ErrorKind::LocalEmpty { precursor: '.' }.into()
        );
        assert_eq!(
            p("5.6-"),
            ErrorKind::UnexpectedEnd {
                version: "5.6".to_string(),
                remaining: "-".to_string()
            }
            .into()
        );
    }

    // Exercise every version accepted by the specialized five-byte fast path.
    // The non-digit cases ensure that it falls back to the general parser.
    #[test]
    fn parse_version_single_digit_release() {
        for major in 0u8..=9 {
            for minor in 0u8..=9 {
                for patch in 0u8..=9 {
                    let input = format!("{major}.{minor}.{patch}");
                    assert_eq!(
                        input.parse(),
                        Ok(Version::new([
                            u64::from(major),
                            u64::from(minor),
                            u64::from(patch),
                        ])),
                        "{input}"
                    );
                }
            }
        }

        assert!("a.1.2".parse::<Version>().is_err());
        assert_eq!(
            "1.a.2"
                .parse::<Version>()
                .map(|version| version.to_string()),
            Ok("1a2".to_string())
        );
        assert_eq!(
            "1.2.a"
                .parse::<Version>()
                .map(|version| version.to_string()),
            Ok("1.2a0".to_string())
        );
    }

    #[test]
    fn parse_version_pattern_valid() {
        let p = |s: &str| match Parser::new(s.as_bytes()).parse_pattern() {
            Ok(v) => v,
            Err(err) => unreachable!("expected valid version, but got error: {err:?}"),
        };

        assert_eq!(p("5.*"), VersionPattern::wildcard(Version::new([5])));
        assert_eq!(p("5.6.*"), VersionPattern::wildcard(Version::new([5, 6])));
        assert_eq!(
            p("2!5.6.*"),
            VersionPattern::wildcard(Version::new([5, 6]).with_epoch(2))
        );
    }

    #[test]
    fn parse_version_pattern_invalid() {
        let p = |s: &str| match Parser::new(s.as_bytes()).parse_pattern() {
            Err(err) => err,
            Ok(vpat) => unreachable!("expected version pattern parser error, but got: {vpat:?}"),
        };

        assert_eq!(p("*"), ErrorKind::NoLeadingNumber.into());
        assert_eq!(p("2!*"), ErrorKind::NoLeadingReleaseNumber.into());
    }

    // Tests that the ordering between versions is correct.
    //
    // The ordering example used here was taken from PEP 440:
    // https://packaging.python.org/en/latest/specifications/version-specifiers/#summary-of-permitted-suffixes-and-relative-ordering
    #[test]
    fn ordering() {
        let versions = &[
            "1.dev0",
            "1.0.dev456",
            "1.0a1",
            "1.0a2.dev456",
            "1.0a12.dev456",
            "1.0a12",
            "1.0b1.dev456",
            "1.0b2",
            "1.0b2.post345.dev456",
            "1.0b2.post345",
            "1.0rc1.dev456",
            "1.0rc1",
            "1.0",
            "1.0+abc.5",
            "1.0+abc.7",
            "1.0+5",
            "1.0.post456.dev34",
            "1.0.post456",
            "1.0.15",
            "1.1.dev1",
        ];
        for (i, v1) in versions.iter().enumerate() {
            for v2 in &versions[i + 1..] {
                let less = v1.parse::<Version>().unwrap();
                let greater = v2.parse::<Version>().unwrap();
                assert_eq!(
                    less.cmp(&greater),
                    Ordering::Less,
                    "less: {:?}\ngreater: {:?}",
                    less.as_bloated_debug(),
                    greater.as_bloated_debug()
                );
            }
        }
    }

    #[test]
    fn local_sentinel_version() {
        let sentinel = Version::new([1, 0]).with_local(LocalVersion::Max);

        // Ensure that the "max local version" sentinel is less than the following versions.
        let versions = &["1.0.post0", "1.1"];

        for greater in versions {
            let greater = greater.parse::<Version>().unwrap();
            assert_eq!(
                sentinel.cmp(&greater),
                Ordering::Less,
                "less: {:?}\ngreater: {:?}",
                greater.as_bloated_debug(),
                sentinel.as_bloated_debug(),
            );
        }

        // Ensure that the "max local version" sentinel is greater than the following versions.
        let versions = &["1.0", "1.0.a0", "1.0+local"];

        for less in versions {
            let less = less.parse::<Version>().unwrap();
            assert_eq!(
                sentinel.cmp(&less),
                Ordering::Greater,
                "less: {:?}\ngreater: {:?}",
                sentinel.as_bloated_debug(),
                less.as_bloated_debug()
            );
        }
    }

    #[test]
    fn min_version() {
        // Ensure that the `.min` suffix precedes all other suffixes.
        let less = Version::new([1, 0]).with_min(Some(0));

        let versions = &[
            "1.dev0",
            "1.0.dev456",
            "1.0a1",
            "1.0a2.dev456",
            "1.0a12.dev456",
            "1.0a12",
            "1.0b1.dev456",
            "1.0b2",
            "1.0b2.post345.dev456",
            "1.0b2.post345",
            "1.0rc1.dev456",
            "1.0rc1",
            "1.0",
            "1.0+abc.5",
            "1.0+abc.7",
            "1.0+5",
            "1.0.post456.dev34",
            "1.0.post456",
            "1.0.15",
            "1.1.dev1",
        ];

        for greater in versions {
            let greater = greater.parse::<Version>().unwrap();
            assert_eq!(
                less.cmp(&greater),
                Ordering::Less,
                "less: {:?}\ngreater: {:?}",
                less.as_bloated_debug(),
                greater.as_bloated_debug()
            );
        }
    }

    #[test]
    fn max_version() {
        // Ensure that the `.max` suffix succeeds all other suffixes.
        let greater = Version::new([1, 0]).with_max(Some(0));

        let versions = &[
            "1.dev0",
            "1.0.dev456",
            "1.0a1",
            "1.0a2.dev456",
            "1.0a12.dev456",
            "1.0a12",
            "1.0b1.dev456",
            "1.0b2",
            "1.0b2.post345.dev456",
            "1.0b2.post345",
            "1.0rc1.dev456",
            "1.0rc1",
            "1.0",
            "1.0+abc.5",
            "1.0+abc.7",
            "1.0+5",
            "1.0.post456.dev34",
            "1.0.post456",
            "1.0",
        ];

        for less in versions {
            let less = less.parse::<Version>().unwrap();
            assert_eq!(
                less.cmp(&greater),
                Ordering::Less,
                "less: {:?}\ngreater: {:?}",
                less.as_bloated_debug(),
                greater.as_bloated_debug()
            );
        }

        // Ensure that the `.max` suffix plays nicely with pre-release versions.
        let greater = Version::new([1, 0])
            .with_pre(Some(Prerelease {
                kind: PrereleaseKind::Alpha,
                number: 1,
            }))
            .with_max(Some(0));

        let versions = &["1.0a1", "1.0a1+local", "1.0a1.post1"];

        for less in versions {
            let less = less.parse::<Version>().unwrap();
            assert_eq!(
                less.cmp(&greater),
                Ordering::Less,
                "less: {:?}\ngreater: {:?}",
                less.as_bloated_debug(),
                greater.as_bloated_debug()
            );
        }

        // Ensure that the `.max` suffix plays nicely with pre-release versions.
        let less = Version::new([1, 0])
            .with_pre(Some(Prerelease {
                kind: PrereleaseKind::Alpha,
                number: 1,
            }))
            .with_max(Some(0));

        let versions = &["1.0b1", "1.0b1+local", "1.0b1.post1", "1.0"];

        for greater in versions {
            let greater = greater.parse::<Version>().unwrap();
            assert_eq!(
                less.cmp(&greater),
                Ordering::Less,
                "less: {:?}\ngreater: {:?}",
                less.as_bloated_debug(),
                greater.as_bloated_debug()
            );
        }
    }

    // Tests our bespoke u64 decimal integer parser.
    #[test]
    fn parse_number_u64() {
        let p = |s: &str| parse_u64(s.as_bytes());
        assert_eq!(p("0"), Ok(0));
        assert_eq!(p("00"), Ok(0));
        assert_eq!(p("1"), Ok(1));
        assert_eq!(p("01"), Ok(1));
        assert_eq!(p("9"), Ok(9));
        assert_eq!(p("10"), Ok(10));
        assert_eq!(p("18446744073709551615"), Ok(18_446_744_073_709_551_615));
        assert_eq!(p("018446744073709551615"), Ok(18_446_744_073_709_551_615));
        assert_eq!(
            p("000000018446744073709551615"),
            Ok(18_446_744_073_709_551_615)
        );

        assert_eq!(p("10a"), Err(ErrorKind::InvalidDigit { got: b'a' }.into()));
        assert_eq!(p("10["), Err(ErrorKind::InvalidDigit { got: b'[' }.into()));
        assert_eq!(p("10/"), Err(ErrorKind::InvalidDigit { got: b'/' }.into()));
        // u64::MAX + 1 is rejected (overflow during parsing).
        assert_eq!(
            p("18446744073709551616"),
            Err(ErrorKind::NumberTooBig {
                bytes: b"18446744073709551616".to_vec()
            }
            .into())
        );
        assert_eq!(
            p("18446744073799551615abc"),
            Err(ErrorKind::NumberTooBig {
                bytes: b"18446744073799551615abc".to_vec()
            }
            .into())
        );
        assert_eq!(
            parse_u64(b"18446744073799551615\xFF"),
            Err(ErrorKind::NumberTooBig {
                bytes: b"18446744073799551615\xFF".to_vec()
            }
            .into())
        );
    }

    impl Version {
        /// Returns a more "bloated" debug representation of this [`Version`].
        ///
        /// We don't do this by default because it takes up a ton of space, and
        /// just printing out the display version of the version is quite a bit
        /// simpler.
        ///
        /// Nevertheless, when *testing* version parsing, you really want to
        /// be able to peek at all of its constituent parts. So we use this in
        /// assertion failure messages.
        pub(crate) fn as_bloated_debug(&self) -> impl std::fmt::Debug + '_ {
            std::fmt::from_fn(|f| {
                f.debug_struct("Version")
                    .field("epoch", &self.epoch())
                    .field("release", &&*self.release())
                    .field("pre", &self.pre())
                    .field("post", &self.post())
                    .field("dev", &self.dev())
                    .field("local", &self.local())
                    .field("min", &self.min())
                    .field("max", &self.max())
                    .finish()
            })
        }
    }

    /// This explicitly tests that we preserve trailing zeros in a version
    /// string. i.e., Both `1.2` and `1.2.0` round-trip, with the former
    /// lacking a trailing zero and the latter including it.
    #[test]
    fn preserve_trailing_zeros() {
        let v1: Version = "1.2.0".parse().unwrap();
        assert_eq!(&*v1.release(), &[1, 2, 0]);
        assert_eq!(v1.to_string(), "1.2.0");

        let v2: Version = "1.2".parse().unwrap();
        assert_eq!(&*v2.release(), &[1, 2]);
        assert_eq!(v2.to_string(), "1.2");
    }

    #[test]
    fn only_release_at_precision_preserves_epoch_and_discards_suffixes() {
        let version = "1!2.3rc1.post2.dev3+local"
            .parse::<Version>()
            .expect("valid version");
        assert_eq!(
            version
                .only_release_at_precision(4)
                .expect("non-zero precision")
                .to_string(),
            "1!2.3.0.0"
        );
        assert_eq!(version.only_release_at_precision(0), None);
    }

    #[test]
    fn only_release_trimmed_discards_non_release_segments() {
        for version in ["1.2a1", "1.2.post1", "1!1.2", "1.2+local", "1.2.dev1"] {
            let version = version.parse::<Version>().unwrap();
            assert_eq!(version.only_release_trimmed(), Version::new([1, 2]));
        }

        assert_eq!(
            Version::new([1, 2])
                .with_min(Some(0))
                .only_release_trimmed(),
            Version::new([1, 2])
        );
        assert_eq!(
            Version::new([1, 2])
                .with_max(Some(0))
                .only_release_trimmed(),
            Version::new([1, 2])
        );
        assert_eq!(
            Version::new([1, 2, 0]).only_release_trimmed(),
            Version::new([1, 2])
        );
        assert_eq!(
            Version::new([1, 2]).only_release_trimmed(),
            Version::new([1, 2])
        );
    }

    #[test]
    fn type_size() {
        assert_eq!(size_of::<VersionSmall>(), size_of::<usize>() * 2);
        assert_eq!(size_of::<Version>(), size_of::<usize>() * 2);
        #[cfg(feature = "rkyv")]
        {
            assert_eq!(size_of::<rkyv::Archived<VersionSmall>>(), 16);
            assert_eq!(size_of::<rkyv::Archived<Version>>(), 24);
        }
    }

    /// Test major bumping
    /// Explicitly using the string display because we want to preserve formatting where possible!
    #[test]
    fn bump_major() {
        // one digit
        let mut version = "0".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpRelease {
            index: 0,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "1");

        // two digit
        let mut version = "1.5".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpRelease {
            index: 0,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "2.0");

        // three digit (zero major)
        let mut version = "0.1.2".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpRelease {
            index: 0,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "1.0.0");

        // three digit (non-zero major)
        let mut version = "1.2.3".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpRelease {
            index: 0,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "2.0.0");

        // four digit
        let mut version = "1.2.3.4".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpRelease {
            index: 0,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "2.0.0.0");

        // All the version junk
        let mut version = "5!1.7.3.5b2.post345.dev456+local"
            .parse::<Version>()
            .unwrap();
        version.bump(BumpCommand::BumpRelease {
            index: 0,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "5!2.0.0.0+local");
        version.bump(BumpCommand::BumpRelease {
            index: 0,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "5!3.0.0.0+local");
    }

    /// Test minor bumping
    /// Explicitly using the string display because we want to preserve formatting where possible!
    #[test]
    fn bump_minor() {
        // one digit
        let mut version = "0".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpRelease {
            index: 1,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "0.1");

        // two digit
        let mut version = "1.5".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpRelease {
            index: 1,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "1.6");

        // three digit (non-zero major)
        let mut version = "5.3.6".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpRelease {
            index: 1,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "5.4.0");

        // four digit
        let mut version = "1.2.3.4".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpRelease {
            index: 1,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "1.3.0.0");

        // All the version junk
        let mut version = "5!1.7.3.5b2.post345.dev456+local"
            .parse::<Version>()
            .unwrap();
        version.bump(BumpCommand::BumpRelease {
            index: 1,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "5!1.8.0.0+local");
        version.bump(BumpCommand::BumpRelease {
            index: 1,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "5!1.9.0.0+local");
    }

    /// Test patch bumping
    /// Explicitly using the string display because we want to preserve formatting where possible!
    #[test]
    fn bump_patch() {
        // one digit
        let mut version = "0".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpRelease {
            index: 2,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "0.0.1");

        // two digit
        let mut version = "1.5".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpRelease {
            index: 2,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "1.5.1");

        // three digit
        let mut version = "5.3.6".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpRelease {
            index: 2,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "5.3.7");

        // four digit
        let mut version = "1.2.3.4".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpRelease {
            index: 2,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "1.2.4.0");

        // All the version junk
        let mut version = "5!1.7.3.5b2.post345.dev456+local"
            .parse::<Version>()
            .unwrap();
        version.bump(BumpCommand::BumpRelease {
            index: 2,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "5!1.7.4.0+local");
        version.bump(BumpCommand::BumpRelease {
            index: 2,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "5!1.7.5.0+local");
    }

    /// Test alpha bumping
    /// Explicitly using the string display because we want to preserve formatting where possible!
    #[test]
    fn bump_alpha() {
        // one digit
        let mut version = "0".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpPrerelease {
            kind: PrereleaseKind::Alpha,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "0a1");

        // two digit
        let mut version = "1.5".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpPrerelease {
            kind: PrereleaseKind::Alpha,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "1.5a1");

        // three digit
        let mut version = "5.3.6".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpPrerelease {
            kind: PrereleaseKind::Alpha,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "5.3.6a1");

        // four digit
        let mut version = "1.2.3.4".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpPrerelease {
            kind: PrereleaseKind::Alpha,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "1.2.3.4a1");

        // All the version junk
        let mut version = "5!1.7.3.5b2.post345.dev456+local"
            .parse::<Version>()
            .unwrap();
        version.bump(BumpCommand::BumpPrerelease {
            kind: PrereleaseKind::Alpha,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "5!1.7.3.5a1+local");
        version.bump(BumpCommand::BumpPrerelease {
            kind: PrereleaseKind::Alpha,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "5!1.7.3.5a2+local");
    }

    /// Test beta bumping
    /// Explicitly using the string display because we want to preserve formatting where possible!
    #[test]
    fn bump_beta() {
        // one digit
        let mut version = "0".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpPrerelease {
            kind: PrereleaseKind::Beta,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "0b1");

        // two digit
        let mut version = "1.5".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpPrerelease {
            kind: PrereleaseKind::Beta,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "1.5b1");

        // three digit
        let mut version = "5.3.6".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpPrerelease {
            kind: PrereleaseKind::Beta,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "5.3.6b1");

        // four digit
        let mut version = "1.2.3.4".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpPrerelease {
            kind: PrereleaseKind::Beta,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "1.2.3.4b1");

        // All the version junk
        let mut version = "5!1.7.3.5a2.post345.dev456+local"
            .parse::<Version>()
            .unwrap();
        version.bump(BumpCommand::BumpPrerelease {
            kind: PrereleaseKind::Beta,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "5!1.7.3.5b1+local");
        version.bump(BumpCommand::BumpPrerelease {
            kind: PrereleaseKind::Beta,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "5!1.7.3.5b2+local");
    }

    /// Test rc bumping
    /// Explicitly using the string display because we want to preserve formatting where possible!
    #[test]
    fn bump_rc() {
        // one digit
        let mut version = "0".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpPrerelease {
            kind: PrereleaseKind::Rc,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "0rc1");

        // two digit
        let mut version = "1.5".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpPrerelease {
            kind: PrereleaseKind::Rc,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "1.5rc1");

        // three digit
        let mut version = "5.3.6".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpPrerelease {
            kind: PrereleaseKind::Rc,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "5.3.6rc1");

        // four digit
        let mut version = "1.2.3.4".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpPrerelease {
            kind: PrereleaseKind::Rc,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "1.2.3.4rc1");

        // All the version junk
        let mut version = "5!1.7.3.5b2.post345.dev456+local"
            .parse::<Version>()
            .unwrap();
        version.bump(BumpCommand::BumpPrerelease {
            kind: PrereleaseKind::Rc,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "5!1.7.3.5rc1+local");
        version.bump(BumpCommand::BumpPrerelease {
            kind: PrereleaseKind::Rc,
            value: None,
        });
        assert_eq!(version.to_string().as_str(), "5!1.7.3.5rc2+local");
    }

    /// Test post bumping
    /// Explicitly using the string display because we want to preserve formatting where possible!
    #[test]
    fn bump_post() {
        // one digit
        let mut version = "0".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpPost { value: None });
        assert_eq!(version.to_string().as_str(), "0.post1");

        // two digit
        let mut version = "1.5".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpPost { value: None });
        assert_eq!(version.to_string().as_str(), "1.5.post1");

        // three digit
        let mut version = "5.3.6".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpPost { value: None });
        assert_eq!(version.to_string().as_str(), "5.3.6.post1");

        // four digit
        let mut version = "1.2.3.4".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpPost { value: None });
        assert_eq!(version.to_string().as_str(), "1.2.3.4.post1");

        // All the version junk
        let mut version = "5!1.7.3.5b2.dev123+local".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpPost { value: None });
        assert_eq!(version.to_string().as_str(), "5!1.7.3.5b2.post1+local");
        version.bump(BumpCommand::BumpPost { value: None });
        assert_eq!(version.to_string().as_str(), "5!1.7.3.5b2.post2+local");
    }

    /// Test dev bumping
    /// Explicitly using the string display because we want to preserve formatting where possible!
    #[test]
    fn bump_dev() {
        // one digit
        let mut version = "0".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpDev { value: None });
        assert_eq!(version.to_string().as_str(), "0.dev1");

        // two digit
        let mut version = "1.5".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpDev { value: None });
        assert_eq!(version.to_string().as_str(), "1.5.dev1");

        // three digit
        let mut version = "5.3.6".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpDev { value: None });
        assert_eq!(version.to_string().as_str(), "5.3.6.dev1");

        // four digit
        let mut version = "1.2.3.4".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpDev { value: None });
        assert_eq!(version.to_string().as_str(), "1.2.3.4.dev1");

        // All the version junk
        let mut version = "5!1.7.3.5b2.post345+local".parse::<Version>().unwrap();
        version.bump(BumpCommand::BumpDev { value: None });
        assert_eq!(
            version.to_string().as_str(),
            "5!1.7.3.5b2.post345.dev1+local"
        );
        version.bump(BumpCommand::BumpDev { value: None });
        assert_eq!(
            version.to_string().as_str(),
            "5!1.7.3.5b2.post345.dev2+local"
        );
    }

    /// Test stable setting
    /// Explicitly using the string display because we want to preserve formatting where possible!
    #[test]
    fn make_stable() {
        // one digit
        let mut version = "0".parse::<Version>().unwrap();
        version.bump(BumpCommand::MakeStable);
        assert_eq!(version.to_string().as_str(), "0");

        // two digit
        let mut version = "1.5".parse::<Version>().unwrap();
        version.bump(BumpCommand::MakeStable);
        assert_eq!(version.to_string().as_str(), "1.5");

        // three digit
        let mut version = "5.3.6".parse::<Version>().unwrap();
        version.bump(BumpCommand::MakeStable);
        assert_eq!(version.to_string().as_str(), "5.3.6");

        // four digit
        let mut version = "1.2.3.4".parse::<Version>().unwrap();
        version.bump(BumpCommand::MakeStable);
        assert_eq!(version.to_string().as_str(), "1.2.3.4");

        // All the version junk
        let mut version = "5!1.7.3.5b2.post345+local".parse::<Version>().unwrap();
        version.bump(BumpCommand::MakeStable);
        assert_eq!(version.to_string().as_str(), "5!1.7.3.5+local");
        version.bump(BumpCommand::MakeStable);
        assert_eq!(version.to_string().as_str(), "5!1.7.3.5+local");
    }
}
