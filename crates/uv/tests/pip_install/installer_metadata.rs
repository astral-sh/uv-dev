//! Recovery of optional metadata on real installed distributions.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

use anyhow::Result;
use assert_cmd::prelude::*;
use assert_fs::fixture::ChildPath;
use assert_fs::prelude::*;
use fs_err as fs;
use fs_err::File;
use indoc::indoc;
use predicates::prelude::*;
use sha2::{Digest, Sha256};
use url::Url;

use uv_fs::write_atomic_sync;
use uv_static::EnvVars;
use uv_test::TestContext;
use uv_test::archive::write_tar_gz;
use uv_test::packse::generate_wheel_with_files;

const NAME: &str = "installed-sidecar-fixture";
const MODULE: &str = "installed_sidecar_fixture";
const VERSION: &str = "1.0.0";
const SIDECARS: [&str; 2] = ["uv_cache.json", "uv_build.json"];
const CANARY: &str = "https://user:sidecar-secret@example.invalid/a?sig=sidecar-signature";

struct InstallerMetadataTestContext {
    inner: TestContext,
    source: ChildPath,
    source_url: Url,
    source_hash: String,
    build_log: ChildPath,
    registry: bool,
}

impl InstallerMetadataTestContext {
    fn new(registry: bool) -> Result<Self> {
        let inner = uv_test::test_context!("3.12");
        let source = inner
            .temp_dir
            .child("installed_sidecar_fixture-1.0.0.tar.gz");
        let build_log = inner.temp_dir.child("backend.log");
        let name = NAME.parse()?;
        let version = VERSION.parse()?;
        let wheel = |flavor: &str| {
            generate_wheel_with_files(
                &name,
                &version,
                &[],
                &BTreeMap::new(),
                None,
                "py3-none-any",
                &[(
                    "installed_sidecar_fixture/value.py",
                    &format!("VALUE = {flavor:?}\n"),
                )],
            )
        };
        let (filename, alpha) = wheel("alpha");
        let (beta_filename, beta) = wheel("beta");
        assert_eq!(filename, beta_filename);

        let pyproject = indoc! {r#"
            [build-system]
            requires = []
            build-backend = "backend"
            backend-path = ["."]

            [project]
            name = "installed-sidecar-fixture"
            version = "1.0.0"
        "#};
        let metadata = indoc! {"
            Metadata-Version: 2.3
            Name: installed-sidecar-fixture
            Version: 1.0.0
        "};
        let backend = indoc! {r#"
            import os
            import shutil
            import zipfile
            from pathlib import Path

            DIST_INFO = "installed_sidecar_fixture-1.0.0.dist-info"
            WHEEL = "installed_sidecar_fixture-1.0.0-py3-none-any.whl"

            def source_wheel(config_settings):
                flavor = (config_settings or {}).get("flavor", "alpha")
                if isinstance(flavor, list):
                    flavor = flavor[-1]
                if flavor not in {"alpha", "beta"}:
                    raise ValueError("Unsupported fixture flavor")
                return flavor, Path(__file__).with_name(f"{flavor}.whl")

            def get_requires_for_build_wheel(config_settings=None):
                return []

            def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
                _, source = source_wheel(config_settings)
                directory = Path(metadata_directory) / DIST_INFO
                directory.mkdir(parents=True, exist_ok=True)
                with zipfile.ZipFile(source) as archive:
                    for name in ("METADATA", "WHEEL"):
                        (directory / name).write_bytes(archive.read(f"{DIST_INFO}/{name}"))
                return DIST_INFO

            def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
                flavor, source = source_wheel(config_settings)
                with open(os.environ["INSTALLER_METADATA_BUILD_LOG"], "a", encoding="utf-8") as log:
                    log.write(flavor + "\n")
                shutil.copyfile(source, Path(wheel_directory) / WHEEL)
                return WHEEL
        "#};
        write_tar_gz(
            File::create(source.path())?,
            &[
                (
                    "installed_sidecar_fixture-1.0.0/pyproject.toml",
                    pyproject.as_bytes(),
                ),
                (
                    "installed_sidecar_fixture-1.0.0/PKG-INFO",
                    metadata.as_bytes(),
                ),
                (
                    "installed_sidecar_fixture-1.0.0/backend.py",
                    backend.as_bytes(),
                ),
                (
                    "installed_sidecar_fixture-1.0.0/alpha.whl",
                    alpha.as_slice(),
                ),
                ("installed_sidecar_fixture-1.0.0/beta.whl", beta.as_slice()),
            ],
        )?;
        let source_hash = hex::encode(Sha256::digest(fs::read(source.path())?));
        let source_url = Url::from_file_path(source.path()).expect("absolute source path");
        let inner = inner
            .with_env("INSTALLER_METADATA_BUILD_LOG", build_log.path())
            .with_env(EnvVars::RUST_LOG, "warn")
            .with_filter((source_hash.clone(), "[SOURCE_HASH]"));
        let context = Self {
            inner,
            source,
            source_url,
            source_hash,
            build_log,
            registry,
        };
        context.install("alpha", false, false)?.assert().success();
        context.assert_flavor("alpha");

        let record = fs::read_to_string(context.dist_info().join("RECORD"))?;
        for sidecar in SIDECARS {
            assert!(context.sidecar(sidecar).is_file());
            assert!(record.contains(&format!("/{sidecar},")));
        }
        assert_eq!(
            context.dist_info().join("direct_url.json").exists(),
            !registry
        );
        Ok(context)
    }

    fn dist_info(&self) -> PathBuf {
        self.inner
            .site_packages()
            .join("installed_sidecar_fixture-1.0.0.dist-info")
    }

    fn sidecar(&self, name: &str) -> PathBuf {
        self.dist_info().join(name)
    }

    fn replace_sidecar(&self, name: &str, contents: &[u8]) -> Result<()> {
        // Installed files can share an inode with the package cache.
        write_atomic_sync(self.sidecar(name), contents)?;
        Ok(())
    }

    fn install(&self, flavor: &str, wrong_hash: bool, upgrade: bool) -> Result<Command> {
        let digest = if wrong_hash {
            "0".repeat(64)
        } else {
            self.source_hash.clone()
        };
        let requirement = if self.registry {
            format!("{NAME}=={VERSION}")
        } else {
            format!("{NAME} @ {}", self.source_url)
        };
        let requirements = self.inner.temp_dir.child(if wrong_hash {
            "wrong.txt"
        } else {
            "requirements.txt"
        });
        requirements.write_str(&format!("{requirement} --hash=sha256:{digest}\n"))?;
        let mut command = self.inner.pip_install();
        command
            .arg("--offline")
            .arg("--no-index")
            .arg("--no-deps")
            .arg("--require-hashes")
            .arg("--requirements")
            .arg(requirements.path())
            .arg("--config-setting")
            .arg(format!("flavor={flavor}"));
        if self.registry {
            command
                .arg("--find-links")
                .arg(self.source.path().parent().expect("source parent"));
        }
        if upgrade {
            command.arg("--upgrade");
        }
        Ok(command)
    }

    fn assert_flavor(&self, flavor: &str) {
        self.inner
            .assert_command(&format!(
                "from {MODULE}.value import VALUE; assert VALUE == {flavor:?}"
            ))
            .success();
    }

    fn builds(&self) -> Result<String> {
        Ok(fs::read_to_string(self.build_log.path())?)
    }

    fn assert_repaired(&self, flavor: &str, allow_missing_cache: bool) -> Result<()> {
        for sidecar in SIDECARS {
            let path = self.sidecar(sidecar);
            if sidecar == "uv_cache.json" && allow_missing_cache && !path.try_exists()? {
                // Registry-indexed built wheels can have empty cache information.
                continue;
            }
            let value: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
            assert!(value.is_object());
            if sidecar == "uv_build.json" {
                assert_eq!(value["config_settings"]["flavor"], flavor);
            }
        }
        Ok(())
    }
}

#[test]
fn malformed_optional_metadata_can_be_inspected() -> Result<()> {
    for sidecar in SIDECARS {
        for contents in [b"{".to_vec(), serde_json::to_vec(CANARY)?] {
            let context = InstallerMetadataTestContext::new(false)?;
            context.replace_sidecar(sidecar, &contents)?;
            let before = fs::read(context.sidecar(sidecar))?;
            let mut list = context.inner.pip_list();
            list.arg("--format=json");
            let mut show = context.inner.pip_show();
            show.arg(NAME);
            let mut tree = context.inner.pip_tree();
            tree.arg("--package").arg(NAME);
            for mut command in [list, show, tree] {
                command
                    .assert()
                    .success()
                    .stdout(predicate::str::contains(NAME))
                    .stderr(predicate::str::contains(sidecar))
                    .stderr(predicate::str::contains("invalid JSON data"))
                    .stderr(predicate::str::contains("sidecar-secret").not())
                    .stderr(predicate::str::contains("sidecar-signature").not());
            }
            assert_eq!(fs::read(context.sidecar(sidecar))?, before);
            context.assert_flavor("alpha");
        }
    }
    Ok(())
}

#[test]
fn optional_metadata_read_errors_remain_errors() -> Result<()> {
    for sidecar in SIDECARS {
        let context = InstallerMetadataTestContext::new(false)?;
        fs::remove_file(context.sidecar(sidecar))?;
        fs::create_dir(context.sidecar(sidecar))?;
        context
            .inner
            .pip_list()
            .assert()
            .failure()
            .stderr(predicate::str::contains(sidecar))
            .stderr(predicate::str::contains("invalid JSON data").not());
        context.assert_flavor("alpha");
    }
    Ok(())
}

#[test]
fn malformed_optional_metadata_does_not_replace_required_metadata() -> Result<()> {
    let context = InstallerMetadataTestContext::new(false)?;
    context.replace_sidecar("uv_cache.json", b"{")?;
    write_atomic_sync(
        context.dist_info().join("METADATA"),
        b"Metadata-Version: 2.3\nVersion: 1.0.0\n",
    )?;
    context.inner.pip_list().assert().success();
    context
        .inner
        .pip_tree()
        .arg("--package")
        .arg(NAME)
        .assert()
        .failure()
        .stdout("")
        .stderr(predicate::str::contains("METADATA"));
    context.assert_flavor("alpha");
    Ok(())
}

#[test]
fn absent_build_metadata_keeps_legacy_reuse() -> Result<()> {
    for registry in [false, true] {
        let context = InstallerMetadataTestContext::new(registry)?;
        let builds = context.builds()?;
        if registry {
            // The stateful registry resolver can reuse installed versions with different settings.
            context.install("beta", false, false)?.assert().success();
            context.assert_flavor("alpha");
        }
        fs::remove_file(context.sidecar("uv_build.json"))?;
        context.install("beta", true, false)?.assert().success();
        context.assert_flavor("alpha");
        assert_eq!(context.builds()?, builds);
    }
    Ok(())
}

#[test]
fn malformed_build_metadata_rechecks_direct_url_hashes() -> Result<()> {
    let context = InstallerMetadataTestContext::new(false)?;
    let builds = context.builds()?;
    context.replace_sidecar("uv_build.json", b"{")?;
    context
        .install("beta", true, false)?
        .assert()
        .failure()
        .stderr(predicate::str::contains("Hash mismatch"));
    context.assert_flavor("alpha");
    assert_eq!(context.builds()?, builds);
    assert_eq!(fs::read(context.sidecar("uv_build.json"))?.as_slice(), b"{");

    context
        .install("beta", false, false)?
        .assert()
        .success()
        .stderr(predicate::str::contains("Installed 1 package"));
    context.assert_flavor("beta");
    context.assert_repaired("beta", false)?;
    Ok(())
}

#[test]
fn malformed_registry_metadata_rechecks_hashes() -> Result<()> {
    for sidecar in SIDECARS {
        for upgrade in [false, true] {
            let context = InstallerMetadataTestContext::new(true)?;
            let builds = context.builds()?;
            let flavor = if sidecar == "uv_build.json" {
                "beta"
            } else {
                "alpha"
            };
            context.replace_sidecar(sidecar, b"{")?;
            context
                .install(flavor, true, upgrade)?
                .assert()
                .failure()
                .stderr(predicate::str::contains("Hash mismatch"));
            context.assert_flavor("alpha");
            assert_eq!(context.builds()?, builds);
            assert_eq!(fs::read(context.sidecar(sidecar))?.as_slice(), b"{");

            context
                .install(flavor, false, upgrade)?
                .assert()
                .success()
                .stderr(predicate::str::contains("Installed 1 package"));
            context.assert_flavor(flavor);
            context.assert_repaired(flavor, sidecar == "uv_cache.json")?;

            let builds = context.builds()?;
            context
                .install(flavor, false, upgrade)?
                .assert()
                .success()
                .stderr(predicate::str::contains("Installed 1 package").not());
            assert_eq!(context.builds()?, builds);
        }
    }
    Ok(())
}
