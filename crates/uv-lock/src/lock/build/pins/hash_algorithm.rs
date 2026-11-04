use std::borrow::Cow;
use std::path::Path;

use rustc_hash::FxHashSet;

use uv_client::RegistryClient;
use uv_distribution_types::{FileLocation, FirstParty, SourceDist};
use uv_pypi_types::HashDigest;

use super::{LockError, LockErrorKind, Package, PackageId, Pin, Source};

/// Digest pairs verified during an index hash-algorithm change. Several build owners can
/// select the same artifact.
pub(super) struct HashAlgorithmChanges<'a> {
    root: &'a Path,
    client: &'a RegistryClient,
    verified: FxHashSet<(HashDigest, HashDigest)>,
}

impl<'a> HashAlgorithmChanges<'a> {
    pub(super) fn new(root: &'a Path, client: &'a RegistryClient) -> Self {
        Self {
            root,
            client,
            verified: FxHashSet::default(),
        }
    }

    pub(super) async fn verify(
        &mut self,
        pin: &Pin,
        package: &Package,
        owner: &PackageId,
        mismatch: LockError,
    ) -> Result<(), LockError> {
        let Source::Registry(registry) = &package.id.source else {
            return Err(mismatch);
        };
        let mut converted = Cow::Borrowed(pin);
        if let Some(previous) = &pin.source_hash
            && let Some(selected) = package.sdist.as_ref().and_then(|sdist| sdist.hash())
            && previous.algorithm() != selected.algorithm()
            && let Some(SourceDist::Registry(source)) =
                package.to_source_dist(self.root, FirstParty::No)?
            && self.same_file(&source.file.url, previous, selected).await?
        {
            converted.to_mut().source_hash = Some(selected.clone());
        }
        for wheel in &package.wheels {
            if let Some(previous) = pin.wheels.get(&wheel.filename)
                && let Some(selected) = &wheel.hash
                && previous.algorithm() != selected.algorithm()
            {
                let file = wheel.to_registry_wheel(registry, self.root)?.file;
                if self.same_file(&file.url, previous, selected).await? {
                    converted
                        .to_mut()
                        .wheels
                        .insert(wheel.filename.clone(), selected.clone());
                }
            }
        }
        match converted {
            Cow::Borrowed(_) => Err(mismatch),
            Cow::Owned(pin) => pin.verify(package, owner),
        }
    }

    /// The new digest can stand in for the saved one only if both describe the same bytes.
    async fn same_file(
        &mut self,
        location: &FileLocation,
        previous: &HashDigest,
        selected: &HashDigest,
    ) -> Result<bool, LockError> {
        let pair = (previous.clone(), selected.clone());
        if self.verified.contains(&pair) {
            return Ok(true);
        }
        let url = location.to_url().map_err(LockErrorKind::InvalidUrl)?;
        let [old, new] = self
            .client
            .hash_file_with(&url, [previous.algorithm(), selected.algorithm()])
            .await
            .map_err(LockErrorKind::FileHash)?;
        if previous == &old && selected == &new {
            self.verified.insert(pair);
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
