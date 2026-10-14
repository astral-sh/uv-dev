//! Virtual environment setup failures that do not require an installed Python.

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

#[test]
fn create_venv_unknown_python_minor() {
    let context = uv_test::test_context_with_versions!(&[]).with_filtered_python_sources();

    let mut command = context.venv();
    command
        .arg(context.venv.as_os_str())
        // Request a version we know we'll never see
        .arg("--python")
        .arg("3.100")
        // Unset this variable to force what the user would see
        .env_remove(EnvVars::UV_PYTHON_SEARCH_PATH);

    uv_snapshot!(context.filters(), &mut command, @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found for Python 3.100 in [PYTHON SOURCES]
    "
    );

    assert_path_missing(&context.venv);
}

#[test]
fn create_venv_unknown_python_patch() {
    let context = uv_test::test_context_with_versions!(&[]).with_filtered_python_sources();

    let mut command = context.venv();
    command
        .arg(context.venv.as_os_str())
        // Request a version we know we'll never see
        .arg("--python")
        .arg("3.12.100")
        // Unset this variable to force what the user would see
        .env_remove(EnvVars::UV_PYTHON_SEARCH_PATH);

    uv_snapshot!(context.filters(), &mut command, @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found for Python 3.12.100 in [PYTHON SOURCES]
    "
    );

    assert_path_missing(&context.venv);
}

/// See <https://github.com/astral-sh/uv/issues/3280>
#[test]
#[cfg(windows)]
fn path_with_trailing_space_gives_proper_error() {
    let context = uv_test::test_context_with_versions!(&[]).with_filtered_path_os_error(3);

    // Set a custom cache directory with a trailing space
    let mut cache_dir = context.cache_dir.as_os_str().to_owned();
    cache_dir.push(" ");
    uv_snapshot!(context.filters(), context.external_command(uv_test::get_bin!())
        .arg("venv")
        .env(EnvVars::UV_CACHE_DIR, cache_dir), @r###"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Could not acquire lock
      cause: failed to open file `[CACHE_DIR]/ /.lock`: [OS ERROR 3]
    "###
    );
    // Note the extra trailing `/` in the snapshot is due to the filters, not the actual output.
}
