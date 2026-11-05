//! Locate a buildable source and its cache entries before choosing a build or metadata operation.

use std::borrow::Cow;
use std::path::Path;

use uv_cache::{Cache, CacheBucket, CacheShard, WheelCache};
use uv_distribution_filename::SourceDistExtension;
use uv_distribution_types::{
    BuildableSource, DirectorySourceUrl, GitDirectorySourceUrl, GitPathSourceUrl, IndexUrl,
    PathSourceUrl, SourceDist, SourceUrl,
};
use uv_redacted::SafeUrl;

use crate::Error;

pub(super) enum SourceLocation<'a> {
    Remote {
        url: Cow<'a, SafeUrl>,
        index: Option<&'a IndexUrl>,
        subdirectory: Option<&'a Path>,
        ext: SourceDistExtension,
        shard: CacheShard,
    },
    Archive {
        url: Cow<'a, SafeUrl>,
        path: Cow<'a, Path>,
        ext: SourceDistExtension,
        shard: CacheShard,
    },
    Directory(DirectorySourceUrl<'a>),
    GitDirectory(GitDirectorySourceUrl<'a>),
    GitArchive(GitPathSourceUrl<'a>),
}

impl<'a> SourceLocation<'a> {
    pub(super) fn new(source: &'a BuildableSource<'_>, cache: &Cache) -> Result<Self, Error> {
        let bucket = CacheBucket::SourceDistributions;
        let location = match source {
            BuildableSource::Dist(SourceDist::Registry(dist)) => {
                let shard = cache.shard(
                    bucket,
                    WheelCache::Index(&dist.index)
                        .wheel_dir(dist.name.as_ref())
                        .join(dist.version.to_string()),
                );
                let url = dist.file.url.to_url()?;
                if url.scheme() == "file" {
                    let path = url
                        .to_file_path()
                        .map_err(|()| Error::NonFileUrl(url.clone()))?;
                    Self::Archive {
                        url: Cow::Owned(url),
                        path: Cow::Owned(path),
                        ext: dist.ext,
                        shard,
                    }
                } else {
                    Self::Remote {
                        url: Cow::Owned(url),
                        index: Some(&dist.index),
                        subdirectory: None,
                        ext: dist.ext,
                        shard,
                    }
                }
            }
            BuildableSource::Dist(SourceDist::DirectUrl(dist)) => Self::Remote {
                url: Cow::Borrowed(&dist.url),
                index: None,
                subdirectory: dist.subdirectory.as_deref(),
                ext: dist.ext,
                shard: cache.shard(bucket, WheelCache::Url(&dist.url).root()),
            },
            BuildableSource::Url(SourceUrl::Direct(resource)) => Self::Remote {
                url: Cow::Borrowed(resource.url),
                index: None,
                subdirectory: resource.subdirectory,
                ext: resource.ext,
                shard: cache.shard(bucket, WheelCache::Url(resource.url).root()),
            },
            BuildableSource::Dist(SourceDist::Path(dist)) => Self::archive(dist.into(), cache),
            BuildableSource::Url(SourceUrl::Path(resource)) => {
                Self::archive(resource.clone(), cache)
            }
            BuildableSource::Dist(SourceDist::Directory(dist)) => Self::Directory(dist.into()),
            BuildableSource::Url(SourceUrl::Directory(resource)) => {
                Self::Directory(resource.clone())
            }
            BuildableSource::Dist(SourceDist::GitDirectory(dist)) => {
                Self::GitDirectory(dist.into())
            }
            BuildableSource::Url(SourceUrl::GitDirectory(resource)) => {
                Self::GitDirectory(resource.clone())
            }
            BuildableSource::Dist(SourceDist::GitPath(dist)) => Self::GitArchive(dist.into()),
            BuildableSource::Url(SourceUrl::GitPath(resource)) => {
                Self::GitArchive(resource.clone())
            }
        };
        Ok(location)
    }

    fn archive(resource: PathSourceUrl<'a>, cache: &Cache) -> Self {
        Self::Archive {
            url: Cow::Borrowed(resource.url),
            path: resource.path,
            ext: resource.ext,
            shard: cache.shard(
                CacheBucket::SourceDistributions,
                WheelCache::Path(resource.url).root(),
            ),
        }
    }
}
