use std::sync::Arc;

use pubgrub::Id;
use rustc_hash::FxHashMap;

use uv_distribution_types::IndexUrl;
use uv_pep440::Version;

use crate::preferences::Entry;
use crate::pubgrub::{PubGrubPackage, Range};
use crate::universal_marker::UniversalMarker;

use super::VersionsResponse;

/// The variable part of a resolver-origin preference. Its source and empty hashes are fixed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SiblingPreference {
    index: IndexUrl,
    marker: UniversalMarker,
    version: Version,
}

impl SiblingPreference {
    pub(super) fn new(index: IndexUrl, marker: UniversalMarker, version: Version) -> Self {
        Self {
            index,
            marker,
            version,
        }
    }

    pub(super) fn to_entry(&self) -> Entry {
        Entry::from_resolver(self.index.clone(), self.marker, self.version.clone())
    }
}

/// Selection inputs that can change while a coordinated fork is suspended.
///
/// The ordered sibling entries extend a fixed base preference set. Holding the response keeps
/// its identity valid even if a different response is later published for the same package.
#[derive(Clone, Debug)]
pub(super) struct SelectedVersionContext {
    siblings: Vec<SiblingPreference>,
    versions: Arc<VersionsResponse>,
}

impl SelectedVersionContext {
    pub(super) fn new(siblings: Vec<SiblingPreference>, versions: Arc<VersionsResponse>) -> Self {
        Self { siblings, versions }
    }

    /// Retain the pre-selection context only while the same response remains published.
    pub(super) fn confirm(self, versions: Option<&Arc<VersionsResponse>>) -> Option<Self> {
        if versions.is_some_and(|versions| Arc::ptr_eq(&self.versions, versions)) {
            Some(self)
        } else {
            None
        }
    }

    fn matches(&self, siblings: &[SiblingPreference], versions: &Arc<VersionsResponse>) -> bool {
        Arc::ptr_eq(&self.versions, versions) && self.siblings.as_slice() == siblings
    }
}

impl PartialEq for SelectedVersionContext {
    fn eq(&self, other: &Self) -> bool {
        self.matches(&other.siblings, &other.versions)
    }
}

impl Eq for SelectedVersionContext {}

#[derive(Clone)]
struct SelectedVersion {
    range: Range<Version>,
    version: Version,
    resume: u64,
    context: Option<SelectedVersionContext>,
}

/// Successful selections in one resolver environment.
#[derive(Clone, Default)]
pub(super) struct SelectedVersions {
    entries: FxHashMap<Id<PubGrubPackage>, SelectedVersion>,
    resume: u64,
}

impl SelectedVersions {
    /// Start a resume that must lazily revalidate earlier selections.
    pub(super) fn start_resume(&mut self) {
        self.resume = self.resume.wrapping_add(1);
        if self.resume == 0 {
            self.clear();
        }
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
    }

    /// Reuse an identical decision without rebuilding preferences within a single resume.
    pub(super) fn get_same_resume(
        &self,
        package: Id<PubGrubPackage>,
        range: &Range<Version>,
    ) -> Option<Version> {
        let selection = self.entries.get(&package)?;
        (selection.resume == self.resume && &selection.range == range)
            .then(|| selection.version.clone())
    }

    /// Revalidate only the package being reconsidered, not every selection in the fork.
    pub(super) fn get_across_resumes(
        &mut self,
        package: Id<PubGrubPackage>,
        range: &Range<Version>,
        siblings: &[SiblingPreference],
        versions: &Arc<VersionsResponse>,
    ) -> Option<Version> {
        let selection = self.entries.get_mut(&package)?;
        let context = selection.context.as_ref()?;
        if &selection.range != range || !context.matches(siblings, versions) {
            return None;
        }
        selection.resume = self.resume;
        Some(selection.version.clone())
    }

    pub(super) fn insert(
        &mut self,
        package: Id<PubGrubPackage>,
        range: Range<Version>,
        version: Version,
        context: Option<SelectedVersionContext>,
    ) {
        self.entries.insert(
            package,
            SelectedVersion {
                range,
                version,
                resume: self.resume,
                context,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::Arc;

    use pubgrub::{Id, State};

    use uv_distribution_types::IndexUrl;
    use uv_normalize::{ExtraName, PackageName};
    use uv_pep440::{MIN_VERSION, Version};
    use uv_pep508::MarkerTree;
    use uv_pypi_types::ConflictItem;
    use uv_resolver_types::PackageNodeKind;

    use crate::dependency_provider::UvDependencyProvider;
    use crate::pubgrub::{PubGrubPackage, PubGrubPackageInner, Range};
    use crate::universal_marker::{ConflictMarker, UniversalMarker};

    use super::{SelectedVersionContext, SelectedVersions, SiblingPreference, VersionsResponse};

    fn package_ids() -> Result<(Id<PubGrubPackage>, Id<PubGrubPackage>), Box<dyn Error>> {
        let mut state = State::<UvDependencyProvider>::init(
            PubGrubPackage::from(PubGrubPackageInner::Root(None)),
            MIN_VERSION.clone(),
        );
        let name: PackageName = "example".parse()?;
        let package = state.package_store.alloc(PubGrubPackage::from_package(
            name.clone(),
            PackageNodeKind::Base,
            MarkerTree::TRUE,
        ));
        let proxy = state.package_store.alloc(PubGrubPackage::from_package(
            name,
            PackageNodeKind::Base,
            "sys_platform == 'linux'".parse()?,
        ));
        Ok((package, proxy))
    }

    fn response() -> Arc<VersionsResponse> {
        Arc::new(VersionsResponse::Found(Vec::new()))
    }

    fn sibling(version: u64) -> Result<SiblingPreference, Box<dyn Error>> {
        Ok(SiblingPreference::new(
            IndexUrl::parse("https://pypi.org/simple", None)?,
            UniversalMarker::TRUE,
            Version::new([version]),
        ))
    }

    #[test]
    fn same_resume_matches_the_package_and_range() -> Result<(), Box<dyn Error>> {
        let (package, proxy) = package_ids()?;
        let mut cache = SelectedVersions::default();
        let range = Range::full();
        let version = Version::new([1]);
        cache.insert(package, range.clone(), version.clone(), None);

        assert_eq!(
            cache.get_same_resume(package, &range),
            Some(version.clone())
        );
        assert_eq!(cache.get_same_resume(proxy, &range), None);
        assert_eq!(
            cache.get_same_resume(package, &Range::singleton(version)),
            None,
        );

        cache.start_resume();
        assert_eq!(cache.get_same_resume(package, &range), None);
        let context = SelectedVersionContext::new(Vec::new(), response());
        assert_eq!(
            cache.get_across_resumes(package, &range, &context.siblings, &context.versions),
            None,
        );
        Ok(())
    }

    #[test]
    fn unchanged_context_revalidates_only_the_selected_package() -> Result<(), Box<dyn Error>> {
        let (package, proxy) = package_ids()?;
        let mut cache = SelectedVersions::default();
        let range = Range::full();
        let version = Version::new([1]);
        let context = SelectedVersionContext::new(vec![sibling(1)?], response());
        for package in [package, proxy] {
            cache.insert(
                package,
                range.clone(),
                version.clone(),
                Some(context.clone()),
            );
        }

        cache.start_resume();
        assert_eq!(cache.get_same_resume(package, &range), None);
        assert_eq!(
            cache.get_across_resumes(package, &range, &context.siblings, &context.versions),
            Some(version.clone()),
        );
        assert_eq!(cache.get_same_resume(package, &range), Some(version));
        assert_eq!(cache.get_same_resume(proxy, &range), None);

        cache.clear();
        assert_eq!(cache.get_same_resume(package, &range), None);
        assert_eq!(
            cache.get_across_resumes(package, &range, &context.siblings, &context.versions),
            None,
        );
        Ok(())
    }

    #[test]
    fn selection_context_compares_order_source_version_and_full_marker()
    -> Result<(), Box<dyn Error>> {
        let versions = response();
        let context = SelectedVersionContext::new(vec![sibling(1)?, sibling(2)?], versions);
        assert_eq!(context, context.clone());

        let mut changed = context.clone();
        changed.siblings.reverse();
        assert_ne!(context, changed);

        let mut changed = context.clone();
        changed.siblings[0].index = IndexUrl::parse("https://example.org/simple", None)?;
        assert_ne!(context, changed);

        let mut changed = context.clone();
        changed.siblings[0].version = Version::new([3]);
        assert_ne!(context, changed);

        let mut changed = context.clone();
        changed.siblings[0].marker =
            UniversalMarker::from_combined("python_version < '3.12'".parse()?);
        assert_ne!(context, changed);

        let item = ConflictItem::from((
            "project".parse::<PackageName>()?,
            "feature".parse::<ExtraName>()?,
        ));
        let mut changed = context.clone();
        changed.siblings[0].marker =
            UniversalMarker::new(MarkerTree::TRUE, ConflictMarker::from_conflict_item(&item));
        assert_ne!(context, changed);
        Ok(())
    }

    #[test]
    fn response_identity_must_survive_selection() {
        let versions = response();
        let context = SelectedVersionContext::new(Vec::new(), versions.clone());
        let replacement = response();
        assert_eq!(
            context.clone().confirm(Some(&versions)),
            Some(context.clone())
        );
        assert!(context.clone().confirm(None).is_none());
        assert!(context.clone().confirm(Some(&replacement)).is_none());
        assert_ne!(
            context,
            SelectedVersionContext::new(Vec::new(), replacement),
        );
    }

    #[test]
    fn resume_counter_wrap_discards_old_contexts() -> Result<(), Box<dyn Error>> {
        let (package, _) = package_ids()?;
        let mut cache = SelectedVersions {
            resume: u64::MAX,
            ..SelectedVersions::default()
        };
        let range = Range::full();
        let context = SelectedVersionContext::new(Vec::new(), response());
        cache.insert(
            package,
            range.clone(),
            Version::new([1]),
            Some(context.clone()),
        );
        cache.start_resume();
        assert_eq!(
            cache.get_across_resumes(package, &range, &context.siblings, &context.versions),
            None,
        );
        Ok(())
    }
}
