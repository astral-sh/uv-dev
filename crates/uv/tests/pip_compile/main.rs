//! Integration tests for `uv pip compile`.

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
use uv_test::pypi_proxy;

#[cfg(feature = "test-python")]
mod artifact_hash_filtering;
#[cfg(feature = "test-python")]
mod editable;
#[cfg(feature = "test-python")]
mod find_links_inputs;
#[cfg(all(feature = "test-python", feature = "test-git"))]
mod git_checkout;
#[cfg(feature = "test-python")]
mod hash_preservation;
#[cfg(feature = "test-python")]
mod index_inputs;
#[cfg(feature = "test-python")]
mod marker_expression;
#[cfg(all(feature = "test-python", unix))]
mod output_streams;
#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod pip_compile;
#[cfg(feature = "test-python")]
mod source_url_hashes;
