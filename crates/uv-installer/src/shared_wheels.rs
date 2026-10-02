use std::sync::Arc;

use uv_distribution_types::{ArchiveHashPolicy, BuiltDist, CachedDist};
use uv_once_map::OnceMap;
use uv_pypi_types::HashDigest;

type DownloadResult = Result<CachedDist, Arc<uv_distribution::Error>>;

/// Remote wheels that can be reused by independent builds for the same interpreter.
#[derive(Default, Clone)]
pub struct SharedWheelDownloads(Arc<OnceMap<WheelDownloadId, DownloadResult>>);

#[derive(Clone, PartialEq, Eq, Hash)]
struct WheelDownloadId {
    dist: Box<BuiltDist>,
    hashes: OwnedArchiveHashPolicy,
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum OwnedArchiveHashPolicy {
    None,
    Generate,
    Any(Box<[HashDigest]>),
    All(Box<[HashDigest]>),
}

impl From<ArchiveHashPolicy<'_>> for OwnedArchiveHashPolicy {
    fn from(policy: ArchiveHashPolicy<'_>) -> Self {
        match policy {
            ArchiveHashPolicy::None => Self::None,
            ArchiveHashPolicy::Generate => Self::Generate,
            ArchiveHashPolicy::Any(hashes) => Self::Any(hashes.into()),
            ArchiveHashPolicy::All(hashes) => Self::All(hashes.into()),
        }
    }
}

impl SharedWheelDownloads {
    pub(crate) fn download(
        &self,
        dist: &BuiltDist,
        hashes: ArchiveHashPolicy<'_>,
    ) -> Option<WheelDownload<'_>> {
        let is_remote = match dist {
            BuiltDist::Registry(registry) => registry
                .best_wheel()
                .file
                .url
                .to_url()
                .is_ok_and(|url| matches!(url.scheme(), "http" | "https")),
            BuiltDist::DirectUrl(wheel) => matches!(wheel.location.scheme(), "http" | "https"),
            BuiltDist::Path(_) | BuiltDist::GitPath(_) => false,
        };
        if !is_remote {
            return None;
        }

        // A distribution ID can omit advertised hashes, sizes, and the index origin.
        // Independent resolutions may also add different trusted requirement hashes.
        Some(WheelDownload {
            downloads: &self.0,
            id: WheelDownloadId {
                dist: Box::new(dist.clone()),
                hashes: hashes.into(),
            },
        })
    }
}

pub(crate) struct WheelDownload<'a> {
    downloads: &'a OnceMap<WheelDownloadId, DownloadResult>,
    id: WheelDownloadId,
}

impl WheelDownload<'_> {
    pub(crate) async fn register_or_wait(&self) -> Option<DownloadResult> {
        self.downloads.register_or_wait(&self.id).await
    }

    pub(crate) fn done(self, result: DownloadResult) {
        self.downloads.done(self.id, result);
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};
    use uv_distribution_types::DirectUrlBuiltDist;

    use super::*;

    fn wheel(url: &str) -> Result<BuiltDist> {
        Ok(BuiltDist::DirectUrl(DirectUrlBuiltDist {
            filename: "example-1.0.0-py3-none-any.whl".parse()?,
            location: Box::new(url.parse()?),
            url: url.parse()?,
            size: Some(123),
        }))
    }

    #[test]
    fn shared_wheels_keep_final_hash_policies_separate() -> Result<()> {
        let first = SharedWheelDownloads::default();
        let second = first.clone();
        let wheel = wheel("https://example.org/example-1.0.0-py3-none-any.whl")?;
        let hashes = [format!("sha256:{}", "0".repeat(64)).parse()?];
        let other_hashes = [format!("sha256:{}", "1".repeat(64)).parse()?];
        for policy in [
            ArchiveHashPolicy::None,
            ArchiveHashPolicy::Generate,
            ArchiveHashPolicy::Any(&hashes),
            ArchiveHashPolicy::All(&hashes),
            ArchiveHashPolicy::Any(&other_hashes),
        ] {
            let entry = second.download(&wheel, policy).context("remote wheel")?;
            assert!(entry.downloads.get(&entry.id).is_none());
            let error = Arc::new(uv_distribution::Error::NoBuild);
            first
                .download(&wheel, policy)
                .context("remote wheel")?
                .done(Err(error.clone()));
            let result = entry.downloads.get(&entry.id).context("cached result")?;
            assert!(result.is_err_and(|cached| Arc::ptr_eq(&cached, &error)));
        }
        Ok(())
    }

    #[test]
    fn shared_wheels_skip_local_files() -> Result<()> {
        let downloads = SharedWheelDownloads::default();
        let wheel = wheel("file:///example-1.0.0-py3-none-any.whl")?;
        assert!(
            downloads
                .download(&wheel, ArchiveHashPolicy::None)
                .is_none()
        );
        Ok(())
    }
}
