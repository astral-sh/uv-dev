use assert_fs::fixture::PathChild;

use uv_static::EnvVars;

use uv_test::{assert_path_missing, uv_snapshot};

#[test]
fn python_dir() {
    let context = uv_test::test_context_with_versions!(&[]);

    let python_dir = context.temp_dir.child("python");
    uv_snapshot!(context.filters(), context.python_dir()
    .env(EnvVars::UV_PYTHON_INSTALL_DIR, python_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/python
    ");
    assert_path_missing(&python_dir);
}
