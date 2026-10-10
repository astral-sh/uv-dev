use std::fmt::Write;
use std::str::FromStr;
use std::sync::LazyLock;

use owo_colors::OwoColorize;
use rustc_hash::FxHashMap;
use version_ranges::Ranges;

use uv_distribution_types::{DerivationChain, DerivationStep};
use uv_errors::Hints;
use uv_normalize::PackageName;
use uv_pep440::{Version, strip_local_version_sentinels};

static SUGGESTIONS: LazyLock<FxHashMap<PackageName, PackageName>> = LazyLock::new(|| {
    let suggestions: Vec<(String, String)> =
        serde_json::from_str(include_str!("suggestions.json")).unwrap();
    suggestions
        .iter()
        .map(|(k, v)| {
            (
                PackageName::from_str(k).unwrap(),
                PackageName::from_str(v).unwrap(),
            )
        })
        .collect()
});

/// Format package context that should follow a distribution error as hints.
pub fn dist_hints(
    name: &PackageName,
    version: Option<&Version>,
    chain: &DerivationChain,
    cause_hints: Hints<'_>,
) -> Hints<'static> {
    let mut hints = Hints::none();
    if let Some(suggestion) = SUGGESTIONS.get(name) {
        hints.push(format!(
            "`{}` is often confused for `{}`. Did you mean to install `{}` instead?",
            name.cyan(),
            suggestion.cyan(),
            suggestion.cyan(),
        ));
    } else if !chain.is_empty() {
        hints.push(format_chain(name, version, chain));
    }
    hints.extend(cause_hints);
    hints.into_owned()
}

/// Format a [`DerivationChain`] as a human-readable error message.
fn format_chain(name: &PackageName, version: Option<&Version>, chain: &DerivationChain) -> String {
    /// Format a step in the [`DerivationChain`] as a human-readable error message.
    fn format_step(step: &DerivationStep, range: Option<Ranges<Version>>) -> String {
        if let Some(range) =
            range.filter(|range| *range != Ranges::empty() && *range != Ranges::full())
        {
            if let Some(extra) = &step.extra {
                if let Some(version) = step.version.as_ref() {
                    // Ex) `flask[dotenv]>=1.0.0` (v1.2.3)
                    format!(
                        "`{}{}` ({})",
                        format!("{}[{}]", step.name, extra).cyan(),
                        range.cyan(),
                        format!("v{version}").cyan(),
                    )
                } else {
                    // Ex) `flask[dotenv]>=1.0.0`
                    format!(
                        "`{}{}`",
                        format!("{}[{}]", step.name, extra).cyan(),
                        range.cyan(),
                    )
                }
            } else if let Some(group) = &step.group {
                if let Some(version) = step.version.as_ref() {
                    // Ex) `flask:dev>=1.0.0` (v1.2.3)
                    format!(
                        "`{}{}` ({})",
                        format!("{}:{}", step.name, group).cyan(),
                        range.cyan(),
                        format!("v{version}").cyan(),
                    )
                } else {
                    // Ex) `flask:dev>=1.0.0`
                    format!(
                        "`{}{}`",
                        format!("{}:{}", step.name, group).cyan(),
                        range.cyan(),
                    )
                }
            } else {
                if let Some(version) = step.version.as_ref() {
                    // Ex) `flask>=1.0.0` (v1.2.3)
                    format!(
                        "`{}{}` ({})",
                        step.name.cyan(),
                        range.cyan(),
                        format!("v{version}").cyan(),
                    )
                } else {
                    // Ex) `flask>=1.0.0`
                    format!("`{}{}`", step.name.cyan(), range.cyan())
                }
            }
        } else {
            if let Some(extra) = &step.extra {
                if let Some(version) = step.version.as_ref() {
                    // Ex) `flask[dotenv]` (v1.2.3)
                    format!(
                        "`{}` ({})",
                        format!("{}[{}]", step.name, extra).cyan(),
                        format!("v{version}").cyan(),
                    )
                } else {
                    // Ex) `flask[dotenv]`
                    format!("`{}`", format!("{}[{}]", step.name, extra).cyan())
                }
            } else if let Some(group) = &step.group {
                if let Some(version) = step.version.as_ref() {
                    // Ex) `flask:dev` (v1.2.3)
                    format!(
                        "`{}` ({})",
                        format!("{}:{}", step.name, group).cyan(),
                        format!("v{version}").cyan(),
                    )
                } else {
                    // Ex) `flask:dev`
                    format!("`{}`", format!("{}:{}", step.name, group).cyan())
                }
            } else {
                if let Some(version) = step.version.as_ref() {
                    // Ex) `flask` (v1.2.3)
                    format!("`{}` ({})", step.name.cyan(), format!("v{version}").cyan())
                } else {
                    // Ex) `flask`
                    format!("`{}`", step.name.cyan())
                }
            }
        }
    }

    let mut message = if let Some(version) = version {
        format!(
            "`{}` ({}) was included because",
            name.cyan(),
            format!("v{version}").cyan()
        )
    } else {
        format!("`{}` was included because", name.cyan())
    };
    let mut range: Option<Ranges<Version>> = None;
    message.reserve(chain.iter().len().saturating_mul(64));
    for (index, step) in chain.iter().enumerate() {
        if index > 0 {
            let _ = write!(message, " {} which depends on", format_step(step, range));
        } else {
            let _ = write!(message, " {} depends on", format_step(step, range));
        }
        range = Some(strip_local_version_sentinels(&step.range));
    }
    if let Some(range) = range.filter(|range| *range != Ranges::empty() && *range != Ranges::full())
    {
        let _ = write!(message, " `{}{}`", name.cyan(), range.cyan());
    } else {
        let _ = write!(message, " `{}`", name.cyan());
    }
    message
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use uv_distribution_types::{DerivationChain, DerivationStep};
    use uv_normalize::{ExtraName, GroupName, PackageName};
    use uv_pep440::Version;
    use version_ranges::Ranges;

    use super::format_chain;

    #[test]
    fn formats_derivation_chain_with_colors_and_ranges() {
        let version = |value| Version::from_str(value).expect("version should be valid");
        let chain: DerivationChain = [
            DerivationStep::new(
                PackageName::from_str("root").expect("package name should be valid"),
                Some(ExtraName::from_str("docs").expect("extra name should be valid")),
                None,
                Some(version("0.1.0")),
                Ranges::from_range_bounds(version("1.2.0")..version("2.0.0")),
            ),
            DerivationStep::new(
                PackageName::from_str("child").expect("package name should be valid"),
                None,
                Some(GroupName::from_str("dev").expect("group name should be valid")),
                None,
                Ranges::full(),
            ),
            DerivationStep::new(
                PackageName::from_str("leaf").expect("package name should be valid"),
                None,
                None,
                Some(version("1.3.0")),
                Ranges::singleton(version("4.5.0")),
            ),
        ]
        .into_iter()
        .collect();

        assert_eq!(
            format_chain(
                &PackageName::from_str("failed-package").expect("package name should be valid"),
                Some(&version("4.5.0")),
                &chain,
            ),
            concat!(
                "`\x1b[36mfailed-package\x1b[39m` (\x1b[36mv4.5.0\x1b[39m) was included because ",
                "`\x1b[36mroot[docs]\x1b[39m` (\x1b[36mv0.1.0\x1b[39m) depends on ",
                "`\x1b[36mchild:dev\x1b[39m\x1b[36m>=1.2.0, <2.0.0\x1b[39m` which depends on ",
                "`\x1b[36mleaf\x1b[39m` (\x1b[36mv1.3.0\x1b[39m) which depends on ",
                "`\x1b[36mfailed-package\x1b[39m\x1b[36m==4.5.0\x1b[39m`",
            )
        );
    }
}
