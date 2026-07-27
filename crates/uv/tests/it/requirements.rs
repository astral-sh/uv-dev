use anyhow::Result;
use assert_fs::prelude::*;
use indoc::indoc;

use uv_client::BaseClientBuilder;
use uv_requirements::{RequirementsSource, RequirementsSpecification};
#[cfg(all(feature = "test-python", feature = "test-pypi"))]
use uv_test::uv_snapshot;

#[tokio::test]
async fn constraint_specifications_preserve_hashes() -> Result<()> {
    let temp_dir = assert_fs::TempDir::new()?;
    let constraints_txt = temp_dir.child("constraints.txt");
    constraints_txt.write_str(indoc! {r"
        packaging==23.2 \
            --hash=sha256:1111111111111111111111111111111111111111111111111111111111111111
        hatchling==1.20.0 \
            --hash=sha256:2222222222222222222222222222222222222222222222222222222222222222 \
            --hash=sha256:3333333333333333333333333333333333333333333333333333333333333333
    "})?;

    let specification = RequirementsSpecification::from_sources(
        &[],
        &[RequirementsSource::RequirementsTxt(
            constraints_txt.to_path_buf(),
        )],
        &[],
        &[],
        None,
        &BaseClientBuilder::default(),
    )
    .await?;

    insta::assert_debug_snapshot!(
        specification
            .constraints
            .iter()
            .map(|entry| (entry.requirement.to_string(), entry.hashes.as_slice()))
            .collect::<Vec<_>>(),
        @r#"
    [
        (
            "packaging==23.2",
            [
                "sha256:1111111111111111111111111111111111111111111111111111111111111111",
            ],
        ),
        (
            "hatchling==1.20.0",
            [
                "sha256:2222222222222222222222222222222222222222222222222222222222222222",
                "sha256:3333333333333333333333333333333333333333333333333333333333333333",
            ],
        ),
    ]
    "#
    );

    Ok(())
}

/// Requirements can continue between the package name and version specifier.
#[cfg(all(feature = "test-python", feature = "test-pypi"))]
#[test]
fn pip_install_continued_version_specifier() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(concat!("idna \\", "\n    >1.0.0\n"))?;

    uv_snapshot!(context.pip_install()
        .arg("--dry-run")
        .arg("-r")
        .arg("requirements.txt"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Would download 1 package
    Would install 1 package
     + idna==3.6
    "
    );

    Ok(())
}
