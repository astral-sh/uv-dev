//! Integration tests for `uv pip compile`.

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
use uv_test::pypi_proxy;

#[cfg(feature = "test-python")]
mod editable;
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
