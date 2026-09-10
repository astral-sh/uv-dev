#![cfg(feature = "non-pep508-extensions")]

use std::error::Error;
use std::fmt::{self, Write};
use std::str::FromStr;

use uv_normalize::ExtraName;
use uv_pep508::{MarkerTree, UnnamedRequirement, VerbatimUrl};

const URL: &str = "https://example.invalid/archive.whl?raw=%23#fragment";

fn requirement(
    url: &str,
    extras: &[&str],
    marker: MarkerTree,
) -> Result<UnnamedRequirement, Box<dyn Error>> {
    Ok(UnnamedRequirement {
        url: VerbatimUrl::from_str(url)?,
        extras: extras
            .iter()
            .map(|name| ExtraName::from_str(name))
            .collect::<Result<Vec<_>, _>>()?
            .into_boxed_slice(),
        marker,
        origin: None,
    })
}

#[test]
fn extras_preserve_stored_order_and_duplicates() -> Result<(), Box<dyn Error>> {
    let cases: &[(&[&str], &str)] = &[
        (&[], ""),
        (&["dev"], "[dev]"),
        (&["z", "a", "z"], "[z,a,z]"),
        (&["B", "a_b", "B", "a.b"], "[b,a-b,b,a-b]"),
        (
            &["a-long-extra-name-that-does-not-fit-inline", "dev"],
            "[a-long-extra-name-that-does-not-fit-inline,dev]",
        ),
    ];
    for (extras, suffix) in cases {
        let requirement = requirement(URL, extras, MarkerTree::TRUE)?;
        let expected = format!("{URL}{suffix}");
        assert_eq!(requirement.to_string(), expected);
        assert_eq!(requirement.to_string(), expected);
        assert_eq!(requirement.url.raw().as_str(), URL);
        assert_eq!(requirement.url.given(), Some(URL));
        assert_eq!(requirement.marker, MarkerTree::TRUE);
    }
    Ok(())
}

#[test]
fn url_and_marker_rendering_are_unchanged() -> Result<(), Box<dyn Error>> {
    let url = "https://user:fake-token@example.invalid/archive.whl?raw=%23#fragment";
    let cases = [
        (MarkerTree::TRUE, ""),
        (MarkerTree::FALSE, " ; python_version < '0'"),
        (
            MarkerTree::from_str("python_full_version >= '3.12'")?,
            " ; python_full_version >= '3.12'",
        ),
    ];
    for (marker, suffix) in cases {
        let requirement = requirement(url, &["z", "a", "z"], marker)?;
        assert_eq!(
            requirement.to_string(),
            format!(
                "https://user:****@example.invalid/archive.whl?raw=%23#fragment[z,a,z]{suffix}"
            )
        );
        assert_eq!(requirement.url.raw().as_str(), url);
        assert_eq!(requirement.url.given(), Some(url));
        assert_eq!(requirement.marker, marker);
    }
    Ok(())
}

#[derive(Default)]
struct RejectExtra {
    rejected: bool,
    wrote_after_error: bool,
}

impl Write for RejectExtra {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        self.wrote_after_error |= self.rejected;
        if value.contains("failure-extra") {
            self.rejected = true;
            Err(fmt::Error)
        } else {
            Ok(())
        }
    }
}

#[test]
fn extras_propagate_formatting_errors() -> Result<(), Box<dyn Error>> {
    let requirement = requirement(URL, &["first", "failure-extra"], MarkerTree::FALSE)?;
    let mut writer = RejectExtra::default();
    assert_eq!(write!(&mut writer, "{requirement}"), Err(fmt::Error));
    assert!(writer.rejected);
    assert!(!writer.wrote_after_error);
    Ok(())
}
