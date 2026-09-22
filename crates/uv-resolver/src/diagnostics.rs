use std::error::Error;

use uv_errors::Diagnostic;

use crate::NoSolutionError;

/// Resolve retained requirement locations without changing the resolver error or its sources.
pub fn diagnostic_for_error<'a>(error: &'a (dyn Error + 'static)) -> Option<Diagnostic<'a>> {
    error
        .downcast_ref::<NoSolutionError>()
        .or_else(|| {
            error
                .downcast_ref::<Box<NoSolutionError>>()
                .map(AsRef::as_ref)
        })?
        .diagnostic()
}
