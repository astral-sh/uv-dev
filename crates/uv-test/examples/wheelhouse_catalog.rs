//! Generate and identify ordinary-I/O wheelhouse census inputs.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::io::{self, Write};
use std::path::Path;
use std::str::FromStr;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use uv_distribution_filename::WheelFilename;
use uv_normalize::PackageName;
use uv_pep440::Version;
use uv_test::packse::generate_wheel;

const BASE_COMMIT: &str = "cdc71a4fd8cd5aa76304c5af6e13473488283f62";
const BASE_TREE: &str = "a6dca5e1aba65d19d0c0154321484db4b28a7449";
const COUNTS: [usize; 3] = [1, 1_000, 10_000];
const VERSION: &str = "1.0.0";
const TAG: &str = "py3-none-any";
const TARGET: &str = "uv-census-target";

#[derive(Deserialize)]
struct IdentityRequest {
    filename: String,
    name: String,
    version: String,
    tags: Vec<String>,
}

#[derive(Serialize)]
struct Identity {
    filename: String,
    name: String,
    version: String,
    tags: BTreeSet<String>,
}

#[derive(Serialize)]
struct GeneratedWheel {
    #[serde(flatten)]
    identity: Identity,
    sha256: String,
    size: usize,
}

fn expanded_tags(filename: &WheelFilename) -> BTreeSet<String> {
    let mut tags = BTreeSet::new();
    for python in filename.python_tags() {
        for abi in filename.abi_tags() {
            for platform in filename.platform_tags() {
                tags.insert(format!("{python}-{abi}-{platform}"));
            }
        }
    }
    tags
}

fn identity(request: IdentityRequest) -> Result<Identity> {
    let filename = WheelFilename::from_str(&request.filename)?;
    ensure!(
        filename.to_string() == request.filename,
        "non-canonical wheel filename: {}",
        request.filename
    );
    ensure!(
        PackageName::from_str(&request.name)? == filename.name,
        "METADATA name differs from {}",
        request.filename
    );
    ensure!(
        Version::from_str(&request.version)? == filename.version,
        "METADATA version differs from {}",
        request.filename
    );
    let mut tags = BTreeSet::new();
    for tag in request.tags {
        let tagged = WheelFilename::from_str(&format!(
            "{}-{}-{tag}.whl",
            filename.name.as_dist_info_name(),
            filename.version
        ))?;
        tags.extend(expanded_tags(&tagged));
    }
    ensure!(
        tags == expanded_tags(&filename),
        "WHEEL tags differ from {}",
        request.filename
    );
    Ok(Identity {
        filename: request.filename,
        name: filename.name.to_string(),
        version: filename.version.to_string(),
        tags,
    })
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = fs_err::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    Ok(())
}

fn generate(root: &Path) -> Result<()> {
    ensure!(root.is_absolute(), "the new catalog root must be absolute");
    fs_err::create_dir(root).context("catalog root must not already exist")?;
    for count in COUNTS {
        fs_err::create_dir(root.join(format!("wheelhouse-{count}")))?;
    }
    write_new(
        &root.join("requirements.in"),
        format!("{TARGET}=={VERSION}\n").as_bytes(),
    )?;

    let version = Version::from_str(VERSION)?;
    let mut wheels = Vec::with_capacity(COUNTS[2]);
    for index in 0..COUNTS[2] {
        let name = if index == 0 {
            TARGET.to_owned()
        } else {
            format!("uv-census-filler-{index:05}")
        };
        let name = PackageName::from_str(&name)?;
        let (filename, bytes) = generate_wheel(&name, &version, &[], &BTreeMap::new(), None, TAG);
        let identity = identity(IdentityRequest {
            filename: filename.clone(),
            name: name.to_string(),
            version: VERSION.to_owned(),
            tags: vec![TAG.to_owned()],
        })?;
        for count in COUNTS {
            if index < count {
                write_new(
                    &root.join(format!("wheelhouse-{count}")).join(&filename),
                    &bytes,
                )?;
            }
        }
        wheels.push(GeneratedWheel {
            identity,
            sha256: hex::encode(Sha256::digest(&bytes)),
            size: bytes.len(),
        });
    }
    let manifest = serde_json::json!({
        "schema": 1,
        "kind": "synthetic-packse",
        "base_commit": BASE_COMMIT,
        "base_tree": BASE_TREE,
        "generator": "uv_test::packse::generate_wheel",
        "counts": COUNTS,
        "requirements": "requirements.in",
        "wheels": wheels,
    });
    write_new(
        &root.join("generation.json"),
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(())
}

fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let command = args
        .next()
        .context("expected generate or verify-identities")?;
    if command == "generate" {
        let root = args
            .next()
            .context("expected a new absolute catalog root")?;
        ensure!(args.next().is_none(), "unexpected additional argument");
        generate(Path::new(&root))
    } else if command == "verify-identities" {
        ensure!(args.next().is_none(), "unexpected additional argument");
        let requests: Vec<IdentityRequest> = serde_json::from_reader(io::stdin().lock())?;
        ensure!(requests.len() <= COUNTS[2], "too many wheel identities");
        let identities = requests
            .into_iter()
            .map(identity)
            .collect::<Result<Vec<_>>>()?;
        serde_json::to_writer(
            io::stdout().lock(),
            &serde_json::json!({
                "schema": 1,
                "base_commit": BASE_COMMIT,
                "base_tree": BASE_TREE,
                "identities": identities,
            }),
        )?;
        Ok(())
    } else {
        bail!("expected generate or verify-identities")
    }
}
