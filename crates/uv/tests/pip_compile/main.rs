//! Integration tests for `uv pip compile`.

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
use uv_test::pypi_proxy;

#[cfg(feature = "test-python")]
mod artifact_hash_filtering;
#[cfg(feature = "test-python")]
mod build_failures;
#[cfg(feature = "test-python")]
mod editable;
#[cfg(all(feature = "test-python", feature = "test-git"))]
mod git_checkout;
#[cfg(feature = "test-python")]
mod hash_preservation;
mod input_validation;
#[cfg(feature = "test-python")]
mod lower_bound_warnings;
#[cfg(feature = "test-python")]
mod marker_expression;
#[cfg(feature = "test-python")]
mod offline;
#[cfg(all(feature = "test-python", unix))]
mod output_streams;
#[cfg(feature = "test-python")]
mod package_locations;
#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod pip_compile;
#[cfg(all(feature = "test-python", feature = "test-universal"))]
mod pylock_paths;
#[cfg(all(feature = "test-python", feature = "test-python-eol"))]
mod requires_python;
#[cfg(feature = "test-python")]
mod source_url_hashes;
#[cfg(feature = "test-python")]
mod wheel_paths;
