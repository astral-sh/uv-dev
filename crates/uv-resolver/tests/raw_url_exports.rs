use std::path::Path;

use serde_json::{Value, json};

use uv_configuration::{DependencyGroupsWithDefaults, ExtrasSpecification, InstallOptions};
use uv_normalize::{DefaultExtras, PackageName};
use uv_preview::Preview;
use uv_resolver::{Installable, Lock, Metadata, cyclonedx_json};

const FIRST_URL: &str = "https://reader:dummy-first@example.test/shared-1.0.tar.gz?X-Amz-Signature=dummy-first#fragment";
const SECOND_URL: &str = "https://reader:dummy-second@example.test/shared-1.0.tar.gz?X-Amz-Signature=dummy-second#fragment";
const REGISTRY_URL: &str =
    "https://reader:dummy-index@example.test/simple?sig=dummy-index#fragment";
const GIT_URL: &str = "https://reader:dummy-git@example.test/repo.git?branch=main#0123456789abcdef0123456789abcdef01234567";
const SDIST_URL: &str = "https://reader:dummy-sdist@example.test/indexed-1.0.tar.gz?X-Amz%2DSignature=dummy-sdist&sig=dummy-sas#fragment";
const WHEEL_URL: &str = "https://reader:dummy-wheel@example.test/indexed-1.0-py3-none-any.whl?X-Amz-Signature=dummy-wheel#fragment";
const HASH: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn lock() -> Lock {
    Lock::from_toml(&format!(
        r#"version = 1
revision = 3
requires-python = ">=3.12"

[manifest]
members = ["root"]

[[package]]
name = "indexed"
version = "1.0"
source = {{ registry = "{REGISTRY_URL}" }}
sdist = {{ url = "{SDIST_URL}", hash = "{HASH}" }}
wheels = [{{ url = "{WHEEL_URL}", hash = "{HASH}" }}]

[[package]]
name = "repository"
version = "1.0"
source = {{ git = "{GIT_URL}" }}

[[package]]
name = "root"
version = "1.0"
source = {{ virtual = "." }}
dependencies = [
    {{ name = "indexed" }},
    {{ name = "repository" }},
    {{ name = "shared", version = "1.0", source = {{ url = "{FIRST_URL}" }}, marker = "sys_platform == 'win32'" }},
    {{ name = "shared", version = "1.0", source = {{ url = "{SECOND_URL}" }}, marker = "sys_platform != 'win32'" }},
]

[[package]]
name = "shared"
version = "1.0"
source = {{ url = "{FIRST_URL}" }}

[[package]]
name = "shared"
version = "1.0"
source = {{ url = "{SECOND_URL}" }}
"#,
    ))
    .expect("valid authored lock")
}

struct Target<'lock> {
    lock: &'lock Lock,
    root: PackageName,
}

impl<'lock> Installable<'lock> for Target<'lock> {
    fn install_path(&self) -> &'lock Path {
        Path::new("")
    }

    fn lock(&self) -> &'lock Lock {
        self.lock
    }

    fn roots(&self) -> impl Iterator<Item = &PackageName> {
        std::iter::once(&self.root)
    }

    fn project_name(&self) -> Option<&PackageName> {
        Some(&self.root)
    }
}

#[test]
fn lock_and_metadata_keep_raw_urls() {
    let lock = lock();
    let serialized = lock.to_toml().expect("serialized lock");
    let reparsed = Lock::from_toml(&serialized).expect("round-tripped lock");
    assert_eq!(lock, reparsed);
    assert_eq!(serialized, reparsed.to_toml().expect("serialized lock"));

    let metadata = Metadata::from_script(Path::new("script.py"), &lock).expect("script metadata");
    let metadata_bytes = serde_json::to_vec(&metadata).expect("serialized metadata");
    let metadata: Value = serde_json::from_slice(&metadata_bytes).expect("valid metadata JSON");
    let resolution = metadata["resolution"]
        .as_object()
        .expect("resolution graph");

    for (name, source_kind, source_pointer, url) in [
        ("indexed", "registry", "/source/registry/url", REGISTRY_URL),
        ("repository", "git", "/source/git", GIT_URL),
        ("shared", "direct", "/source/url", FIRST_URL),
        ("shared", "direct", "/source/url", SECOND_URL),
    ] {
        let id = format!("{name}==1.0@{source_kind}+{url}");
        let node = resolution.get(&id).expect("distinct raw source ID");
        assert_eq!(node.pointer(source_pointer), Some(&json!(url)));
    }

    let indexed = &resolution[&format!("indexed==1.0@registry+{REGISTRY_URL}")];
    assert_eq!(indexed["sdist"]["url"], SDIST_URL);
    assert_eq!(indexed["wheels"][0]["url"], WHEEL_URL);

    let metadata = Metadata::from_script(Path::new("script.py"), &reparsed)
        .expect("round-tripped script metadata");
    assert_eq!(
        serde_json::to_vec(&metadata).expect("serialized metadata"),
        metadata_bytes
    );
}

#[test]
fn cyclonedx_distribution_references_keep_raw_urls() {
    let lock = lock();
    let target = Target {
        lock: &lock,
        root: "root".parse().expect("valid root name"),
    };
    let install_options = InstallOptions::default();
    let bom = cyclonedx_json::from_lock(
        &target,
        &[],
        &ExtrasSpecification::default().with_defaults(DefaultExtras::default()),
        &DependencyGroupsWithDefaults::none(),
        true,
        true,
        &install_options,
        Preview::all(),
        false,
    )
    .expect("exported CycloneDX");
    let mut bytes = Vec::new();
    bom.output_as_json_v1_5(&mut bytes)
        .expect("serialized CycloneDX");
    let document: Value = serde_json::from_slice(&bytes).expect("valid CycloneDX JSON");
    let indexed = document["components"]
        .as_array()
        .expect("components")
        .iter()
        .find(|component| component["name"] == "indexed")
        .expect("indexed component");
    assert_eq!(
        indexed["externalReferences"],
        json!([
            {"type": "distribution", "url": SDIST_URL, "hashes": [{"alg": "SHA-256", "content": &HASH[7..]}]},
            {"type": "distribution", "url": WHEEL_URL, "hashes": [{"alg": "SHA-256", "content": &HASH[7..]}]},
        ])
    );
}
