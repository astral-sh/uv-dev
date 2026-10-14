use uv_static::EnvVars;
use uv_test::uv_snapshot;

#[test]
fn tool_run_hints_when_python_downloads_are_disabled() {
    let context = uv_test::test_context_with_versions!(&[]).with_filtered_python_sources();

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("python@3.12")
        .env(EnvVars::UV_PYTHON_DOWNLOADS, "never"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found for Python 3.12 in [PYTHON SOURCES]

    hint: A managed Python download is available for Python 3.12, but Python downloads are set to 'never'
    ");
}

#[test]
fn tool_run_hints_when_python_downloads_are_offline() {
    let context = uv_test::test_context_with_versions!(&[]).with_filtered_python_sources();

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("python@3.12")
        .env(EnvVars::UV_PYTHON_DOWNLOADS, "auto")
        .env(EnvVars::UV_OFFLINE, "true"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found for Python 3.12 in [PYTHON SOURCES]

    hint: A managed Python download is available for Python 3.12, but uv is set to offline mode
    ");
}

#[test]
fn tool_run_hints_when_managed_python_is_disabled() {
    let context = uv_test::test_context_with_versions!(&[]).with_filtered_python_sources();

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("python@3.12")
        .env(EnvVars::UV_PYTHON_DOWNLOADS, "auto")
        .env(EnvVars::UV_NO_MANAGED_PYTHON, "true"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found for Python 3.12 in [PYTHON SOURCES]

    hint: A managed Python download is available for Python 3.12, but the Python preference is set to 'only system'
    ");
}

#[test]
fn tool_run_managed_python_cli_preference_controls_hint() {
    let context = uv_test::test_context_with_versions!(&[]).with_filtered_python_sources();

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--no-managed-python")
        .arg("python@3.12")
        .env(EnvVars::UV_PYTHON_DOWNLOADS, "auto")
        .env(EnvVars::UV_OFFLINE, "true")
        .env(EnvVars::UV_MANAGED_PYTHON, "true"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found for Python 3.12 in [PYTHON SOURCES]

    hint: A managed Python download is available for Python 3.12, but the Python preference is set to 'only system'
    ");
}
