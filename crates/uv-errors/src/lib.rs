mod diagnostic;
mod line_wrap;
mod report;
mod source;
mod structured;
mod suggestion;

use std::borrow::Cow;
use std::error::Error;
use std::fmt;
use std::iter;

use owo_colors::{AnsiColors, DynColor, OwoColorize};

pub use diagnostic::{Diagnostic, DiagnosticFn, Info};
use diagnostic::{write_hints, write_info};
use line_wrap::{get_wrap_width, wrap_text};
use report::resolve_error_chain;
pub use source::{SourceAnnotation, SourceFile, SourceSnippet};
use source::{SourceLevel, write_snippets, write_suggestion};
use structured::ErrorReport;
pub use suggestion::{SourceEdit, SourceSuggestion, SuggestionApplicability};

/// An error that may carry user-facing hints.
///
/// Implement this on error types that want to surface contextual suggestions
/// (e.g., "try `--prerelease=allow`") to the diagnostics layer.
pub trait Hinted {
    /// Return all hints associated with this error, including forwarded suggestions.
    ///
    /// This aggregate form is useful when formatting an error without walking its source chain.
    fn hints(&self) -> Hints<'_> {
        Hints::none()
    }

    /// Return only the hints owned by this error, excluding its source chain's suggestions.
    ///
    /// Source-chain renderers collect each error's suggestions separately. Concrete forwarding
    /// wrappers omit delegated suggestions; a type-erased wrapper retains its erased error's
    /// dynamically available own hints. [`Self::transparent_source`] also lets diagnostic resolvers
    /// reach metadata on a hidden concrete root.
    fn own_hints(&self) -> Hints<'_> {
        self.hints()
    }

    /// Return the inner error whose root is hidden by this error's transparent presentation.
    ///
    /// The returned error has the same visible message and source chain as this error. Ordinary
    /// causes belong in [`Error::source`] instead. Diagnostic resolvers use this method to reach
    /// metadata on a hidden root without adding a duplicate cause to the report.
    fn transparent_source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }
}

/// The display order of a user-facing hint within its owning error.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum HintOrdering {
    /// Advice that should be shown before the error's other immediate hints.
    First,
    /// Advice with no preferred placement.
    #[default]
    Any,
    /// General advice that should follow the error's complete source chain.
    Last,
}

/// A user-facing hint and its preferred display order.
pub struct Hint<'a> {
    message: Cow<'a, str>,
    ordering: HintOrdering,
    suggestion: Option<SourceSuggestion>,
}

impl<'a> Hint<'a> {
    /// Create a hint with no preferred placement.
    pub fn new(message: impl Into<Cow<'a, str>>) -> Self {
        Self {
            message: message.into(),
            ordering: HintOrdering::default(),
            suggestion: None,
        }
    }

    /// Set the preferred display order of this hint.
    #[must_use]
    pub fn with_ordering(mut self, ordering: HintOrdering) -> Self {
        self.ordering = ordering;
        self
    }

    /// Attach exact source edits owned by this hint.
    #[must_use]
    pub fn with_suggestion(mut self, suggestion: SourceSuggestion) -> Self {
        self.suggestion = Some(suggestion);
        self
    }

    /// Convert a borrowed hint to owned, extending its lifetime to `'static`.
    fn into_owned(self) -> Hint<'static> {
        Hint {
            message: Cow::Owned(self.message.into_owned()),
            ordering: self.ordering,
            suggestion: self.suggestion,
        }
    }
}

impl<'a> From<&'a str> for Hint<'a> {
    fn from(message: &'a str) -> Self {
        Self::new(message)
    }
}

impl From<String> for Hint<'_> {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

/// A collection of user-facing hint messages.
///
/// Each hint is rendered on its own line, prefixed with the styled `hint:` label.
/// Hints are grouped by [`HintOrdering`], retaining insertion order within each group.
#[derive(Default)]
pub struct Hints<'a>(Vec<Hint<'a>>);

impl<'a> Hints<'a> {
    /// No hints.
    pub fn none() -> Self {
        Self(Vec::new())
    }

    /// Add a single hint.
    pub fn push(&mut self, hint: impl Into<Hint<'a>>) {
        self.0.push(hint.into());
    }

    /// Set the display order of every hint in this collection.
    #[must_use]
    pub fn with_ordering(mut self, ordering: HintOrdering) -> Self {
        for hint in &mut self.0 {
            hint.ordering = ordering;
        }
        self
    }

    /// Convert all borrowed hints to owned, extending the lifetime to `'static`.
    pub fn into_owned(self) -> Hints<'static> {
        Hints(self.0.into_iter().map(Hint::into_owned).collect())
    }

    /// Whether the collection is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Iterate over hint messages in display order.
    pub fn iter(&self) -> HintsIter<'_, 'a> {
        HintsIter {
            hints: &self.0,
            current: self.0.iter(),
            ordering: HintOrdering::First,
            remaining: [HintOrdering::Any, HintOrdering::Last].into_iter(),
        }
    }

    /// Iterate over one ordering group without changing insertion order.
    fn iter_for_ordering(&self, ordering: HintOrdering) -> impl Iterator<Item = &Hint<'a>> {
        self.0.iter().filter(move |hint| hint.ordering == ordering)
    }

    /// Extend with another set of hints, converting borrowed hints to owned.
    ///
    /// Duplicate messages retain their first insertion position and earliest ordering. Exact
    /// edits are retained only when every duplicate refers to the same source suggestion.
    pub fn extend(&mut self, other: Hints<'_>) {
        for hint in other.0 {
            if let Some(existing) = self
                .0
                .iter_mut()
                .find(|existing| existing.message == hint.message)
            {
                existing.ordering = existing.ordering.min(hint.ordering);
                existing.suggestion = existing
                    .suggestion
                    .as_ref()
                    .zip(hint.suggestion.as_ref())
                    .and_then(|(existing, incoming)| existing.unambiguous_with(incoming));
            } else {
                self.0.push(hint.into_owned());
            }
        }
    }
}

/// A borrowed iterator over hint messages in display order.
pub struct HintsIter<'h, 'a> {
    hints: &'h [Hint<'a>],
    current: std::slice::Iter<'h, Hint<'a>>,
    ordering: HintOrdering,
    remaining: std::array::IntoIter<HintOrdering, 2>,
}

impl<'h> Iterator for HintsIter<'h, '_> {
    type Item = &'h str;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(hint) = self.current.find(|hint| hint.ordering == self.ordering) {
                return Some(hint.message.as_ref());
            }
            self.ordering = self.remaining.next()?;
            self.current = self.hints.iter();
        }
    }
}

impl<'h, 'a> IntoIterator for &'h Hints<'a> {
    type Item = &'h str;
    type IntoIter = HintsIter<'h, 'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a> IntoIterator for Hints<'a> {
    type Item = Cow<'a, str>;
    type IntoIter = std::iter::Map<std::vec::IntoIter<Hint<'a>>, fn(Hint<'a>) -> Cow<'a, str>>;

    fn into_iter(mut self) -> Self::IntoIter {
        self.0.sort_by_key(|hint| hint.ordering);
        self.0.into_iter().map(|hint| hint.message)
    }
}

/// A display adapter for an error followed by its hints.
///
/// Error renderers line-terminate the error before rendering [`Hints`]. Use
/// this adapter when an error and its hints need to be formatted together.
pub struct ErrorWithHints<'a, E> {
    error: E,
    hints: Hints<'a>,
}

impl<'a, E> ErrorWithHints<'a, E> {
    /// Format an error followed by any hints.
    pub fn new(error: E, hints: Hints<'a>) -> Self {
        Self { error, hints }
    }
}

impl<E: fmt::Display> fmt::Display for ErrorWithHints<'_, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.error)?;
        if !self.hints.is_empty() {
            writeln!(f)?;
            write!(f, "{}", self.hints)?;
        }
        Ok(())
    }
}

impl<'a> From<&'a str> for Hints<'a> {
    fn from(hint: &'a str) -> Self {
        Self::from(Hint::from(hint))
    }
}

impl From<String> for Hints<'_> {
    fn from(hint: String) -> Self {
        Self::from(Hint::from(hint))
    }
}

impl<'a> From<Hint<'a>> for Hints<'a> {
    fn from(hint: Hint<'a>) -> Self {
        Self(vec![hint])
    }
}

impl<'a, T: Into<Hint<'a>>> FromIterator<T> for Hints<'a> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self(iter.into_iter().map(Into::into).collect())
    }
}

impl fmt::Display for Hints<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for hint in self {
            write!(f, "\n{HintPrefix} {hint}")?;
        }
        Ok(())
    }
}

/// A styled `hint:` prefix for use in user-facing messages.
pub struct HintPrefix;

impl fmt::Display for HintPrefix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", "hint".bold().cyan(), ":".bold())
    }
}

/// The output representation for an error chain.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ErrorFormat {
    /// Render a human-readable terminal diagnostic.
    #[default]
    Text,
    /// Serialize an experimental [`ErrorReport`] as one JSON object per line.
    Json,
}

/// Options for formatting an error chain.
#[must_use]
pub struct ErrorOptions<'a, C = AnsiColors, W = Stderr> {
    level: Cow<'a, str>,
    color: C,
    format: ErrorFormat,
    width_override: Option<usize>,
    stream: W,
    diagnostic: Option<DiagnosticFn>,
}

/// A standard-error writer for formatted error chains.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stderr;

impl fmt::Write for Stderr {
    fn write_str(&mut self, output: &str) -> fmt::Result {
        anstream::eprint!("{output}");
        Ok(())
    }
}

impl Default for ErrorOptions<'_, AnsiColors, Stderr> {
    fn default() -> Self {
        Self {
            level: Cow::Borrowed("error"),
            color: AnsiColors::Red,
            format: ErrorFormat::Text,
            width_override: None,
            stream: Stderr,
            diagnostic: None,
        }
    }
}

impl<'a, C, W> ErrorOptions<'a, C, W> {
    /// Use a custom level prefix, such as `warning`.
    pub fn with_level(mut self, level: impl Into<Cow<'a, str>>) -> Self {
        self.level = level.into();
        self
    }

    /// Use a custom color for the level and cause prefixes.
    pub fn with_color<D>(self, color: D) -> ErrorOptions<'a, D, W> {
        ErrorOptions {
            level: self.level,
            color,
            format: self.format,
            width_override: self.width_override,
            stream: self.stream,
            diagnostic: self.diagnostic,
        }
    }

    /// Choose the output representation for this report.
    pub fn with_format(mut self, format: ErrorFormat) -> Self {
        self.format = format;
        self
    }

    /// Override the terminal width used for wrapping.
    ///
    /// This is primarily useful for testing.
    pub fn with_width_override(mut self, width_override: usize) -> Self {
        self.width_override = Some(width_override);
        self
    }

    /// Write the rendered error chain to a custom stream.
    pub fn with_stream<D>(self, stream: D) -> ErrorOptions<'a, C, D> {
        ErrorOptions {
            level: self.level,
            color: self.color,
            format: self.format,
            width_override: self.width_override,
            stream,
            diagnostic: self.diagnostic,
        }
    }

    /// Resolve presentation data for each error in the chain.
    pub fn with_diagnostic(mut self, diagnostic: DiagnosticFn) -> Self {
        self.diagnostic = Some(diagnostic);
        self
    }
}

/// Format an error chain and explicitly supplied hints to standard error using the default level
/// and color.
pub fn write_error_chain(err: &(dyn Error + 'static), hints: &Hints<'_>) -> fmt::Result {
    write_error_chain_with_options(err, hints, ErrorOptions::default())
}

/// Format the [`Debug`] representation of every error in an error chain.
pub fn debug_error_chain(err: &dyn Error) -> impl fmt::Display + '_ {
    DebugErrorChain(err)
}

struct DebugErrorChain<'a>(&'a dyn Error);

impl fmt::Display for DebugErrorChain<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, error) in iter::successors(Some(self.0), |&error| error.source()).enumerate() {
            if index > 0 {
                formatter.write_str("\n")?;
            }
            write!(formatter, "{index}: {error:?}")?;
        }
        Ok(())
    }
}

/// Formats an error or warning chain with custom options.
///
/// Each hint is rendered on its own line, prefixed with the styled `hint:` label.
pub fn write_error_chain_with_options<C: DynColor + Copy, W: fmt::Write>(
    err: &(dyn Error + 'static),
    hints: &Hints<'_>,
    options: ErrorOptions<'_, C, W>,
) -> fmt::Result {
    let ErrorOptions {
        level,
        color,
        format,
        width_override,
        mut stream,
        diagnostic,
    } = options;
    if format == ErrorFormat::Json {
        let report = ErrorReport::new(err, diagnostic)
            .with_level(&level)
            .with_trailing_hints(hints);
        return structured::write_report(&mut stream, &report);
    }
    let width = get_wrap_width(width_override);

    let (main, sources) = resolve_error_chain(err, diagnostic);
    let main_msg = main.message();
    let main_padding = " ".repeat(level.len() + 2);
    let wrapped_main = wrap_text(&main_msg, width, &main_padding, &main_padding, "");
    writeln!(
        &mut stream,
        "{}{} {}",
        level.as_ref().color(color).bold(),
        ":".bold(),
        wrapped_main.trim()
    )?;
    write_snippets(
        &mut stream,
        &main.diagnostic.snippets,
        width,
        SourceLevel::for_error(&level),
    )?;
    write_info(&mut stream, &main.diagnostic.info, width)?;
    write_hints(
        &mut stream,
        &main.diagnostic.hints,
        HintOrdering::First,
        width,
    )?;
    write_hints(
        &mut stream,
        &main.diagnostic.hints,
        HintOrdering::Any,
        width,
    )?;

    // Keep each owner's hints together while walking the real source chain. Unwinding in reverse
    // order places general outer advice after more specific source advice.
    let mut trailing_hints = vec![main.diagnostic.hints];
    for source in sources {
        let msg = source.message();
        // Reserve the display width of the prefix before wrapping the message. Authored lines
        // retain their own indentation beneath it.
        let wrapped = wrap_text(&msg, width.map(|width| width.saturating_sub(9)), "", "", "");

        let mut lines = wrapped.lines();
        if let Some(first) = lines.next() {
            writeln!(
                &mut stream,
                "  {}{} {}",
                "cause".color(color).bold(),
                ":".bold(),
                first.trim()
            )?;
            for line in lines {
                if line.trim().is_empty() {
                    writeln!(&mut stream)?;
                } else {
                    writeln!(&mut stream, "         {line}")?;
                }
            }
        }
        write_snippets(
            &mut stream,
            &source.diagnostic.snippets,
            width,
            SourceLevel::for_error(&level),
        )?;
        write_info(&mut stream, &source.diagnostic.info, width)?;
        write_hints(
            &mut stream,
            &source.diagnostic.hints,
            HintOrdering::First,
            width,
        )?;
        write_hints(
            &mut stream,
            &source.diagnostic.hints,
            HintOrdering::Any,
            width,
        )?;
        trailing_hints.push(source.diagnostic.hints);
    }

    for hints in trailing_hints.iter().rev() {
        write_hints(&mut stream, hints, HintOrdering::Last, width)?;
    }

    for ordering in [HintOrdering::First, HintOrdering::Any, HintOrdering::Last] {
        for hint in hints.iter_for_ordering(ordering) {
            writeln!(&mut stream, "\n{HintPrefix} {}", hint.message)?;
            if let Some(suggestion) = &hint.suggestion {
                write_suggestion(&mut stream, suggestion, width)?;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use anyhow::anyhow;
    use indoc::indoc;
    use insta::{assert_debug_snapshot, assert_snapshot};
    use owo_colors::AnsiColors;

    use super::{
        Diagnostic, ErrorFormat, ErrorOptions, ErrorWithHints, Hint, HintOrdering, Hints, Info,
        debug_error_chain, write_error_chain_with_options,
    };

    #[test]
    fn writes_one_json_report_with_outer_trailing_hints() -> Result<(), Box<dyn Error>> {
        let error = anyhow!("inner").context("outer");
        let hints = [
            Hint::new("general"),
            Hint::new("specific").with_ordering(HintOrdering::First),
        ]
        .into_iter()
        .collect();
        let mut output = String::new();
        write_error_chain_with_options(
            error.as_ref(),
            &hints,
            ErrorOptions::default()
                .with_level("warning")
                .with_format(ErrorFormat::Json)
                .with_width_override(10)
                .with_stream(&mut output),
        )?;
        assert!(output.ends_with('\n'));
        assert_eq!(output.lines().count(), 1);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&output)?,
            serde_json::json!({
                "schema_version": 1,
                "coordinates": {
                    "line_base": 1,
                    "column_base": 0,
                    "column_encoding": "utf-8"
                },
                "level": "warning",
                "errors": [
                    {
                        "message": "outer",
                        "hints": [
                            {"message": "specific", "ordering": "last"},
                            {"message": "general", "ordering": "last"}
                        ]
                    },
                    {"message": "inner"}
                ]
            })
        );
        Ok(())
    }

    #[test]
    fn extend_deduplicates_matching_hints() {
        let mut hints = Hints::from("same");
        hints.extend(Hints::from("same"));
        hints.extend(Hints::from("other"));

        let hints = hints.iter().collect::<Vec<_>>();
        assert_debug_snapshot!(hints, @r#"
        [
            "same",
            "other",
        ]
        "#);
    }

    #[test]
    fn hint_ordering_retains_insertion_order() {
        let mut hints = Hints::from("any 1");
        hints.push(Hint::new("last 1").with_ordering(HintOrdering::Last));
        hints.push(Hint::new("first 1").with_ordering(HintOrdering::First));
        hints.push("any 2".to_string());
        hints.extend(Hints::from("last 2").with_ordering(HintOrdering::Last));
        hints.extend(Hints::from("first 2").with_ordering(HintOrdering::First));

        assert_snapshot!(anstream::adapter::strip_str(&hints.to_string()), @"
        hint: first 1
        hint: first 2
        hint: any 1
        hint: any 2
        hint: last 1
        hint: last 2
        ");
        assert_debug_snapshot!((&hints).into_iter().collect::<Vec<_>>(), @r#"
        [
            "first 1",
            "first 2",
            "any 1",
            "any 2",
            "last 1",
            "last 2",
        ]
        "#);
        assert_debug_snapshot!(hints.into_iter().collect::<Vec<_>>(), @r#"
        [
            "first 1",
            "first 2",
            "any 1",
            "any 2",
            "last 1",
            "last 2",
        ]
        "#);
    }

    #[test]
    fn changing_ordering_retains_insertion_order() {
        let hints = [
            Hint::new("last").with_ordering(HintOrdering::Last),
            Hint::new("first").with_ordering(HintOrdering::First),
            Hint::new("any"),
        ]
        .into_iter()
        .collect::<Hints<'_>>()
        .with_ordering(HintOrdering::Any);

        assert_debug_snapshot!(hints.iter().collect::<Vec<_>>(), @r#"
        [
            "last",
            "first",
            "any",
        ]
        "#);
    }

    #[test]
    fn duplicate_hints_retain_the_earliest_ordering() {
        let message = String::from("shared");
        let mut hints = Hints::from(message.as_str())
            .with_ordering(HintOrdering::Last)
            .into_owned();
        drop(message);

        hints.extend(Hints::from("first").with_ordering(HintOrdering::First));
        hints.extend(Hints::from("shared").with_ordering(HintOrdering::First));
        hints.extend(Hints::from("shared").with_ordering(HintOrdering::Last));
        hints.extend(Hints::from("any"));

        assert_snapshot!(anstream::adapter::strip_str(&hints.to_string()), @"
        hint: shared
        hint: first
        hint: any
        ");
    }

    #[test]
    fn error_with_hints_separates_hints_from_error() {
        let output = ErrorWithHints::new("error", Hints::from("fix it")).to_string();
        assert_snapshot!(anstream::adapter::strip_str(&output), @"
        error

        hint: fix it
        ");
        assert_snapshot!(ErrorWithHints::new("error", Hints::none()), @"error");
    }

    #[test]
    fn test_error_wrapping_with_columns() {
        #[derive(Debug, thiserror::Error)]
        #[error(
            "Because fiasobfhuasbf was not found in the package registry and you require fiasobfhuasbf, we can conclude that your requirements are unsatisfiable."
        )]
        struct Inner;

        #[derive(Debug, thiserror::Error)]
        #[error("No solution found when resolving dependencies")]
        struct Outer {
            #[source]
            source: Inner,
        }

        let error = Outer { source: Inner };
        let mut output = String::new();
        write_error_chain_with_options(
            &error,
            &Hints::none(),
            ErrorOptions::default()
                .with_width_override(80)
                .with_stream(&mut output),
        )
        .unwrap();
        let output = anstream::adapter::strip_str(&output);

        assert_snapshot!(output, @"
        error: No solution found when resolving dependencies
          cause: Because fiasobfhuasbf was not found in the package registry and you
                 require fiasobfhuasbf, we can conclude that your requirements are
                 unsatisfiable.
        ");
    }

    #[test]
    fn test_error_chain_with_cause() {
        #[derive(Debug, thiserror::Error)]
        #[error("Permission denied")]
        struct Inner;

        #[derive(Debug, thiserror::Error)]
        #[error("Failed to write file")]
        struct Outer {
            #[source]
            source: Inner,
        }

        let error = Outer { source: Inner };
        let mut output = String::new();
        write_error_chain_with_options(
            &error,
            &Hints::none(),
            ErrorOptions::default().with_stream(&mut output),
        )
        .unwrap();
        assert_snapshot!(format!("{output:?}"), @r#""\u{1b}[1m\u{1b}[31merror\u{1b}[39m\u{1b}[0m\u{1b}[1m:\u{1b}[0m Failed to write file\n  \u{1b}[1m\u{1b}[31mcause\u{1b}[39m\u{1b}[0m\u{1b}[1m:\u{1b}[0m Permission denied\n""#);
        let output = anstream::adapter::strip_str(&output);

        assert_snapshot!(output, @"
        error: Failed to write file
          cause: Permission denied
        ");
    }

    #[test]
    fn formats_debug_error_chain() {
        #[derive(Debug, thiserror::Error)]
        #[error("inner error")]
        struct InnerError {
            code: u8,
        }

        #[derive(Debug, thiserror::Error)]
        #[error("outer error")]
        struct OuterError {
            #[source]
            source: InnerError,
        }

        let error = OuterError {
            source: InnerError { code: 42 },
        };

        assert_eq!(
            debug_error_chain(&error).to_string(),
            "0: OuterError { source: InnerError { code: 42 } }\n1: InnerError { code: 42 }"
        );
    }

    #[test]
    fn format_with_custom_level() {
        let error = anyhow!("Failed to create registry entry");
        let mut output = String::new();
        write_error_chain_with_options(
            error.as_ref(),
            &Hints::none(),
            ErrorOptions::default()
                .with_level("warning")
                .with_color(AnsiColors::Yellow)
                .with_stream(&mut output),
        )
        .unwrap();
        let output = anstream::adapter::strip_str(&output);

        assert_snapshot!(output, @"warning: Failed to create registry entry
");
    }

    #[test]
    fn test_no_hyphenation() {
        #[derive(Debug, thiserror::Error)]
        #[error(
            "Failed to download package from https://files.pythonhosted.org/packages/verylongpackagename"
        )]
        struct LongWord;

        let error = LongWord;
        let mut output = String::new();
        write_error_chain_with_options(
            &error,
            &Hints::none(),
            ErrorOptions::default()
                .with_width_override(50)
                .with_stream(&mut output),
        )
        .unwrap();
        let output = anstream::adapter::strip_str(&output);
        assert_snapshot!(output, @r"
        error: Failed to download package from
               https://files.pythonhosted.org/packages/verylongpackagename
        ");
    }

    #[test]
    fn test_long_words_not_broken() {
        #[derive(Debug, thiserror::Error)]
        #[error(
            "The package supercalifragilisticexpialidocious-extraordinarily-long-name was not found"
        )]
        struct VeryLongWord;

        let error = VeryLongWord;
        let mut output = String::new();
        write_error_chain_with_options(
            &error,
            &Hints::none(),
            ErrorOptions::default()
                .with_width_override(40)
                .with_stream(&mut output),
        )
        .unwrap();
        let output = anstream::adapter::strip_str(&output);
        assert_snapshot!(output, @r"
        error: The package
               supercalifragilisticexpialidocious-extraordinarily-long-name
               was not found
        ");
    }

    #[test]
    fn test_multiple_error_sources() {
        #[derive(Debug, thiserror::Error)]
        #[error("Network connection timeout after multiple retry attempts")]
        struct DeepError;

        #[derive(Debug, thiserror::Error)]
        #[error("Failed to fetch package metadata from registry")]
        struct MiddleError {
            #[source]
            source: DeepError,
        }

        #[derive(Debug, thiserror::Error)]
        #[error("Unable to resolve package dependencies")]
        struct TopError {
            #[source]
            source: MiddleError,
        }

        let error = TopError {
            source: MiddleError { source: DeepError },
        };
        let mut output = String::new();
        write_error_chain_with_options(
            &error,
            &Hints::none(),
            ErrorOptions::default()
                .with_width_override(40)
                .with_stream(&mut output),
        )
        .unwrap();
        let output = anstream::adapter::strip_str(&output);
        assert_snapshot!(output, @"
        error: Unable to resolve package
               dependencies
          cause: Failed to fetch package
                 metadata from registry
          cause: Network connection timeout
                 after multiple retry attempts
        ");
    }

    #[test]
    fn format_cause_with_narrow_width() {
        let error = anyhow!("one two").context("root");
        let mut output = String::new();
        write_error_chain_with_options(
            error.as_ref(),
            &Hints::none(),
            ErrorOptions::default()
                .with_width_override(4)
                .with_stream(&mut output),
        )
        .unwrap();
        let output = anstream::adapter::strip_str(&output);

        assert_snapshot!(output, @"
        error: root
          cause: one
                 two
        ");
    }

    #[test]
    fn test_multiline_main_message_wraps_each_line() {
        #[derive(Debug, thiserror::Error)]
        #[error(
            "There is no command `foobar` for `uv`. Did you mean one of:\n    auth\n    run\n    init"
        )]
        struct Suggestions;

        let error = Suggestions;
        let mut output = String::new();
        write_error_chain_with_options(
            &error,
            &Hints::none(),
            ErrorOptions::default()
                .with_width_override(50)
                .with_stream(&mut output),
        )
        .unwrap();
        let output = anstream::adapter::strip_str(&output);

        assert_snapshot!(output, @r"
        error: There is no command `foobar` for `uv`. Did
               you mean one of:
            auth
            run
            init
        ");
    }

    #[test]
    fn test_wrap_only_on_ascii_space() {
        #[derive(Debug, thiserror::Error)]
        #[error("Path /usr/local/lib/python3.12/site-packages not found in filesystem hierarchy")]
        struct SpecialChars;

        let error = SpecialChars;
        let mut output = String::new();
        write_error_chain_with_options(
            &error,
            &Hints::none(),
            ErrorOptions::default()
                .with_width_override(50)
                .with_stream(&mut output),
        )
        .unwrap();
        let output = anstream::adapter::strip_str(&output);
        assert_snapshot!(output, @r"
        error: Path /usr/local/lib/python3.12/site-packages
               not found in filesystem hierarchy
        ");
    }

    #[test]
    fn format_with_hints() {
        let err = anyhow!("Permission denied").context("Failed to fetch package");

        let hints = [
            "Try running with `--verbose` for more information.".to_string(),
            "Try running without --offline.".to_string(),
        ]
        .into_iter()
        .collect();

        let mut rendered = String::new();
        write_error_chain_with_options(
            err.as_ref(),
            &hints,
            ErrorOptions::default().with_stream(&mut rendered),
        )
        .unwrap();
        let rendered = anstream::adapter::strip_str(&rendered);

        assert_snapshot!(rendered, @"
        error: Failed to fetch package
          cause: Permission denied

        hint: Try running with `--verbose` for more information.

        hint: Try running without --offline.
        ");
    }

    #[test]
    fn format_multiline_message() {
        let err_middle = indoc! {"Failed to fetch https://example.com/upload/python3.13.tar.zst
        Server says: This endpoint only support POST requests.

        For downloads, please refer to https://example.com/download/python3.13.tar.zst"};
        let err = anyhow!("Caused By: HTTP Error 400")
            .context(err_middle)
            .context("Failed to download Python 3.12");

        let mut rendered = String::new();
        write_error_chain_with_options(
            err.as_ref(),
            &Hints::none(),
            ErrorOptions::default().with_stream(&mut rendered),
        )
        .unwrap();
        let rendered = anstream::adapter::strip_str(&rendered);

        assert_snapshot!(rendered, @"
        error: Failed to download Python 3.12
          cause: Failed to fetch https://example.com/upload/python3.13.tar.zst
                 Server says: This endpoint only support POST requests.

                 For downloads, please refer to https://example.com/download/python3.13.tar.zst
          cause: Caused By: HTTP Error 400
        ");
    }

    #[derive(Debug, thiserror::Error)]
    #[error("HTTP error 400 Bad Request")]
    struct HttpError;

    #[derive(Debug, thiserror::Error)]
    #[error("Failed to fetch {url}. Server says: {body}")]
    struct FetchError {
        url: String,
        body: String,
        #[source]
        source: HttpError,
    }

    fn fetch_diagnostic<'a>(error: &'a (dyn Error + 'static)) -> Option<Diagnostic<'a>> {
        let error = error.downcast_ref::<FetchError>()?;
        Some(
            Diagnostic::new(format!("Failed to fetch {}", error.url)).with_info(
                Info::new("The server included the following context:")
                    .with_details(error.body.as_str()),
            ),
        )
    }

    #[test]
    fn format_info_between_causes() {
        let error = anyhow!(FetchError {
            url: "https://example.com/python.tar.zst".to_string(),
            body: "This endpoint accepts POST requests only.\n\nUse /download/ instead."
                .to_string(),
            source: HttpError,
        })
        .context("Failed to download Python 3.13");
        let mut output = String::new();
        write_error_chain_with_options(
            error.as_ref(),
            &Hints::from("Check the download URL"),
            ErrorOptions::default()
                .with_diagnostic(fetch_diagnostic)
                .with_stream(&mut output),
        )
        .unwrap();
        assert_snapshot!(anstream::adapter::strip_str(&output), @"
        error: Failed to download Python 3.13
          cause: Failed to fetch https://example.com/python.tar.zst
          info: The server included the following context:
            |
            | This endpoint accepts POST requests only.
            |
            | Use /download/ instead.
          cause: HTTP error 400 Bad Request

        hint: Check the download URL
        ");
    }

    #[test]
    fn format_info_on_root() {
        let mut output = String::new();
        write_error_chain_with_options(
            &HttpError,
            &Hints::none(),
            ErrorOptions::default()
                .with_diagnostic(|error| {
                    error.downcast_ref::<HttpError>().map(|_| {
                        Diagnostic::default()
                            .with_info(Info::new("First detail"))
                            .with_info(Info::new("Second detail").with_details(""))
                    })
                })
                .with_stream(&mut output),
        )
        .unwrap();
        assert_snapshot!(anstream::adapter::strip_str(&output), @"
        error: HTTP error 400 Bad Request
          info: First detail
          info: Second detail
        ");
    }

    #[test]
    fn format_info_wrapping() {
        let error = FetchError {
            url: "https://example.com".to_string(),
            body: "First paragraph has several words.\n\n  Indented second paragraph.".to_string(),
            source: HttpError,
        };
        let mut output = String::new();
        write_error_chain_with_options(
            &error,
            &Hints::none(),
            ErrorOptions::default()
                .with_level("warning")
                .with_color(AnsiColors::Yellow)
                .with_width_override(30)
                .with_diagnostic(fetch_diagnostic)
                .with_stream(&mut output),
        )
        .unwrap();
        assert_snapshot!(anstream::adapter::strip_str(&output), @"
        warning: Failed to fetch
                 https://example.com
          info: The server included
                the following context:
            |
            | First paragraph has
            | several words.
            |
            |   Indented second
            | paragraph.
          cause: HTTP error 400 Bad
                 Request
        ");
    }

    #[test]
    fn format_untrusted_info_details() {
        let mut output = String::new();
        write_error_chain_with_options(
            &HttpError,
            &Hints::none(),
            ErrorOptions::default()
                .with_diagnostic(|_| {
                    Some(Diagnostic::default().with_info(Info::new("Server response:").with_details(
                        "café 👩‍💻\r\n\tindented\n\u{1b}[31mred\u{1b}[0m\n\u{1b}]8;;https://example.com\u{7}link\u{85}\u{202e}text\u{2029}\rrewritten",
                    )))
                })
                .with_stream(&mut output),
        )
        .unwrap();
        assert_snapshot!(anstream::adapter::strip_str(&output), @r"
        error: HTTP error 400 Bad Request
          info: Server response:
            |
            | café 👩‍💻
            |     indented
            | \u{1b}[31mred\u{1b}[0m
            | \u{1b}]8;;https://example.com\u{7}link\u{85}\u{202e}text\u{2029}\rrewritten
        ");
    }

    #[test]
    fn format_source_diagnostic_overrides() {
        #[derive(Debug, thiserror::Error)]
        #[error("Outer error")]
        struct Outer(#[source] Middle);

        #[derive(Debug, thiserror::Error)]
        #[error("Middle error")]
        struct Middle(#[source] Inner);

        #[derive(Debug, thiserror::Error)]
        #[error("Inner error")]
        struct Inner(#[source] HttpError);

        let error = Outer(Middle(Inner(HttpError)));
        let mut output = String::new();
        write_error_chain_with_options(
            &error,
            &Hints::none(),
            ErrorOptions::default()
                .with_diagnostic(|error| {
                    if error.is::<Outer>() {
                        Some(
                            Diagnostic::default().with_source(
                                Diagnostic::default()
                                    .with_info(Info::new("Context from the outer error"))
                                    .with_source(Diagnostic::new("Explicit inner message")),
                            ),
                        )
                    } else if error.is::<Middle>() || error.is::<Inner>() {
                        Some(Diagnostic::new("Ordinary callback message"))
                    } else if error.is::<HttpError>() {
                        Some(
                            Diagnostic::new("Resolved HTTP error")
                                .with_source(Diagnostic::new("Not an actual source")),
                        )
                    } else {
                        None
                    }
                })
                .with_stream(&mut output),
        )
        .unwrap();
        assert_snapshot!(anstream::adapter::strip_str(&output), @"
        error: Outer error
          cause: Middle error
          info: Context from the outer error
          cause: Explicit inner message
          cause: Resolved HTTP error
        ");
    }

    #[test]
    fn format_hints_with_their_owner_and_source_subtree() {
        #[derive(Debug, thiserror::Error)]
        #[error("Outer operation failed")]
        struct Outer(#[source] Middle);

        #[derive(Debug, thiserror::Error)]
        #[error("Middle operation failed")]
        struct Middle(#[source] HttpError);

        let error = Outer(Middle(HttpError));
        let mut output = String::new();
        write_error_chain_with_options(
            &error,
            &Hints::from("Explicit report-level fallback"),
            ErrorOptions::default()
                .with_width_override(80)
                .with_diagnostic(|error| {
                    if error.is::<Outer>() {
                        let hints = [
                            Hint::new("Outer trailing advice 1").with_ordering(HintOrdering::Last),
                            Hint::new("Outer immediate advice"),
                            Hint::new("Outer first advice").with_ordering(HintOrdering::First),
                            Hint::new("Outer trailing advice 2").with_ordering(HintOrdering::Last),
                        ]
                        .into_iter()
                        .collect();
                        Some(Diagnostic::default().with_hints(hints))
                    } else if error.is::<Middle>() {
                        Some(
                            Diagnostic::default()
                                .with_info(Info::new("Middle context"))
                                .with_hints(Hints::from("Middle immediate advice"))
                                .with_hints(
                                    Hints::from("Middle trailing advice")
                                        .with_ordering(HintOrdering::Last),
                                ),
                        )
                    } else if error.is::<HttpError>() {
                        Some(
                            Diagnostic::default()
                                .with_hints(Hints::from("HTTP-specific advice"))
                                .with_hints(
                                    Hints::from("HTTP trailing advice")
                                        .with_ordering(HintOrdering::Last),
                                ),
                        )
                    } else {
                        None
                    }
                })
                .with_stream(&mut output),
        )
        .unwrap();

        assert_snapshot!(anstream::adapter::strip_str(&output), @"
        error: Outer operation failed
          hint: Outer first advice
          hint: Outer immediate advice
          cause: Middle operation failed
          info: Middle context
          hint: Middle immediate advice
          cause: HTTP error 400 Bad Request
          hint: HTTP-specific advice
          hint: HTTP trailing advice
          hint: Middle trailing advice
          hint: Outer trailing advice 1
          hint: Outer trailing advice 2

        hint: Explicit report-level fallback
        ");
    }

    #[test]
    fn format_source_override_retains_native_hints() {
        #[derive(Debug, thiserror::Error)]
        #[error("Outer operation failed")]
        struct Outer(#[source] Middle);

        #[derive(Debug, thiserror::Error)]
        #[error("Middle operation failed")]
        struct Middle(#[source] HttpError);

        let error = Outer(Middle(HttpError));
        let mut output = String::new();
        write_error_chain_with_options(
            &error,
            &Hints::none(),
            ErrorOptions::default()
                .with_width_override(80)
                .with_diagnostic(|error| {
                    if error.is::<Outer>() {
                        Some(
                            Diagnostic::default()
                                .with_hints(
                                    Hints::from("Outer trailing advice")
                                        .with_ordering(HintOrdering::Last),
                                )
                                .with_source(
                                    Diagnostic::new("Presented middle error")
                                        .with_hints(Hints::from("Context-dependent middle advice"))
                                        .with_hints(
                                            Hints::from("Presented middle trailing advice")
                                                .with_ordering(HintOrdering::Last),
                                        )
                                        .with_source(
                                            Diagnostic::new("Presented HTTP error").with_hints(
                                                Hints::from("Context-dependent HTTP advice"),
                                            ),
                                        ),
                                ),
                        )
                    } else if error.is::<Middle>() {
                        Some(
                            Diagnostic::new("Ordinary middle message")
                                .with_info(Info::new("Ordinary middle context"))
                                .with_hints(
                                    Hints::from("Middle specific advice")
                                        .with_ordering(HintOrdering::First),
                                )
                                .with_hints(
                                    Hints::from("Middle trailing advice")
                                        .with_ordering(HintOrdering::Last),
                                ),
                        )
                    } else if error.is::<HttpError>() {
                        Some(
                            Diagnostic::default()
                                .with_hints(Hints::from("HTTP-specific advice"))
                                .with_hints(
                                    Hints::from("HTTP trailing advice")
                                        .with_ordering(HintOrdering::Last),
                                ),
                        )
                    } else {
                        None
                    }
                })
                .with_stream(&mut output),
        )
        .unwrap();

        assert_snapshot!(anstream::adapter::strip_str(&output), @"
        error: Outer operation failed
          cause: Presented middle error
          hint: Middle specific advice
          hint: Context-dependent middle advice
          cause: Presented HTTP error
          hint: HTTP-specific advice
          hint: Context-dependent HTTP advice
          hint: HTTP trailing advice
          hint: Middle trailing advice
          hint: Presented middle trailing advice
          hint: Outer trailing advice
        ");
    }

    #[test]
    fn format_owned_hint_wrapping() {
        let mut output = String::new();
        write_error_chain_with_options(
            &HttpError,
            &Hints::none(),
            ErrorOptions::default()
                .with_width_override(35)
                .with_diagnostic(|_| {
                    Some(Diagnostic::default().with_hints(Hints::from(
                        "First paragraph contains several words.\n\n  Indented example.",
                    )))
                })
                .with_stream(&mut output),
        )
        .unwrap();
        assert_snapshot!(anstream::adapter::strip_str(&output), @"
        error: HTTP error 400 Bad Request
          hint: First paragraph contains
                several words.

                  Indented example.
        ");
    }
}
