use std::collections::BTreeMap;
use std::hint::black_box;
use std::str::FromStr;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime};
use tempfile::TempDir;

use uv_cache::{Cache, CacheBucket, WheelCache};
use uv_cache_info::CacheInfo;
use uv_distribution::RegistryWheelIndex;
use uv_distribution_types::{
    BuildInfo, ConfigSettingEntry, ConfigSettingPackageEntry, ConfigSettings,
    ExtraBuildRequirement, ExtraBuildRequires, ExtraBuildVariables, Index, IndexLocations,
    IndexUrl, PackageConfigSettings,
};
use uv_normalize::PackageName;
use uv_platform_tags::{Arch, Os, Platform, Tags, TagsOptions};
use uv_types::HashStrategy;

struct RegistryCache {
    _temp_dir: TempDir,
    cache: Cache,
    package: PackageName,
    tags: Tags,
    index_locations: IndexLocations,
    config_settings: ConfigSettings,
    config_settings_package: PackageConfigSettings,
    extra_build_requires: ExtraBuildRequires,
    extra_build_variables: ExtraBuildVariables,
    versions: usize,
}

impl RegistryCache {
    fn new(versions: usize, settings: usize) -> Self {
        let temp_dir = tempfile::tempdir().expect("temporary cache");
        let cache = Cache::from_path(temp_dir.path().join("cache"));
        let package = PackageName::from_str("demo").expect("valid package name");
        let index_url = IndexUrl::parse(
            temp_dir.path().join("local-index").to_str().expect("UTF-8"),
            None,
        )
        .expect("valid index URL");
        let index_locations = IndexLocations::new(
            vec![],
            vec![Index::from_find_links(index_url.clone())],
            true,
        );
        let tags = Tags::from_env(
            Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 17,
                },
                Arch::X86_64,
            ),
            (3, 12),
            "cpython",
            (3, 12),
            TagsOptions::default(),
        )
        .expect("valid platform tags");

        let config_settings = (0..settings)
            .map(|index| {
                ConfigSettingEntry::from_str(&format!("setting-{index:05}=global-{index:05}"))
                    .expect("valid config setting")
            })
            .collect();
        let config_settings_package = (0..settings)
            .map(|index| {
                ConfigSettingPackageEntry::from_str(&format!(
                    "demo:setting-{index:05}=package-{index:05}"
                ))
                .expect("valid package config setting")
            })
            .collect();
        let extra_build_requirements = (0..settings)
            .map(|index| ExtraBuildRequirement {
                requirement: uv_pep508::Requirement::from_str(&format!(
                    "build-requirement-{index:05}>=1"
                ))
                .expect("valid build requirement")
                .into(),
                match_runtime: false,
            })
            .collect::<Vec<_>>();
        let extra_build_requires =
            ExtraBuildRequires::from_iter([(package.clone(), extra_build_requirements)]);
        let extra_build_variables = ExtraBuildVariables::from_iter([(
            package.clone(),
            (0..settings)
                .map(|index| {
                    (
                        format!("DEMO_BUILD_{index:05}"),
                        format!("value-{index:05}"),
                    )
                })
                .collect::<BTreeMap<_, _>>(),
        )]);

        let build_info = build_info_for(
            &package,
            &config_settings,
            &config_settings_package,
            &extra_build_requires,
            &extra_build_variables,
        );
        let digest = build_info.cache_shard();
        let package_shard = cache.shard(
            CacheBucket::SourceDistributions,
            WheelCache::Index(&index_url).wheel_dir(package.as_ref()),
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Tokio runtime");
        for version in 0..versions {
            let version = format!("{version}.0");
            let revision_id = format!("revision-{version}");
            let version_shard = package_shard.shard(&version);
            fs_err::create_dir_all(version_shard.as_ref()).expect("version shard");
            let revision = (CacheInfo::default(), (&revision_id, Vec::<String>::new()));
            fs_err::write(
                version_shard.entry("revision.rev"),
                rmp_serde::to_vec(&revision).expect("revision bytes"),
            )
            .expect("revision pointer");

            let mut wheel = version_shard.shard(&revision_id);
            if let Some(digest) = digest.as_deref() {
                wheel = wheel.shard(digest);
            }
            let wheel = wheel.shard(format!("demo-{version}-py3-none-any"));
            let archive = cache.build_dir().expect("temporary archive");
            runtime
                .block_on(cache.persist(archive.keep(), wheel.as_ref()))
                .expect("cached wheel");
        }

        Self {
            _temp_dir: temp_dir,
            cache,
            package,
            tags,
            index_locations,
            config_settings,
            config_settings_package,
            extra_build_requires,
            extra_build_variables,
            versions,
        }
    }

    fn invalidate_revisions(&self) {
        let indexes = self.index_locations.allowed_indexes();
        let index = indexes.first().expect("local index");
        let package_shard = self.cache.shard(
            CacheBucket::SourceDistributions,
            WheelCache::Index(index.url()).wheel_dir(self.package.as_ref()),
        );
        for version in 0..self.versions {
            fs_err::write(
                package_shard
                    .shard(format!("{version}.0"))
                    .entry("revision.rev"),
                b"invalid revision",
            )
            .expect("invalid revision pointer");
        }
    }
}

fn build_info_for(
    package: &PackageName,
    config_settings: &ConfigSettings,
    config_settings_package: &PackageConfigSettings,
    extra_build_requires: &ExtraBuildRequires,
    extra_build_variables: &ExtraBuildVariables,
) -> BuildInfo {
    let config_settings = config_settings_package.get(package).map_or_else(
        || config_settings.clone(),
        |settings| settings.clone().merge(config_settings.clone()),
    );
    let extra_build_requires = extra_build_requires
        .get(package)
        .map_or_else(Vec::new, Clone::clone);
    let extra_build_variables = extra_build_variables.get(package).cloned();
    BuildInfo::from_settings(config_settings, extra_build_requires, extra_build_variables)
}

fn registry_wheel_build_shard(c: &mut Criterion<WallTime>) {
    let mut group = c.benchmark_group("registry_wheel_build_shard");
    for (versions, settings) in [
        (0, 0),
        (1, 0),
        (64, 0),
        (256, 0),
        (64, 64),
        (64, 256),
        (64, 1_024),
        (64, 4_096),
        (256, 64),
        (1_024, 64),
        (4_096, 64),
    ] {
        let registry = RegistryCache::new(versions, settings);
        let input = format!("{versions}x{settings}");
        let hasher = HashStrategy::None;
        let mut index = RegistryWheelIndex::new(
            &registry.cache,
            &registry.tags,
            &registry.index_locations,
            &hasher,
            &registry.config_settings,
            &registry.config_settings_package,
            &registry.extra_build_requires,
            &registry.extra_build_variables,
        );
        assert_eq!(index.get(&registry.package).count(), registry.versions);

        group.bench_function(BenchmarkId::new("repeated", &input), |benchmark| {
            benchmark.iter(|| {
                black_box(
                    (0..registry.versions)
                        .map(|_| {
                            let build_info = build_info_for(
                                &registry.package,
                                &registry.config_settings,
                                &registry.config_settings_package,
                                &registry.extra_build_requires,
                                &registry.extra_build_variables,
                            );
                            let digest = build_info.cache_shard();
                            black_box((build_info.clone(), digest))
                        })
                        .count(),
                )
            });
        });
        group.bench_function(BenchmarkId::new("hoisted", &input), |benchmark| {
            benchmark.iter(|| {
                if registry.versions == 0 {
                    return black_box(0);
                }

                let build_info = build_info_for(
                    &registry.package,
                    &registry.config_settings,
                    &registry.config_settings_package,
                    &registry.extra_build_requires,
                    &registry.extra_build_variables,
                );
                let digest = build_info.cache_shard();
                for _ in 0..registry.versions {
                    black_box((build_info.clone(), &digest));
                }
                black_box(registry.versions)
            });
        });
        group.bench_function(BenchmarkId::new("full_scan", &input), |benchmark| {
            benchmark.iter(|| {
                let hasher = HashStrategy::None;
                let mut index = RegistryWheelIndex::new(
                    &registry.cache,
                    &registry.tags,
                    &registry.index_locations,
                    &hasher,
                    &registry.config_settings,
                    &registry.config_settings_package,
                    &registry.extra_build_requires,
                    &registry.extra_build_variables,
                );
                black_box(index.get(&registry.package).count())
            });
        });
    }

    let registry = RegistryCache::new(64, 64);
    registry.invalidate_revisions();
    let hasher = HashStrategy::None;
    let mut index = RegistryWheelIndex::new(
        &registry.cache,
        &registry.tags,
        &registry.index_locations,
        &hasher,
        &registry.config_settings,
        &registry.config_settings_package,
        &registry.extra_build_requires,
        &registry.extra_build_variables,
    );
    assert_eq!(index.get(&registry.package).count(), 0);
    group.bench_function("full_scan/64x64_invalid", |benchmark| {
        benchmark.iter(|| {
            let hasher = HashStrategy::None;
            let mut index = RegistryWheelIndex::new(
                &registry.cache,
                &registry.tags,
                &registry.index_locations,
                &hasher,
                &registry.config_settings,
                &registry.config_settings_package,
                &registry.extra_build_requires,
                &registry.extra_build_variables,
            );
            black_box(index.get(&registry.package).count())
        });
    });
    group.finish();
}

criterion_group!(registry_wheel_index, registry_wheel_build_shard);
criterion_main!(registry_wheel_index);
