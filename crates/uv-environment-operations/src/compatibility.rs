use itertools::Itertools;
use uv_lock::Lock;
use uv_pep508::MarkerTree;
use uv_pypi_types::ResolverMarkerEnvironment;
use uv_python_interpreter::Interpreter;

use crate::EnvironmentError;

/// Check that the selected interpreter satisfies the lockfile's Python requirement.
pub fn validate_lock_python(
    lock: &Lock,
    interpreter: &Interpreter,
) -> Result<(), EnvironmentError> {
    if !lock
        .requires_python()
        .contains(interpreter.python_version())
    {
        return Err(EnvironmentError::LockedPythonIncompatibility(
            interpreter.python_version().clone(),
            lock.requires_python().clone(),
        ));
    }
    Ok(())
}

/// Check that the selected marker environment is supported by the lockfile.
pub fn validate_lock_platform(
    lock: &Lock,
    markers: &ResolverMarkerEnvironment,
) -> Result<(), EnvironmentError> {
    let environments = lock.supported_environments();
    if !environments.is_empty()
        && !environments
            .iter()
            .any(|environment| environment.evaluate(markers, &[]))
    {
        return Err(EnvironmentError::LockedPlatformIncompatibility(
            // Report the user's marker expressions without the implicit Python restriction.
            lock.simplified_supported_environments()
                .into_iter()
                .filter_map(MarkerTree::contents)
                .map(|environment| format!("`{environment}`"))
                .join(", "),
        ));
    }
    Ok(())
}
