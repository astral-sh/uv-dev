use std::str::FromStr;

use anyhow::Result;
use serde_json::{Value, json};
use uv_configuration::{BuildOptions, NoBinary, NoBuild};
use uv_normalize::PackageName;

fn policies() -> Result<[NoBinary; 6]> {
    let alpha = PackageName::from_str("alpha-pkg")?;
    let beta = PackageName::from_str("beta-pkg")?;
    Ok([
        NoBinary::None,
        NoBinary::All,
        NoBinary::Packages(vec![]),
        NoBinary::Packages(vec![alpha.clone()]),
        NoBinary::Packages(vec![beta, alpha.clone()]),
        NoBinary::Packages(vec![alpha.clone(), alpha]),
    ])
}

fn build_policy(policy: &NoBinary) -> NoBuild {
    match policy {
        NoBinary::None => NoBuild::None,
        NoBinary::All => NoBuild::All,
        NoBinary::Packages(packages) => NoBuild::Packages(packages.clone()),
    }
}

fn expected_policy(left: &NoBinary, right: &NoBinary) -> NoBinary {
    match (left, right) {
        (NoBinary::All, _) | (_, NoBinary::All) => NoBinary::All,
        (NoBinary::None, other) | (other, NoBinary::None) => other.clone(),
        (NoBinary::Packages(left), NoBinary::Packages(right)) => {
            NoBinary::Packages(left.iter().chain(right).cloned().collect())
        }
    }
}

fn representation(policy: &NoBinary) -> Value {
    match policy {
        NoBinary::None => json!("none"),
        NoBinary::All => json!("all"),
        NoBinary::Packages(packages) => json!({ "packages": packages }),
    }
}

#[test]
fn combine_and_extend_preserve_policy_values() -> Result<()> {
    let policies = policies()?;
    for left in &policies {
        for right in &policies {
            let expected = expected_policy(left, right);
            let binary = left.clone().combine(right.clone());
            let build = build_policy(left).combine(build_policy(right));
            assert_eq!(binary, expected, "{left:?} + {right:?}");
            assert_eq!(build, build_policy(&expected), "{left:?} + {right:?}");

            let mut extended_binary = left.clone();
            extended_binary.extend(right.clone());
            let mut extended_build = build_policy(left);
            extended_build.extend(build_policy(right));
            assert_eq!(extended_binary, expected);
            assert_eq!(extended_build, build_policy(&expected));
            assert_eq!(binary.is_none(), matches!(expected, NoBinary::None));
            assert_eq!(build.is_none(), matches!(expected, NoBinary::None));
            assert_eq!(serde_json::to_value(&binary)?, representation(&expected));
            assert_eq!(serde_json::to_value(&build)?, representation(&expected));
        }
    }
    Ok(())
}

#[test]
fn build_options_combines_each_policy_independently() -> Result<()> {
    let policies = policies()?;
    let names = [
        PackageName::from_str("alpha-pkg")?,
        PackageName::from_str("beta-pkg")?,
        PackageName::from_str("unrelated")?,
    ];
    for binary_left in &policies {
        for build_left in &policies {
            for binary_right in &policies {
                for build_right in &policies {
                    let no_binary = expected_policy(binary_left, binary_right);
                    let no_build = expected_policy(build_left, build_right);
                    let expected = BuildOptions::new(no_binary.clone(), build_policy(&no_build));
                    let combined = BuildOptions::new(binary_left.clone(), build_policy(build_left))
                        .combine(binary_right.clone(), build_policy(build_right));
                    assert_eq!(combined, expected);
                    assert_eq!(combined.no_binary(), &no_binary);
                    assert_eq!(combined.no_build(), &build_policy(&no_build));
                    assert_eq!(
                        serde_json::to_value(&combined)?,
                        json!({
                            "no-binary": representation(&no_binary),
                            "no-build": representation(&no_build),
                        }),
                    );
                    for name in &names {
                        assert_eq!(
                            combined.no_binary_package(name),
                            expected.no_binary_package(name),
                        );
                        assert_eq!(
                            combined.no_build_package(name),
                            expected.no_build_package(name),
                        );
                        assert_eq!(
                            combined.no_build_requirement(Some(name)),
                            expected.no_build_requirement(Some(name)),
                        );
                    }
                    assert_eq!(
                        combined.no_build_requirement(None),
                        expected.no_build_requirement(None),
                    );
                }
            }
        }
    }
    Ok(())
}
