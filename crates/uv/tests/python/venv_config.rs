//! Virtual environment configuration errors that do not require an installed Python.

use uv_static::EnvVars;
use uv_test::{assert_path_missing, uv_snapshot};

#[test]
fn create_venv_with_invalid_http_timeout() {
    let context = uv_test::test_context_with_versions!(&[]);
    uv_snapshot!(context.filters(), context.venv().env(EnvVars::UV_HTTP_TIMEOUT, "not_a_number"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to parse environment variable `UV_HTTP_TIMEOUT` with invalid value `not_a_number`: invalid digit found in string; value should be an integer number of seconds
    ");
    assert_path_missing(&context.venv);
}

#[test]
fn create_venv_with_invalid_concurrent_installs() {
    let context = uv_test::test_context_with_versions!(&[]);
    uv_snapshot!(context.filters(), context.venv().env(EnvVars::UV_CONCURRENT_INSTALLS, "0"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to parse environment variable `UV_CONCURRENT_INSTALLS` with invalid value `0`: number would be zero for non-zero type
    ");
    assert_path_missing(&context.venv);
}

#[test]
fn create_venv_with_invalid_cuda_driver_version() {
    let context = uv_test::test_context_with_versions!(&[]);
    uv_snapshot!(context.filters(), context.venv().env(EnvVars::UV_CUDA_DRIVER_VERSION, "invalid"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to parse environment variable `UV_CUDA_DRIVER_VERSION` with invalid value `invalid`: expected version to start with a number, but no leading ASCII digits were found
    ");
    assert_path_missing(&context.venv);
}

#[test]
fn create_venv_with_invalid_amd_gpu_architecture() {
    let context = uv_test::test_context_with_versions!(&[]);
    uv_snapshot!(context.filters(), context.venv().env(EnvVars::UV_AMD_GPU_ARCHITECTURE, "invalid"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to parse environment variable `UV_AMD_GPU_ARCHITECTURE` with invalid value `invalid`: Unknown AMD GPU architecture: invalid
    ");
    assert_path_missing(&context.venv);
}
