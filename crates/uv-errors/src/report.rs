use std::borrow::Cow;
use std::error::Error;

use crate::{Diagnostic, DiagnosticFn};

/// One actual error node and the presentation selected for it.
pub(crate) struct ResolvedError<'a> {
    error: &'a (dyn Error + 'static),
    pub(crate) diagnostic: Diagnostic<'a>,
}

impl ResolvedError<'_> {
    pub(crate) fn message(&self) -> Cow<'_, str> {
        self.diagnostic
            .message
            .as_deref()
            .map_or_else(|| Cow::Owned(self.error.to_string()), Cow::Borrowed)
    }
}

/// Resolve the root and lazily walk its real source chain.
///
/// A presentation override applies to the next actual source. It cannot invent a source or erase
/// hints owned by that source.
pub(crate) fn resolve_error_chain<'a>(
    error: &'a (dyn Error + 'static),
    resolver: Option<DiagnosticFn>,
) -> (ResolvedError<'a>, ResolvedSources<'a>) {
    let mut diagnostic = resolver
        .and_then(|resolver| resolver(error))
        .unwrap_or_default();
    let source_override = diagnostic.source.take();
    (
        ResolvedError { error, diagnostic },
        ResolvedSources {
            previous: error,
            resolver,
            source_override,
        },
    )
}

pub(crate) struct ResolvedSources<'a> {
    previous: &'a (dyn Error + 'static),
    resolver: Option<DiagnosticFn>,
    source_override: Option<Box<Diagnostic<'a>>>,
}

impl<'a> Iterator for ResolvedSources<'a> {
    type Item = ResolvedError<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let error = self.previous.source()?;
        let native = self.resolver.and_then(|resolver| resolver(error));
        let mut diagnostic = match (self.source_override.take(), native) {
            (Some(presentation), Some(native)) => native.with_presentation_override(*presentation),
            (Some(presentation), None) => *presentation,
            (None, native) => native.unwrap_or_default(),
        };
        self.source_override = diagnostic.source.take();
        self.previous = error;
        Some(ResolvedError { error, diagnostic })
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use insta::assert_debug_snapshot;

    use crate::{Diagnostic, Hint, Hints};

    use super::resolve_error_chain;

    #[derive(Debug, thiserror::Error)]
    #[error("outer")]
    struct Outer(#[source] Inner);

    #[derive(Debug, thiserror::Error)]
    #[error("inner")]
    struct Inner;

    fn diagnostic<'a>(error: &'a (dyn Error + 'static)) -> Option<Diagnostic<'a>> {
        error.downcast_ref::<Outer>()?;
        Some(
            Diagnostic::default().with_source(Diagnostic {
                hints: [Hint::new("same"), Hint::new("same")]
                    .into_iter()
                    .collect::<Hints<'_>>(),
                ..Diagnostic::default()
            }),
        )
    }

    #[test]
    fn source_override_without_native_metadata_keeps_its_hints() {
        let error = Outer(Inner);
        let (_, mut sources) = resolve_error_chain(&error, Some(diagnostic));
        let source = sources.next().expect("the actual source");
        assert!(source.error.is::<Inner>());
        assert!(sources.next().is_none());
        assert_debug_snapshot!(source.diagnostic.hints.iter().collect::<Vec<_>>(), @r#"
        [
            "same",
            "same",
        ]
        "#);
    }
}
