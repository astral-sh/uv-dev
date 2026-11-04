//! Owner-specific versions and artifact hashes used when updating a build lock.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use uv_client::RegistryClient;
use uv_configuration::Upgrade;
use uv_distribution_filename::WheelFilename;
use uv_distribution_types::{IndexUrl, Name, ResolvedDist, VersionId};
use uv_normalize::PackageName;
use uv_pep440::Version;
use uv_pep508::MarkerTree;
use uv_pypi_types::HashDigest;
use uv_resolver_types::UniversalMarker;
use uv_types::{HashStrategy, LockedArtifactHashes};

use self::hash_algorithm::HashAlgorithmChanges;
use super::{Lock, LockError, LockErrorKind, Package, PackageId, Source, error};

mod hash_algorithm;

/// Previous build selections, partitioned by exact source owner and marker domain.
#[derive(Debug, Default)]
pub struct BuildLockPins {
    owners: BTreeMap<PackageId, Vec<(MarkerTree, BuildPins)>>,
}

/// The selections that a batch of owners can share in one build solve.
#[derive(Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct BuildPins(BTreeMap<PackageId, Arc<Pin>>);

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Pin {
    index: Option<IndexUrl>,
    identity: Option<VersionId>,
    source_hash: Option<HashDigest>,
    wheels: BTreeMap<WheelFilename, HashDigest>,
}

impl PackageId {
    pub(super) fn build_owner(owner: &ResolvedDist, root: &Path) -> Result<Self, LockError> {
        Ok(Self {
            name: owner.name().clone(),
            source: Source::from_resolved_dist(owner, root)?,
            version: owner.version().cloned(),
        })
    }
}

fn package_domain(package: &Package) -> MarkerTree {
    if package.fork_markers.is_empty() {
        MarkerTree::TRUE
    } else {
        package
            .fork_markers
            .iter()
            .map(|marker| marker.combined())
            .fold(MarkerTree::FALSE, MarkerTree::or)
    }
}

impl Lock {
    /// Recover build pins from either explicit descriptors or ordinary package roots.
    pub fn build_pins(&self, root: &Path, upgrade: &Upgrade) -> Result<BuildLockPins, LockError> {
        let mut pins = BuildLockPins::default();
        let Some(builds) = &self.builds else {
            return Ok(pins);
        };
        if upgrade.is_all() {
            return Ok(pins);
        }
        let excluded = self.upgrade_packages(upgrade);
        let project = self.build_project_domain();
        for universe in &builds.universes {
            let domain = project.and(universe.marker.combined());
            let mut scopes = vec![(domain, BuildPins::default())];
            for package in universe
                .packages
                .values()
                .filter(|package| !excluded.contains(package.name()))
            {
                let applicable = package_domain(package);
                let pin = Arc::new(Pin {
                    index: package.index(root)?,
                    identity: package.id.hash_identity(root)?,
                    source_hash: package
                        .sdist
                        .as_ref()
                        .and_then(|sdist| sdist.hash())
                        .cloned(),
                    wheels: package
                        .wheels
                        .iter()
                        .filter_map(|wheel| Some((wheel.filename.clone(), wheel.hash.clone()?)))
                        .collect(),
                });
                // Partition only when an old fork actually selected different packages.
                let mut rest = Vec::new();
                for (scope, pins) in &mut scopes {
                    let selected = scope.and(applicable);
                    if selected.is_false() {
                        continue;
                    }
                    let other = scope.and(applicable.negate());
                    if !other.is_false() {
                        rest.push((other, pins.clone()));
                    }
                    *scope = selected;
                    pins.0.insert(package.id.clone(), Arc::clone(&pin));
                }
                scopes.extend(rest);
            }
            pins.owners
                .entry(universe.owner.clone())
                .or_default()
                .extend(scopes);
        }
        Ok(pins)
    }
}

impl BuildLockPins {
    /// Split a newly discovered owner at selection boundaries in the previous lock. Newly
    /// covered environments and owners have no previous selections.
    pub fn for_owner(
        &self,
        owner: &ResolvedDist,
        root: &Path,
        scope: UniversalMarker,
        project: MarkerTree,
    ) -> Result<Vec<(UniversalMarker, Option<&BuildPins>)>, LockError> {
        if self.owners.is_empty() {
            return Ok(vec![(scope, None)]);
        }
        let owner = PackageId::build_owner(owner, root)?;
        let Some(previous) = self.owners.get(&owner) else {
            return Ok(vec![(scope, None)]);
        };
        let mut remaining = scope.combined().and(project);
        let mut scopes = Vec::new();
        for (domain, pins) in previous {
            let intersection = remaining.and(*domain);
            if !intersection.is_false() {
                scopes.push((
                    UniversalMarker::from_combined(intersection.restrict(project)),
                    Some(pins),
                ));
                remaining = remaining.and(domain.negate());
            }
        }
        if !remaining.is_false() {
            scopes.push((
                UniversalMarker::from_combined(remaining.restrict(project)),
                None,
            ));
        }
        Ok(scopes)
    }

    /// Verify the completed inventory, including foreign wheels and files whose digest
    /// was unavailable until the writer hashed the archive.
    pub async fn verify(
        &self,
        lock: &Lock,
        root: &Path,
        client: &RegistryClient,
    ) -> Result<(), LockError> {
        let Some(builds) = &lock.builds else {
            return Ok(());
        };
        let project = lock.build_project_domain();
        let mut changes = HashAlgorithmChanges::new(root, client);
        for universe in &builds.universes {
            let Some(previous) = self.owners.get(&universe.owner) else {
                continue;
            };
            let domain = project.and(universe.marker.combined());
            for (scope, pins) in previous {
                if domain.is_disjoint(*scope) {
                    continue;
                }
                for (id, package) in &universe.packages {
                    let Some(pin) = pins.0.get(id) else { continue };
                    if !domain.and(*scope).is_disjoint(package_domain(package))
                        && let Err(mismatch) = pin.verify(package, &universe.owner)
                    {
                        changes
                            .verify(pin, package, &universe.owner, mismatch)
                            .await?;
                    }
                }
            }
        }
        Ok(())
    }
}

impl BuildPins {
    /// Compatible pins are tried before runtime preferences for this scope.
    pub fn versions(&self) -> impl Iterator<Item = (&PackageName, &Version, Option<&IndexUrl>)> {
        self.0
            .iter()
            .filter_map(|(id, pin)| Some((&id.name, id.version.as_ref()?, pin.index.as_ref())))
    }

    /// Direct archives are identified by URL or path. Registry identities additionally
    /// require the old index so identically named packages cannot share trusted digests.
    pub fn apply_hashes(&self, base: HashStrategy) -> HashStrategy {
        let mut hashes = LockedArtifactHashes::default();
        for pin in self.0.values() {
            if let Some(identity) = &pin.identity {
                let expected = pin.hashes().cloned().collect::<Vec<_>>();
                if !expected.is_empty() {
                    hashes.insert(pin.index.as_ref(), identity.clone(), expected);
                }
            }
        }
        base.with_locked_hashes(&hashes)
    }
}

impl Pin {
    fn hashes(&self) -> impl Iterator<Item = &HashDigest> {
        self.source_hash.iter().chain(self.wheels.values())
    }

    fn verify(&self, package: &Package, owner: &PackageId) -> Result<(), LockError> {
        if self.source_hash.is_none() && self.wheels.is_empty() {
            return Ok(());
        }
        let source = package
            .sdist
            .iter()
            .map(|sdist| (self.source_hash.as_ref(), sdist.hash()));
        let wheels = package
            .wheels
            .iter()
            .map(|wheel| (self.wheels.get(&wheel.filename), wheel.hash.as_ref()));
        for (expected, actual) in source.chain(wheels) {
            let Some(actual) = actual else {
                return Err(error(format!(
                    "Missing artifact hash for `{}` in the build environment of `{owner}`",
                    package.id
                )));
            };
            let matches = match expected {
                Some(expected) => expected == actual,
                None => self.hashes().any(|hash| hash == actual),
            };
            if !matches {
                return Err(error(format!(
                    "Artifact hash for `{}` in the build environment of `{owner}` does not match the existing lockfile: {actual}",
                    package.id
                )));
            }
        }
        Ok(())
    }
}
