use std::iter::once;
use std::ops::Range;

use crate::{
    SENSITIVE_QUERY_PARAMETERS, has_credential_like_pattern, is_file_transport,
    is_generic_git_username, is_sensitive_query_parameter,
};

/// Find sensitive components of visibly spelled, authority-bearing URLs in source text.
///
/// The returned ranges are nonempty, sorted, non-overlapping UTF-8 byte ranges into `text`.
/// They use the same userinfo and sensitive query-parameter policy as [`crate::DisplaySafeUrl`],
/// but retain the original spelling instead of serializing a parsed URL. Callers can mask these
/// ranges without changing source locations or suggested edits.
///
/// This recognizes `scheme://` URLs delimited by whitespace or source quotes. Percent-encoded
/// query names are decoded when checking the sensitive-parameter policy. Ambiguous credentials
/// are checked in the original path and fragment, including segments URL normalization would
/// remove. It does not decode the enclosing source language, expand variables, or identify
/// arbitrary secrets. In particular, source escapes that obscure URL delimiters require a
/// producer-specific source mapping.
pub fn url_redaction_ranges(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut cursor = 0;
    let mut search = 0;
    let mut quote = None;
    while let Some(colon) = text[search..].find("://").map(|index| search + index) {
        let Some(start) = scheme_start(text, colon) else {
            search = colon + 3;
            continue;
        };
        quote = source_quote(text, cursor..start, quote);
        let end = url_end(text, colon + 3, quote);
        redact_url_token(&text[start..end], start, quote, &mut ranges);
        cursor = end;
        search = end;
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

fn scheme_start(text: &str, colon: usize) -> Option<usize> {
    let start = text[..colon]
        .char_indices()
        .rev()
        .take_while(|(_, character)| {
            character.is_ascii_alphanumeric() || matches!(*character, '+' | '-' | '.')
        })
        .last()
        .map_or(colon, |(index, _)| index);
    text.as_bytes()
        .get(start)
        .is_some_and(u8::is_ascii_alphabetic)
        .then_some(start)
}

/// The enclosing source quote, which can precede a named requirement or other URL prefix.
#[derive(Clone, Copy)]
struct SourceQuote {
    byte: u8,
    width: usize,
}

impl SourceQuote {
    fn at(text: &str) -> Option<Self> {
        let byte = *text.as_bytes().first()?;
        if !matches!(byte, b'\'' | b'"') {
            return None;
        }
        let width = if text.as_bytes().starts_with(&[byte; 3]) {
            3
        } else {
            1
        };
        Some(Self { byte, width })
    }

    fn matches(self, text: &str) -> bool {
        text.as_bytes()
            .get(..self.width)
            .is_some_and(|bytes| bytes.iter().all(|byte| *byte == self.byte))
    }

    fn uses_escapes(self) -> bool {
        self.byte == b'"'
    }
}

fn source_quote(
    text: &str,
    range: Range<usize>,
    mut quote: Option<SourceQuote>,
) -> Option<SourceQuote> {
    let mut cursor = range.start;
    while cursor < range.end {
        let remaining = &text[cursor..range.end];
        let Some(character) = remaining.chars().next() else {
            break;
        };
        if let Some(current) = quote {
            if current.uses_escapes() && character == '\\' {
                cursor += 1;
                if let Some(next) = text[cursor..range.end].chars().next() {
                    if current.width == 1 && matches!(next, '\r' | '\n') {
                        quote = None;
                    }
                    cursor += next.len_utf8();
                }
                continue;
            }
            if current.matches(remaining) {
                cursor += current.width;
                quote = None;
                continue;
            }
            if current.width == 1 && matches!(character, '\r' | '\n') {
                quote = None;
            }
        } else if let Some(current) = SourceQuote::at(remaining)
            && (current.uses_escapes()
                || current.width == 3
                || text[..cursor]
                    .chars()
                    .next_back()
                    .is_none_or(|previous| !previous.is_alphanumeric() && previous != '_'))
        {
            quote = Some(current);
            cursor += current.width;
            continue;
        }
        cursor += character.len_utf8();
    }
    quote
}

fn url_end(text: &str, authority_start: usize, quote: Option<SourceQuote>) -> usize {
    let mut escaped = false;
    for (index, character) in text[authority_start..].char_indices() {
        if character.is_whitespace() || (quote.is_none() && matches!(character, '<' | '>' | '`')) {
            return authority_start + index;
        }
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && quote.is_some_and(SourceQuote::uses_escapes) {
            escaped = true;
            continue;
        }
        // An apostrophe is a legal URL sub-delimiter, including in bare requirements. Only a
        // matching source quote ends a quoted URL.
        if quote.map_or_else(
            || character == '"',
            |quote| quote.matches(&text[authority_start + index..]),
        ) {
            return authority_start + index;
        }
    }
    text.len()
}

fn is_special_network_scheme(scheme: &str) -> bool {
    ["http", "https", "ws", "wss", "ftp"]
        .iter()
        .any(|special| scheme.eq_ignore_ascii_case(special))
}

/// Inspect all nested URLs in one lexical token without reparsing their overlapping suffixes.
fn redact_url_token(
    text: &str,
    offset: usize,
    quote: Option<SourceQuote>,
    ranges: &mut Vec<Range<usize>>,
) {
    let mut path_boundaries = text
        .match_indices(['?', '#'])
        .map(|(index, _)| index)
        .peekable();
    let mut fragments = text.match_indices('#').map(|(index, _)| index).peekable();
    let mut queries = Vec::new();
    let last_at = text.rfind('@');
    let mut checked_path_end = None;
    let mut checked_fragment = false;
    let mut ambiguous = false;

    for (colon, _) in text.match_indices("://") {
        let Some(start) = scheme_start(text, colon) else {
            continue;
        };
        let scheme = &text[start..colon];
        let file_transport = is_file_transport(scheme);
        let special = is_special_network_scheme(scheme);
        // Basic source strings can spell a password with escapes such as `\u0077`. Their raw
        // backslashes remain part of the visible userinfo; mapping decoded path separators back
        // to these source bytes requires a producer-specific source mapping.
        let backslash_separator =
            file_transport || (special && !quote.is_some_and(SourceQuote::uses_escapes));
        let mut authority_start = colon + 3;
        if special {
            authority_start += text[authority_start..]
                .bytes()
                .take_while(|byte| *byte == b'/' || (backslash_separator && *byte == b'\\'))
                .count();
        }
        let authority_end = text[authority_start..]
            .find(|character| {
                matches!(character, '/' | '?' | '#') || (backslash_separator && character == '\\')
            })
            .map_or(text.len(), |index| authority_start + index);
        let authority = &text[authority_start..authority_end];
        let mut has_password = false;
        if let Some(at) = authority.rfind('@') {
            let userinfo = &authority[..at];
            let (username, password) = userinfo.split_once(':').unwrap_or((userinfo, ""));
            // URL parsing removes an empty password delimiter, so its username follows the
            // passwordless policy even though the original source spelling retains the colon.
            has_password = !password.is_empty();
            if has_password {
                let start = authority_start + username.len() + 1;
                let end = authority_start + at;
                ranges.push(offset + start..offset + end);
            } else if !username.is_empty() && !is_generic_git_username(scheme, username, false) {
                ranges.push(offset + authority_start..offset + authority_start + username.len());
            }
        }

        // Authority ends advance through a token, so these delimiter scans also advance only
        // forwards. URLs nested in one path share its end; their paths are successively smaller
        // suffixes, and only the first eligible path needs the ambiguity check.
        while path_boundaries
            .next_if(|index| *index < authority_end)
            .is_some()
        {}
        let path_end = path_boundaries.peek().copied().unwrap_or(text.len());
        while fragments.next_if(|index| *index < authority_end).is_some() {}
        let fragment = fragments.peek().copied().unwrap_or(text.len());

        if let Some(last_at) = last_at
            && colon < last_at
            && !file_transport
            && !has_password
            && !ambiguous
        {
            let path_start = if text.as_bytes().get(authority_end) == Some(&b'\\') {
                // A raw special-URL backslash can split an apparent password. Include the
                // apparent authority so `user:secret\more@host` is still masked, while an
                // ordinary `host\repo@revision` has no credential-like colon.
                authority_start
            } else {
                authority_end
            };
            let ambiguous_path = if checked_path_end == Some(path_end) {
                false
            } else {
                checked_path_end = Some(path_end);
                has_credential_like_pattern(&text[path_start..path_end])
            };
            let ambiguous_fragment = if checked_fragment {
                false
            } else {
                checked_fragment = true;
                has_credential_like_pattern(&text[fragment..])
            };
            if ambiguous_path || ambiguous_fragment {
                ranges.push(offset + colon + 1..offset + last_at);
                ambiguous = true;
            }
        }

        if text.as_bytes().get(path_end) == Some(&b'?') {
            queries.push(path_end + 1..fragment);
        }
    }

    // Nested queries share their next fragment boundary. Visit each `&` pair once, and also
    // inspect the first pair of each nested URL (which can begin inside an outer pair's value).
    for queries in queries.chunk_by(|left, right| left.end == right.end) {
        let first = &queries[0];
        let mut starts = queries.iter().map(|query| query.start).peekable();
        let mut pair_start = first.start;
        for pair_end in text[first.start..first.end]
            .match_indices('&')
            .map(|(index, _)| first.start + index)
            .chain(once(first.end))
        {
            if let Some(range) = sensitive_query_value(text, pair_start..pair_end) {
                ranges.push(offset + range.start..offset + range.end);
            }
            while let Some(start) = starts.next_if(|start| *start <= pair_end) {
                if start > pair_start
                    && let Some(range) = sensitive_query_value(text, start..pair_end)
                {
                    ranges.push(offset + range.start..offset + range.end);
                }
            }
            pair_start = pair_end + 1;
        }
    }
}

fn sensitive_query_value(text: &str, pair: Range<usize>) -> Option<Range<usize>> {
    // Percent-encoding can triple a policy key's byte length. Bounding only key recognition
    // avoids rescanning long overlapping values without limiting which URLs or values we visit.
    let max_key_length = SENSITIVE_QUERY_PARAMETERS
        .iter()
        .map(|key| key.len() * 3)
        .max()
        .unwrap_or_default();
    let equals = text.as_bytes()[pair.clone()]
        .iter()
        .take(max_key_length + 1)
        .position(|byte| *byte == b'=')?;
    let value_start = pair.start + equals + 1;
    if value_start == pair.end {
        return None;
    }
    let key = &text[pair.start..pair.start + equals];
    url::form_urlencoded::parse(key.as_bytes())
        .next()
        .is_some_and(|(key, _)| is_sensitive_query_parameter(&key))
        .then_some(value_start..pair.end)
}

#[cfg(test)]
mod tests {
    use crate::DisplaySafeUrl;

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
    fn source_url_empty_password_uses_username_policy() {
        let token = "https://token:@example.invalid/";
        assert_eq!(components(token), ["token"]);
        assert_eq!(
            DisplaySafeUrl::parse(token)
                .expect("a URL with an empty password is valid")
                .to_string(),
            "https://****@example.invalid/",
        );
        assert!(components("https://:@example.invalid/").is_empty());
        assert!(components("https://@example.invalid/").is_empty());
        assert_eq!(
            components("https://token@part:@example.invalid/"),
            ["token@part"],
        );
        assert_eq!(components("https://:secret@example.invalid/"), ["secret"]);
        assert_eq!(
            components("https://user:secret@example.invalid/"),
            ["secret"],
        );

        let git = "ssh://git:@example.invalid/repo";
        assert!(components(git).is_empty());
        assert_eq!(
            DisplaySafeUrl::parse(git)
                .expect("a Git URL with an empty password is valid")
                .to_string(),
            "ssh://git@example.invalid/repo",
        );
        assert!(components("git+ssh://git:@example.invalid/repo").is_empty());
        assert!(components("git+https://git:@example.invalid/repo").is_empty());
        assert_eq!(components("https://git:@example.invalid/repo"), ["git"]);
        assert_eq!(
            components("ssh://git:secret@example.invalid/repo"),
            ["secret"],
        );
        assert_eq!(components("ssh://git::@example.invalid/repo"), [":"]);
    }

    #[test]
    fn source_url_special_scheme_authorities() {
        assert_eq!(
            components(
                r"http:///user:first@example.invalid HTTPS:////second@example.invalid ws:///user:third@example.invalid wss://\\user:fourth@example.invalid ftp:////user:fifth@example.invalid",
            ),
            ["first", "second", "third", "fourth", "fifth"],
        );
        assert_eq!(
            components(r#"url = "https:///user:secret@example.invalid/simple""#),
            ["secret"],
        );
    }

    #[test]
    fn source_url_named_requirements_keep_enclosing_quotes() {
        assert_eq!(
            components(
                r#"dependencies = ["demo @ https://user:pa'ss@example.invalid/demo.whl ; python_version >= '3.12'"]"#,
            ),
            ["pa'ss"],
        );
        assert_eq!(
            components(r#"dependencies = ['demo @ https://user:pa"ss@example.invalid/demo.whl']"#,),
            ["pa\"ss"],
        );
        assert_eq!(
            components(r#"dependencies = ["demo @ https://user:pa\"ss@example.invalid/demo.whl"]"#,),
            [r#"pa\"ss"#],
        );
        assert_eq!(
            components(
                r#"dependencies = ["""demo @ https://user:pa"ss@example.invalid/demo.whl"""]"#,
            ),
            ["pa\"ss"],
        );
        assert_eq!(
            components(
                r"dependencies = ['''demo @ https://user:pa'ss@example.invalid/demo.whl''']",
            ),
            ["pa'ss"],
        );
        assert_eq!(
            components("demo @ https://user:pa'ss@example.invalid/demo.whl?sig=si'gned"),
            ["pa'ss", "si'gned"],
        );
        assert_eq!(
            components("# don't echo https://user:pa'ss@example.invalid/demo.whl"),
            ["pa'ss"],
        );
        assert!(
            components(r#"url = "https://example.invalid/safe", note = "user:secret@host""#)
                .is_empty(),
        );
    }

    #[test]
    fn source_url_quoted_markup_delimiters() {
        assert_eq!(
            components(r#"url = "https://user:pa<ss>`word@example.invalid/simple?sig=si<gn>`ed""#,),
            ["pa<ss>`word", "si<gn>`ed"],
        );
        assert_eq!(
            components("<https://user:secret@example.invalid/?sig=signed>"),
            ["secret", "signed"],
        );
    }

    #[test]
    fn source_url_backslash_authority_boundary() {
        assert_eq!(
            components(r"https://user:secret\more@host"),
            [r"//user:secret\more"],
        );
        assert_eq!(
            components(r"url = 'https://user:secret\more@host'"),
            [r"//user:secret\more"],
        );
        // A port and a numeric password cannot be distinguished in this raw spelling.
        assert_eq!(
            components(r"https://example.invalid:443\repo@revision"),
            [r"//example.invalid:443\repo"],
        );
        assert!(components(r"https://example.invalid\repo@revision").is_empty());
        assert!(components(r"url = 'https://example.invalid\repo@revision'").is_empty());
        assert!(components(r"file://C:\Users\ferris\project@home").is_empty());
        assert!(components(r"git+file://C:\Users\ferris\repo.git@v1.0").is_empty());
    }

    #[test]
    fn source_url_queries_use_display_policy() {
        assert_eq!(
            components(
                "https://example.com/dist.whl?X-Amz%2DSignature=signature&x-amz-credential=credential&X-Amz-Security-Token=token&sig=azure&token=unchanged#sig=fragment"
            ),
            ["signature", "credential", "token", "azure"],
        );
        assert_eq!(
            components(
                "https://example.com/?%58%2D%41%6D%7A%2D%53%65%63%75%72%69%74%79%2D%54%6F%6B%65%6E=token",
            ),
            ["token"],
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
    fn source_url_ambiguity_uses_original_path() {
        let text = "https://example.invalid/name:password@host/../safe";
        assert_eq!(components(text), ["//example.invalid/name:password"]);
        assert_eq!(
            DisplaySafeUrl::parse(text)
                .expect("the URL normalizes away the credential-like path segment")
                .to_string(),
            "https://example.invalid/safe",
        );
        assert!(components("git+file://C:/name:password@host/../safe").is_empty());
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

    #[test]
    fn source_url_nested_query_boundaries() {
        assert_eq!(
            components("https://outer.invalid/?next=https://user:secret@inner.invalid/pkg"),
            ["secret"],
        );
        assert_eq!(
            components("https://outer.invalid/#next=https://user:secret@inner.invalid/pkg"),
            ["//outer.invalid/#next=https://user:secret"],
        );
        assert_eq!(
            components(
                "https://outer.invalid/?next=https://inner.invalid/?next=https://third.invalid/?sig=first&safe=yes#next=https://fourth.invalid/?sig=second",
            ),
            ["first", "second"],
        );
        assert!(
            components(
                "https://outer.invalid/?safe=?sig=unchanged#next=https://inner.invalid/repo@branch"
            )
            .is_empty(),
        );
    }

    #[test]
    fn source_url_long_nested_tokens() {
        let text = format!(
            "{}https://user:secret@inner.invalid/pkg?sig=signed",
            "https://outer.invalid/?next=".repeat(4096),
        );
        assert_eq!(components(&text), ["secret", "signed"]);

        let text = format!("{}repo.git@branch", "https://proxy.invalid/".repeat(4096));
        assert!(components(&text).is_empty());
    }
}
