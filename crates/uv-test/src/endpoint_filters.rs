//! Labels for local endpoints in human-readable command output.

use std::net::SocketAddr;

pub(super) fn add_endpoint_role(
    filters: &mut Vec<(String, String)>,
    address: &SocketAddr,
    label: &str,
) {
    let authority = regex::escape(&address.to_string());
    // Start at a displayed token boundary, keeping quoted URLs and keyring's user@URL form intact.
    // Consume the rest of the URL so authority-looking text in its path is not labelled separately.
    let pattern = format!(
        r#"(?m)(?P<prefix>(?:^|\s)["'`(\[]*(?:[^\s/]+@)?(?:https?://(?:[^\s/]+@)?)?){authority}(?P<suffix>(?:[/?#][^\s"'`<>]*)?)(?P<end>[\s"'`<>)\],;]|$)"#
    );
    let replacement = format!("${{prefix}}{}${{suffix}}${{end}}", label.replace('$', "$$"));
    let filter = (pattern, replacement);
    // Both boundaries consume separators. A second pass covers adjacent tokens sharing one.
    // Insert before the generic localhost filter, which would otherwise erase endpoint identity.
    filters.splice(0..0, [filter.clone(), filter]);
}

#[cfg(test)]
mod tests {
    use indoc::indoc;

    use super::add_endpoint_role;
    use crate::apply_filters;

    #[test]
    fn endpoint_roles_precede_localhost_fallback() {
        let mut filters = vec![(r"127\.0\.0\.1:\d*".to_string(), "[LOCALHOST]".to_string())];
        add_endpoint_role(
            &mut filters,
            &"127.0.0.1:1234".parse().expect("valid endpoint"),
            "[INDEX]",
        );
        add_endpoint_role(
            &mut filters,
            &"127.0.0.1:5678".parse().expect("valid endpoint"),
            "[ARTIFACT_HOST]",
        );
        let output = indoc! {r#"
            error: index `http://alice:secret@127.0.0.1:1234/simple`
            metadata "http://127.0.0.1:5678/package.whl.metadata"
            Keyring request for alice@http://127.0.0.1:1234/simple
            Keyring request for alice@127.0.0.1:1234
            Keyring request for alice@example.com@http://127.0.0.1:1234/simple
            Keyring request for alice@example.com@127.0.0.1:1234
            fallback http://127.0.0.1:9999/other
        "#};
        insta::assert_snapshot!(apply_filters(output.to_string(), filters), @r#"
        error: index `http://alice:secret@[INDEX]/simple`
        metadata "http://[ARTIFACT_HOST]/package.whl.metadata"
        Keyring request for alice@http://[INDEX]/simple
        Keyring request for alice@[INDEX]
        Keyring request for alice@example.com@http://[INDEX]/simple
        Keyring request for alice@example.com@[INDEX]
        fallback http://[LOCALHOST]/other
        "#);
    }

    #[test]
    fn adjacent_aliases_and_authority_boundaries() {
        let mut filters = Vec::new();
        add_endpoint_role(
            &mut filters,
            &"127.0.0.1:1234".parse().expect("valid endpoint"),
            "[INDEX]",
        );
        let output = indoc! {"
            http://127.0.0.1:1234/ http://127.0.0.1:1234/ http://127.0.0.1:1234/ http://127.0.0.1:1234/
            127.0.0.1:1234 127.0.0.1:1234 127.0.0.1:1234
            http://127.0.0.1:12345/ http://127x0x0x1:1234/
            http://127.0.0.1:1234@other.example/path
            http://alice:127.0.0.1:1234@127.0.0.1:1234/path/127.0.0.1:1234
            http://other.example/path/http://127.0.0.1:1234/
            http://other.example/path/(http://127.0.0.1:1234/)
            http://other.example/path?next=http://127.0.0.1:1234/
        "};
        insta::assert_snapshot!(apply_filters(output.to_string(), filters), @"
        http://[INDEX]/ http://[INDEX]/ http://[INDEX]/ http://[INDEX]/
        [INDEX] [INDEX] [INDEX]
        http://127.0.0.1:12345/ http://127x0x0x1:1234/
        http://127.0.0.1:1234@other.example/path
        http://alice:127.0.0.1:1234@[INDEX]/path/127.0.0.1:1234
        http://other.example/path/http://127.0.0.1:1234/
        http://other.example/path/(http://127.0.0.1:1234/)
        http://other.example/path?next=http://127.0.0.1:1234/
        ");
    }

    #[test]
    fn ipv6_and_literal_label() {
        let mut filters = Vec::new();
        add_endpoint_role(
            &mut filters,
            &"[::1]:1234".parse().expect("valid endpoint"),
            "[INDEX$1]",
        );
        assert_eq!(
            apply_filters("http://[::1]:1234/simple".to_string(), filters),
            "http://[INDEX$1]/simple",
        );
    }
}
