use assert_cmd::assert::OutputAssertExt;

fn assert_extra_requires_unsupported(hook: &str) {
    let context = uv_test::test_context_with_versions!(&[]);
    context
        .build_backend()
        .arg(hook)
        .assert()
        .code(2)
        .stdout("")
        .stderr("error: uv does not support extra requires\n");
}

#[test]
fn get_requires_for_build_sdist_returns_error() {
    assert_extra_requires_unsupported("get-requires-for-build-sdist");
}

#[test]
fn get_requires_for_build_wheel_returns_error() {
    assert_extra_requires_unsupported("get-requires-for-build-wheel");
}

#[test]
fn get_requires_for_build_editable_returns_error() {
    assert_extra_requires_unsupported("get-requires-for-build-editable");
}
