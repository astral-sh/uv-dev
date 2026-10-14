use uv_test::uv_snapshot;

#[test]
fn locked() {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.filters(), context.lock().arg("--locked"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No `pyproject.toml` found in current directory or any parent directory
    ");
}
