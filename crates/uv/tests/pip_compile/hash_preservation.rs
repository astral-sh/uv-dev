//! Reuse and regeneration of hashes from an existing requirements file.

use anyhow::Result;
use assert_cmd::prelude::*;
use assert_fs::prelude::*;
use indoc::indoc;

use uv_test::TestContext;
use uv_test::packse::PackseServer;
use uv_test::packse::scenario::Scenario;

struct HashFixture {
    context: TestContext,
    index: PackseServer,
}

impl HashFixture {
    fn new() -> Self {
        let scenario: Scenario = toml::from_str(indoc! {r#"
            name = "hash-preservation"

            [root]
            [expected]
            satisfiable = true

            [packages.a.versions."1.0.0"]
            wheel_tags = ["py2-none-any", "py3-none-any"]

            [packages.a.versions."2.0.0"]
            wheel_tags = ["py2-none-any", "py3-none-any"]
        "#})
        .expect("valid hash fixture scenario");
        Self {
            context: uv_test::test_context!("3.12"),
            index: PackseServer::from_scenario(&scenario),
        }
    }

    /// The fixture publishes a compatible wheel, an incompatible wheel, and one source archive.
    fn hashes(&self, version: &str) -> [String; 3] {
        let filenames = [
            format!("a-{version}-py2-none-any.whl"),
            format!("a-{version}-py3-none-any.whl"),
            format!("a-{version}.tar.gz"),
        ];
        let mut hashes = filenames.map(|filename| {
            self.index
                .files()
                .find(|(candidate, _)| *candidate == filename.as_str())
                .unwrap_or_else(|| panic!("fixture index does not contain {filename}"))
                .1
                .to_owned()
        });
        hashes.sort_unstable();
        assert!(
            hashes.windows(2).all(|pair| pair[0] != pair[1]),
            "fixture artifact hashes must differ"
        );
        hashes
    }

    fn compile(&self, requirement: &str, previous: &str, args: &[&str]) -> Result<String> {
        self.context
            .temp_dir
            .child("requirements.in")
            .write_str(requirement)?;
        self.context
            .temp_dir
            .child("requirements.txt")
            .write_str(previous)?;

        let output = self
            .context
            .pip_compile()
            .arg("requirements.in")
            .arg("--output-file=requirements.txt")
            .arg("--generate-hashes")
            .arg("--no-header")
            .arg("--no-annotate")
            .arg("--index-url")
            .arg(self.index.index_url())
            .args(args)
            .assert()
            .success();
        let contents = self.context.read("requirements.txt");
        output.stdout(contents.clone());
        Ok(contents)
    }
}

fn expected_requirement(version: &str, hashes: &[String]) -> String {
    let mut lines = vec![format!("a=={version}")];
    lines.extend(hashes.iter().map(|hash| format!("--hash=sha256:{hash}")));
    format!("{}\n", lines.join(" \\\n    "))
}

#[test]
fn preserve_hashes_no_upgrade() -> Result<()> {
    let fixture = HashFixture::new();
    let hashes = fixture.hashes("1.0.0");
    let previous = expected_requirement("1.0.0", &hashes[..2]);
    let contents = fixture.compile("a", &previous, &[])?;
    assert_eq!(contents, previous);
    Ok(())
}

#[test]
fn preserve_hashes_upgrade() -> Result<()> {
    let fixture = HashFixture::new();
    let hashes = fixture.hashes("1.0.0");
    let previous = expected_requirement("1.0.0", &hashes[..2]);
    let contents = fixture.compile("a==1.0.0", &previous, &["--upgrade"])?;
    assert_eq!(contents, expected_requirement("1.0.0", &hashes));
    Ok(())
}

#[test]
fn preserve_hashes_no_existing_hashes() -> Result<()> {
    let fixture = HashFixture::new();
    let hashes = fixture.hashes("1.0.0");
    let contents = fixture.compile("a", "a==1.0.0", &[])?;
    assert_eq!(contents, expected_requirement("1.0.0", &hashes));
    Ok(())
}

#[test]
fn preserve_hashes_newer_version() -> Result<()> {
    let fixture = HashFixture::new();
    let old_hashes = fixture.hashes("1.0.0");
    let new_hashes = fixture.hashes("2.0.0");
    let previous = expected_requirement("1.0.0", &old_hashes[..2]);
    let contents = fixture.compile("a==2.0.0", &previous, &[])?;
    assert_eq!(contents, expected_requirement("2.0.0", &new_hashes));
    Ok(())
}
