use std::ops::Range;

use url::Url;

use crate::{ambiguous_credential_range, is_generic_git_username, is_sensitive_query_parameter};

/// Find sensitive components of visibly spelled, authority-bearing URLs in source text.
///
/// The returned ranges are nonempty, sorted, non-overlapping UTF-8 byte ranges into `text`.
/// They use the same userinfo and sensitive query-parameter policy as [`crate::DisplaySafeUrl`],
/// but retain the original spelling instead of serializing a parsed URL. Callers can mask these
/// ranges without changing source locations or suggested edits.
///
/// This recognizes `scheme://` URLs delimited by whitespace or source quotes. Percent-encoded
/// query names are decoded when checking the sensitive-parameter policy. It does not decode the
/// enclosing source language, expand variables, or identify arbitrary secrets. In particular,
/// source escapes that obscure URL delimiters require a producer-specific source mapping.
pub fn url_redaction_ranges(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    for (colon, _) in text.match_indices("://") {
        let scheme_start = text[..colon]
            .char_indices()
            .rev()
            .take_while(|(_, character)| {
                character.is_ascii_alphanumeric() || matches!(*character, '+' | '-' | '.')
            })
            .last()
            .map_or(colon, |(index, _)| index);
        if !text
            .as_bytes()
            .get(scheme_start)
            .is_some_and(u8::is_ascii_alphabetic)
        {
            continue;
        }

        let scheme = &text[scheme_start..colon];
        let authority_start = colon + 3;
        let end = url_end(text, scheme_start, authority_start);
        let raw = &text[scheme_start..end];
        if let Ok(url) = Url::parse(raw)
            && let Some(range) = ambiguous_credential_range(raw, &url)
        {
            ranges.push(scheme_start + range.start..scheme_start + range.end);
        }
        let authority_end = text[authority_start..end]
            .find(['/', '?', '#'])
            .map_or(end, |index| authority_start + index);
        let authority = &text[authority_start..authority_end];
        if let Some(at) = authority.rfind('@') {
            let userinfo = &authority[..at];
            if let Some(colon) = userinfo.find(':') {
                let start = authority_start + colon + 1;
                let end = authority_start + at;
                if start < end {
                    ranges.push(start..end);
                }
            } else if !userinfo.is_empty() && !is_generic_git_username(scheme, userinfo, false) {
                ranges.push(authority_start..authority_start + at);
            }
        }

        let remainder = &text[authority_end..end];
        let before_fragment = remainder.split('#').next().unwrap_or_default();
        let Some(question) = before_fragment.find('?') else {
            continue;
        };
        let query_start = authority_end + question + 1;
        let query_end = authority_end + before_fragment.len();
        let mut start = query_start;
        while start < query_end {
            let end = text[start..query_end]
                .find('&')
                .map_or(query_end, |index| start + index);
            let pair = &text[start..end];
            if let Some((key, value)) = pair.split_once('=')
                && !value.is_empty()
                && url::form_urlencoded::parse(key.as_bytes())
                    .next()
                    .is_some_and(|(key, _)| is_sensitive_query_parameter(&key))
            {
                ranges.push(start + key.len() + 1..end);
            }
            if end == query_end {
                break;
            }
            start = end + 1;
        }
    }

    // A URL can contain another URL in its path or query. Merge overlapping discoveries so each
    // original byte is masked at most once.
    ranges.sort_unstable_by_key(|range| (range.start, range.end));
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        if let Some(previous) = merged.last_mut()
            && range.start <= previous.end
        {
            previous.end = previous.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    merged
}

fn url_end(text: &str, scheme_start: usize, authority_start: usize) -> usize {
    let quote = text[..scheme_start]
        .chars()
        .next_back()
        .filter(|character| matches!(*character, '\'' | '"'));
    let mut escaped = false;
    for (index, character) in text[authority_start..].char_indices() {
        if character.is_whitespace() || matches!(character, '<' | '>' | '`') {
            return authority_start + index;
        }
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if quote.map_or_else(
            || matches!(character, '\'' | '"'),
            |quote| character == quote,
        ) {
            return authority_start + index;
        }
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::url_redaction_ranges;

    fn components(text: &str) -> Vec<&str> {
        url_redaction_ranges(text)
            .into_iter()
            .map(|range| &text[range])
            .collect()
    }

    #[test]
    fn source_url_userinfo() {
        assert_eq!(
            components(
                r#"index = "https://user:secret@example.com/simple"; dependency = 'demo @ https://token@example.com/demo.whl'"#,
            ),
            ["secret", "token"],
        );
        assert_eq!(
            components(
                "ssh://git@example.com/repo git+ssh://git@example.com/repo git+https://git@example.com/repo https://git@example.com/repo ssh://git:secret@example.com/repo"
            ),
            ["git", "secret"],
        );
    }

    #[test]
    fn source_url_queries_use_display_policy() {
        assert_eq!(
            components(
                "https://example.com/dist.whl?X-Amz%2DSignature=signature&x-amz-credential=credential&X-Amz-Security-Token=token&sig=azure&token=unchanged#sig=fragment"
            ),
            ["signature", "credential", "token", "azure"],
        );
        assert!(components("https://example.com/?sig=&safe=value").is_empty());
    }

    #[test]
    fn source_url_ambiguous_credentials_use_display_policy() {
        assert_eq!(
            components("https://user/name:password@domain/a/b/c"),
            ["//user/name:password"],
        );
        assert!(components("file://C:/Users/ferris/project@home/workspace").is_empty());
        assert!(components("git+file://C:/Users/ferris/repo.git@v1.0").is_empty());
        assert!(
            components("git+https://proxy.com/https://github.com/user/repo.git@branch").is_empty(),
        );
    }

    #[test]
    fn source_url_requirements_physical_lines() {
        assert_eq!(
            components(
                "invalid = !\r--index-url https://user:first@example.com/simple\r\n-r https://second@example.com/requirements.txt\n"
            ),
            ["first", "second"],
        );
    }

    #[test]
    fn source_url_ranges_use_original_utf8() {
        let text = r#"url = "https://user:pāss\u0077ord🦀@example.com/?sig=秘密""#;
        let ranges = url_redaction_ranges(text);
        assert!(ranges.iter().all(|range| text.get(range.clone()).is_some()));
        assert_eq!(components(text), [r"pāss\u0077ord🦀", "秘密"]);
        assert_eq!(
            components(r#"url = "https://user:pass'word@example.com/""#),
            ["pass'word"],
        );
    }

    #[test]
    fn source_url_nested_ranges_are_disjoint() {
        let text = "https://example.com/?sig=https://user:secret@example.com/a&safe=value";
        assert_eq!(components(text), ["https://user:secret@example.com/a"],);
        assert!(components("not a URL: user:secret@example.com").is_empty());
        assert!(components(r"https\u003a//user:secret@example.com").is_empty());
    }
}
