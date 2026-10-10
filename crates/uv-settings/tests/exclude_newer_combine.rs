use uv_resolver::ExcludeNewer;
use uv_settings::Combine;

fn parse(input: &str) -> ExcludeNewer {
    toml::from_str(input).unwrap()
}

#[test]
fn exclude_newer_combine_preserves_precedence() {
    let current = parse(
        r#"
global = "2024-01-03T00:00:00Z"

[package]
disabled = false
enabled = "2024-01-02T00:00:00Z"
relative = { timestamp = "2024-01-04T00:00:00Z", span = "P3D" }
"#,
    );
    let fallback = parse(
        r#"
global = "2024-01-01T00:00:00Z"

[package]
disabled = "2023-12-01T00:00:00Z"
enabled = false
fallback-disabled = false
fallback-enabled = "2023-11-01T00:00:00Z"
fallback-relative = { timestamp = "2024-01-02T00:00:00Z", span = "P2D" }
relative = { timestamp = "2024-01-03T00:00:00Z", span = "P1D" }
"#,
    );
    let expected = parse(
        r#"
global = "2024-01-03T00:00:00Z"

[package]
disabled = false
enabled = "2024-01-02T00:00:00Z"
fallback-disabled = false
fallback-enabled = "2023-11-01T00:00:00Z"
fallback-relative = { timestamp = "2024-01-02T00:00:00Z", span = "P2D" }
relative = { timestamp = "2024-01-04T00:00:00Z", span = "P3D" }
"#,
    );

    // Compare relative values directly, without resolving their timestamps against the clock.
    assert_eq!(current.combine(fallback), expected);

    let current = parse("global = \"2024-01-03T00:00:00Z\"");
    let fallback = parse("global = \"2024-01-01T00:00:00Z\"");
    assert_eq!(current.clone().combine(fallback.clone()), current);
    assert_eq!(ExcludeNewer::default().combine(fallback.clone()), fallback);
    assert_eq!(current.clone().combine(ExcludeNewer::default()), current);
}

#[test]
fn exclude_newer_combine_preserves_empty_map_cases() {
    let populated = parse(
        r#"
[package]
disabled = false
enabled = "2024-01-02T00:00:00Z"
relative = { timestamp = "2024-01-04T00:00:00Z", span = "P3D" }
"#,
    );

    assert_eq!(
        ExcludeNewer::default().combine(populated.clone()),
        populated
    );
    assert_eq!(
        populated.clone().combine(ExcludeNewer::default()),
        populated
    );
    assert_eq!(
        ExcludeNewer::default().combine(ExcludeNewer::default()),
        ExcludeNewer::default()
    );
}

#[test]
fn exclude_newer_combine_preserves_serialized_order() {
    let current = ExcludeNewer::from_args(
        Some("2024-01-03T00:00:00Z".parse().unwrap()),
        vec![
            "zeta=false".parse().unwrap(),
            "alpha=2024-01-02T00:00:00Z".parse().unwrap(),
        ],
    );
    let fallback = ExcludeNewer::from_args(
        Some("2024-01-01T00:00:00Z".parse().unwrap()),
        vec![
            "middle=2023-12-01T00:00:00Z".parse().unwrap(),
            "alpha=false".parse().unwrap(),
            "beta=false".parse().unwrap(),
        ],
    );
    let combined = current.combine(fallback);

    assert_eq!(
        toml::to_string(&combined).unwrap(),
        concat!(
            "global = \"2024-01-03T00:00:00Z\"\n",
            "\n[package]\n",
            "alpha = \"2024-01-02T00:00:00Z\"\n",
            "beta = false\n",
            "middle = \"2023-12-01T00:00:00Z\"\n",
            "zeta = false\n",
        )
    );
    assert_eq!(toml::to_string(&ExcludeNewer::default()).unwrap(), "");
    assert_eq!(
        toml::to_string(&parse("global = \"2024-01-03T00:00:00Z\"")).unwrap(),
        "global = \"2024-01-03T00:00:00Z\"\n"
    );
    assert_eq!(
        toml::to_string(&parse("[package]\nalpha = false")).unwrap(),
        "[package]\nalpha = false\n"
    );
}
