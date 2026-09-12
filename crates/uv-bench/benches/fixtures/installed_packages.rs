use std::path::Path;

use uv_cache_info::CacheInfo;
use uv_distribution_types::BuildInfo;

#[derive(Clone, Copy)]
pub(crate) enum Sidecars {
    Missing,
    Present,
    Mixed,
}

impl Sidecars {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Present => "present",
            Self::Mixed => "mixed",
        }
    }

    fn includes(self, index: usize, frequency: usize) -> bool {
        match self {
            Self::Missing => false,
            Self::Present => true,
            Self::Mixed => index.is_multiple_of(frequency),
        }
    }
}

/// Create valid installed wheels with controlled eager JSON sidecars.
pub(crate) fn create(root: &Path, package_count: usize, sidecars: Sidecars) {
    let cache_info =
        serde_json::to_vec(&CacheInfo::default()).expect("Failed to encode cache info");
    let build_info =
        serde_json::to_vec(&BuildInfo::default()).expect("Failed to encode build info");

    for index in 0..package_count {
        let module = format!("metadata_bench_{index:04}");
        let name = module.replace('_', "-");
        fs_err::create_dir(root.join(&module)).expect("Failed to create package directory");
        let dist_info = root.join(format!("{module}-1.0.0.dist-info"));
        fs_err::create_dir(&dist_info).expect("Failed to create dist-info directory");
        fs_err::write(
            dist_info.join("METADATA"),
            format!("Metadata-Version: 2.1\nName: {name}\nVersion: 1.0.0\n"),
        )
        .expect("Failed to write package metadata");
        fs_err::write(
            dist_info.join("WHEEL"),
            "Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n",
        )
        .expect("Failed to write wheel metadata");

        if sidecars.includes(index, 4) {
            fs_err::write(dist_info.join("uv_cache.json"), &cache_info)
                .expect("Failed to write cache info");
        }
        if sidecars.includes(index, 8) {
            fs_err::write(dist_info.join("uv_build.json"), &build_info)
                .expect("Failed to write build info");
        }
        if sidecars.includes(index, 10) {
            let direct_url = serde_json::json!({
                "url": format!("https://example.com/{module}-1.0.0-py3-none-any.whl"),
                "archive_info": {},
            });
            fs_err::write(
                dist_info.join("direct_url.json"),
                serde_json::to_vec(&direct_url).expect("Failed to encode direct URL"),
            )
            .expect("Failed to write direct URL");
        }
    }
}
