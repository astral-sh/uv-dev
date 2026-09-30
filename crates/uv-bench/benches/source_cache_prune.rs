//! Prune caches containing real source-distribution builds.

mod common;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use criterion::{
    BatchSize, BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main,
    measurement::WallTime,
};
use sha2::{Digest, Sha256};
use uv_bench::{
    FixtureServer, copy_cache, fixture_path, is_codspeed_simulation, run_command,
    uv_command_with_cache,
};
use uv_cache::{Cache, CacheBucket};

#[derive(Clone, Copy)]
enum ArchiveLocation {
    Path,
    Http,
}

impl ArchiveLocation {
    fn name(self) -> &'static str {
        match self {
            Self::Path => "path",
            Self::Http => "http",
        }
    }

    fn pointer(self) -> &'static str {
        match self {
            Self::Path => "revision.rev",
            Self::Http => "revision.http",
        }
    }

    fn requirement(self, server: &FixtureServer, archive: &str) -> OsString {
        match self {
            Self::Path => std::path::absolute(fixture_path(archive))
                .expect("Failed to locate source archive")
                .into_os_string(),
            Self::Http => server.url(&format!("/files/{archive}")).into(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum SourceEntry {
    Directory,
    File { bytes: u64, digest: [u8; 32] },
    Symlink(PathBuf),
}

fn source_entries(root: &Path) -> BTreeMap<PathBuf, SourceEntry> {
    let mut result = BTreeMap::new();
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        for entry in fs_err::read_dir(directory).expect("Failed to read source directory") {
            let entry = entry.expect("Failed to read source entry");
            let path = entry.path();
            let file_type = entry.file_type().expect("Failed to inspect source entry");
            let value = if file_type.is_dir() {
                directories.push(path.clone());
                SourceEntry::Directory
            } else if file_type.is_file() {
                let contents = fs_err::read(&path).expect("Failed to read source file");
                SourceEntry::File {
                    bytes: contents.len() as u64,
                    digest: Sha256::digest(contents).into(),
                }
            } else {
                assert!(file_type.is_symlink(), "Unsupported source entry");
                SourceEntry::Symlink(fs_err::read_link(&path).expect("Failed to read source link"))
            };
            result.insert(
                path.strip_prefix(root)
                    .expect("Source entry is outside its root")
                    .to_path_buf(),
                value,
            );
        }
    }
    result
}

struct RetainedSourceCache {
    _directory: tempfile::TempDir,
    cache: PathBuf,
}

impl RetainedSourceCache {
    fn prepare(
        server: &FixtureServer,
        archive: &str,
        module: &str,
        location: ArchiveLocation,
    ) -> Self {
        let directory = tempfile::tempdir().expect("Failed to create source cache");
        let cache = directory.path().join("cache");
        run_command(
            server
                .command(&cache)
                .args(["pip", "install", "--no-deps", "--no-index", "--find-links"])
                .arg(
                    std::path::absolute("../../.cache/bench-fixtures")
                        .expect("Failed to locate backend wheels"),
                )
                .arg("--build-constraints")
                .arg(
                    std::path::absolute("../../scripts/benchmark/source-build-constraints.txt")
                        .expect("Failed to locate build constraints"),
                )
                .args(["--link-mode", "hardlink", "--target"])
                .arg(directory.path().join("environment"))
                .arg(location.requirement(server, archive)),
        );

        let bucket = Cache::from_path(&cache).bucket(CacheBucket::SourceDistributions);
        let mut directories = vec![bucket];
        let mut sources = Vec::new();
        while let Some(directory) = directories.pop() {
            if directory.join("pyproject.toml").is_file() && directory.join(module).is_file() {
                sources.push(directory);
                continue;
            }
            for entry in fs_err::read_dir(directory).expect("Failed to read source cache") {
                let entry = entry.expect("Failed to read source-cache entry");
                if entry
                    .file_type()
                    .expect("Failed to inspect source-cache entry")
                    .is_dir()
                {
                    directories.push(entry.path());
                }
            }
        }
        assert_eq!(sources.len(), 1, "Expected one retained project source");
        let source = &sources[0];
        let revision = source.parent().expect("Missing source revision directory");
        let pointer = revision
            .parent()
            .expect("Missing source revision root")
            .join(location.pointer());
        assert!(pointer.is_file(), "Missing source revision pointer");
        let before = source_entries(source);
        assert!(!before.is_empty(), "Retained source tree is empty");

        // Normalize unrelated cache entries before timing a retained-source no-op prune.
        run_command(uv_command_with_cache(&cache).args(["--offline", "cache", "prune"]));
        assert_eq!(source_entries(source), before, "Source payload changed");
        Self {
            _directory: directory,
            cache,
        }
    }

    fn copy(&self) -> tempfile::TempDir {
        let directory = tempfile::tempdir().expect("Failed to create source-cache copy");
        copy_cache(&self.cache, directory.path()).expect("Failed to copy retained source cache");
        directory
    }
}

fn retained_source_cache(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let server = FixtureServer::start(&[]);
    let mut group = c.benchmark_group("cache_prune_retained_source");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    for (name, archive, module) in [
        ("flask", "flask-3.1.1.tar.gz", "src/flask/__init__.py"),
        ("django", "django-5.2.6.tar.gz", "django/__init__.py"),
    ] {
        for location in [ArchiveLocation::Path, ArchiveLocation::Http] {
            let source = RetainedSourceCache::prepare(&server, archive, module, location);
            group.bench_function(BenchmarkId::new(location.name(), name), |b| {
                b.iter_batched(
                    || {
                        let cache = source.copy();
                        let mut command = uv_command_with_cache(cache.path());
                        command.args(["--offline", "cache", "prune"]);
                        (cache, command)
                    },
                    |(cache, mut command)| {
                        run_command(&mut command);
                        cache
                    },
                    BatchSize::PerIteration,
                );
            });
        }
    }
    group.finish();
}

criterion_group! {
    name = sources;
    config = common::walltime_criterion();
    targets = retained_source_cache
}
criterion_main!(sources);
