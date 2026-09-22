use std::error::Error;
use std::sync::Arc;

use uv_errors::{Diagnostic, Hinted};

use crate::{LockError, PylockTomlError};

/// Resolve lockfile-owned presentation data without changing error sources.
pub fn diagnostic_for_error<'a>(error: &'a (dyn Error + 'static)) -> Option<Diagnostic<'a>> {
    if let Some(error) = downcast_error::<LockError>(error) {
        let info = error.own_info()?;
        return Some(
            Diagnostic::default()
                .with_hints(error.own_hints())
                .with_info(info),
        );
    }
    let error = downcast_error::<PylockTomlError>(error)?;
    let info = error.own_info()?;
    Some(
        Diagnostic::default()
            .with_hints(error.own_hints())
            .with_info(info),
    )
}

fn downcast_error<'a, E: Error + 'static>(error: &'a (dyn Error + 'static)) -> Option<&'a E> {
    error
        .downcast_ref::<E>()
        .or_else(|| error.downcast_ref::<Box<E>>().map(AsRef::as_ref))
        .or_else(|| error.downcast_ref::<Arc<E>>().map(AsRef::as_ref))
}
