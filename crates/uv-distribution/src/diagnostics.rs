use std::borrow::Cow;
use std::fmt::Write;
use std::sync::LazyLock;

use owo_colors::OwoColorize;
use rustc_hash::FxHashMap;
use version_ranges::Ranges;

use uv_distribution_types::{DerivationChain, DerivationStep};
use uv_errors::Hints;
use uv_normalize::PackageName;
use uv_pep440::{Version, strip_local_version_sentinels};

static SUGGESTIONS: LazyLock<FxHashMap<PackageName, PackageName>> = LazyLock::new(|| {
    let suggestions: Vec<(PackageName, PackageName)> =
        serde_json::from_str(include_str!("suggestions.json")).unwrap();
    suggestions.into_iter().collect()
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
        let name = if let Some(extra) = &step.extra {
            Cow::Owned(format!("{}[{extra}]", step.name))
        } else if let Some(group) = &step.group {
            Cow::Owned(format!("{}:{group}", step.name))
        } else {
            Cow::Borrowed(step.name.as_ref())
        };
        let range = range.filter(|range| *range != Ranges::empty() && *range != Ranges::full());
        match (range, &step.version) {
            (Some(range), Some(version)) => format!(
                "`{}{}` ({})",
                name.cyan(),
                range.cyan(),
                format!("v{version}").cyan(),
            ),
            (Some(range), None) => format!("`{}{}`", name.cyan(), range.cyan()),
            (None, Some(version)) => {
                format!("`{}` ({})", name.cyan(), format!("v{version}").cyan())
            }
            (None, None) => format!("`{}`", name.cyan()),
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
    for (i, step) in chain.iter().enumerate() {
        message.push(' ');
        message.push_str(&format_step(step, range));
        message.push_str(if i > 0 {
            " which depends on"
        } else {
            " depends on"
        });
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
