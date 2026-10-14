use uv_test::uv_snapshot;

#[test]
fn get_requires_for_build_sdist_returns_error() {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.build_backend().arg("get-requires-for-build-sdist"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: uv does not support extra requires
    ");
}

#[test]
fn get_requires_for_build_wheel_returns_error() {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.build_backend().arg("get-requires-for-build-wheel"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: uv does not support extra requires
    ");
}

#[test]
fn get_requires_for_build_editable_returns_error() {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.build_backend().arg("get-requires-for-build-editable"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: uv does not support extra requires
    ");
}
