use std::{borrow::Cow, fmt::Display};

/// Format a repeatable flag for diagnostics, eliding its values if it was passed multiple times.
pub(crate) fn format_repeated_flag<'a>(
    flag: &'a str,
    values: &[impl Display],
) -> Option<Cow<'a, str>> {
    match values {
        [] => None,
        [value] => Some(Cow::Owned(format!("{flag} {value}"))),
        _ => Some(Cow::Borrowed(flag)),
    }
}
