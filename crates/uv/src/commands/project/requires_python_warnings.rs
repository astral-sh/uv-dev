use uv_workspace::RequiresPythonSources;

use super::format_tilde_requires_python_warning;

fn sources(entries: &[(&str, Option<&str>, &str)]) -> anyhow::Result<RequiresPythonSources> {
    entries
        .iter()
        .map(|(package, group, specifiers)| {
            Ok((
                (package.parse()?, group.map(str::parse).transpose()?),
                specifiers.parse()?,
            ))
        })
        .collect()
}

#[test]
fn preserves_single_source_warning() -> anyhow::Result<()> {
    for (group, source) in [(None, "warehouse"), (Some("test"), "warehouse:test")] {
        let requires_python = sources(&[("warehouse", group, "~=3.13")])?;
        assert_eq!(
            format_tilde_requires_python_warning(&requires_python),
            Some(format!(
                "The `requires-python` specifier (`~=3.13`) in `{source}` uses the tilde \
                specifier (`~=`) without a patch version. This will be interpreted as \
                `>=3.13, <4`. Did you mean `~=3.13.0` to constrain the version as \
                `>=3.13.0, <3.14`? We recommend only using the tilde specifier with a \
                patch version to avoid ambiguity."
            ))
        );
    }
    Ok(())
}

#[test]
fn consolidates_sources_in_stable_order() -> anyhow::Result<()> {
    let requires_python = sources(&[
        ("widget", None, "~=3.12"),
        ("warehouse", Some("test"), "~=3.13"),
        ("aardvark", None, "~=3.10"),
        ("warehouse", None, "~=3.13"),
    ])?;
    assert_eq!(
        format_tilde_requires_python_warning(&requires_python).as_deref(),
        Some(concat!(
            "The following `requires-python` specifiers use the tilde specifier (`~=`) ",
            "without a patch version:\n",
            "- `aardvark`: `~=3.10` is interpreted as `>=3.10, <4`; use `~=3.10.0` ",
            "to constrain the version as `>=3.10.0, <3.11`\n",
            "- `warehouse`: `~=3.13` is interpreted as `>=3.13, <4`; use `~=3.13.0` ",
            "to constrain the version as `>=3.13.0, <3.14`\n",
            "- `warehouse:test`: `~=3.13` is interpreted as `>=3.13, <4`; use `~=3.13.0` ",
            "to constrain the version as `>=3.13.0, <3.14`\n",
            "- `widget`: `~=3.12` is interpreted as `>=3.12, <4`; use `~=3.12.0` ",
            "to constrain the version as `>=3.12.0, <3.13`\n",
            "We recommend only using the tilde specifier with a patch version to avoid ambiguity."
        ))
    );
    Ok(())
}

#[test]
fn preserves_warning_eligibility() -> anyhow::Result<()> {
    assert_eq!(
        format_tilde_requires_python_warning(&RequiresPythonSources::new()),
        None
    );
    for specifiers in [
        ">=3.12",
        "==3.12",
        "~=3.12.0",
        "~=3.12.0.1",
        "~=3.12, <3.13",
        "~=3.12rc1",
        "~=3.12.dev1",
        "~=3.12.post1",
    ] {
        let requires_python = sources(&[("warehouse", None, specifiers)])?;
        assert_eq!(
            format_tilde_requires_python_warning(&requires_python),
            None,
            "{specifiers}"
        );
    }
    Ok(())
}

#[test]
fn excluded_sources_do_not_make_a_plural_warning() -> anyhow::Result<()> {
    let single = sources(&[("warehouse", Some("test"), "~=3.13")])?;
    let mixed = sources(&[
        ("aardvark", None, "~=3.12, <3.13"),
        ("warehouse", None, "~=3.13.0"),
        ("warehouse", Some("test"), "~=3.13"),
        ("widget", None, ">=3.12"),
    ])?;
    assert_eq!(
        format_tilde_requires_python_warning(&mixed),
        format_tilde_requires_python_warning(&single)
    );
    Ok(())
}
