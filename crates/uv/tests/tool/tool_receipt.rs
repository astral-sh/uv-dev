use anyhow::Result;
use uv_distribution_types::IndexLocations;
use uv_settings::{ResolverInstallerOptions, ToolOptions, ToolOptionsWire};

/// Receipt decoding retains named defaults while removing overridden unnamed defaults.
#[test]
fn receipt_retains_named_defaults() -> Result<()> {
    let wire: ToolOptionsWire = toml::from_str(
        r#"
        index = [
            { url = "https://default.example.com/simple", default = true },
            { name = "source", url = "https://source.example.com/simple", default = true, explicit = true },
            { url = "https://old.example.com/simple", default = true },
        ]
        "#,
    )?;
    let options = ResolverInstallerOptions::from(ToolOptions::from(wire));
    let locations = IndexLocations::from(options.indexes);
    assert_eq!(
        locations
            .default_index()
            .map(|index| index.raw_url().as_str()),
        Some("https://default.example.com/simple")
    );
    assert!(locations.known_indexes().any(|index| {
        index.name.as_deref() == Some("source")
            && index.raw_url().as_str() == "https://source.example.com/simple"
            && index.default
            && index.explicit
    }));
    assert!(
        !locations
            .known_indexes()
            .any(|index| index.raw_url().as_str() == "https://old.example.com/simple")
    );
    Ok(())
}

/// A shadowed named default does not suppress the effective unnamed default during receipt decoding.
#[test]
fn shadowed_named_default_does_not_hide_the_active_default() -> Result<()> {
    let wire: ToolOptionsWire = toml::from_str(
        r#"
        index = [
            { name = "named", url = "https://preferred.example.com/simple" },
            { name = "named", url = "https://shadowed.example.com/simple", default = true },
            { url = "https://active.example.com/simple", default = true },
            { url = "https://old.example.com/simple", default = true },
        ]
        "#,
    )?;
    let options = ResolverInstallerOptions::from(ToolOptions::from(wire));
    let locations = IndexLocations::from(options.indexes);
    assert_eq!(
        locations
            .default_index()
            .map(|index| index.raw_url().as_str()),
        Some("https://active.example.com/simple")
    );
    assert!(
        !locations
            .known_indexes()
            .any(|index| index.raw_url().as_str() == "https://old.example.com/simple")
    );
    Ok(())
}
