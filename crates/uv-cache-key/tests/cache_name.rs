use uv_cache_key::cache_name;

#[test]
fn cache_name_limits_utf8_input_bytes() {
    for (name, limit, expected) in [
        ("ABéCD", 1, Some("a")),
        ("ABéCD", 2, Some("ab")),
        ("ABéCD", 3, Some("ab")),
        ("ABéCD", 4, Some("ab")),
        ("ABéCD", 5, Some("ab-c")),
        ("ABéCD", 6, Some("ab-cd")),
        ("A🦀Z", 1, Some("a")),
        ("A🦀Z", 2, Some("a")),
        ("A🦀Z", 3, Some("a")),
        ("A🦀Z", 4, Some("a")),
        ("A🦀Z", 5, Some("a")),
        ("A🦀Z", 6, Some("a-z")),
        ("é", 1, None),
        ("é", 2, None),
        ("🦀", 4, None),
    ] {
        assert_eq!(cache_name(name, Some(limit)).as_deref(), expected);
    }
}

#[test]
fn cache_name_preserves_environment_name_budget() {
    // Script and centralized project environments use a 100-byte name budget.
    let name = "G".repeat(240);
    let expected = "g".repeat(100);
    assert_eq!(
        cache_name(&name, Some(100)).as_deref(),
        Some(expected.as_str())
    );

    // Truncation inside a Unicode scalar still yields an ASCII name without a trailing dash.
    let prefix = "Z".repeat(99);
    let name = format!("{prefix}🦀tail");
    let expected = "z".repeat(99);
    assert_eq!(
        cache_name(&name, Some(100)).as_deref(),
        Some(expected.as_str())
    );
}
