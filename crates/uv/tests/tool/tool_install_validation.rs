use assert_cmd::assert::OutputAssertExt;

#[test]
fn tool_install_python() {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    let message = "error: Cannot install Python with `uv tool install`. Did you mean to use `uv python install`?\n";

    for request in ["python", "python@3.12"] {
        context
            .tool_install()
            .args([request, "--offline"])
            .assert()
            .code(2)
            .stdout("")
            .stderr(message);
    }
}

#[test]
fn tool_install_mismatched_name() {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    let wheel = "https://example.invalid/flask-3.1.0-py3-none-any.whl";
    let named_flask = format!("flask @ {wheel}");
    let named_black = format!("black @ {wheel}");

    for (request, source, source_name) in [
        ("black", wheel, "flask"),
        ("black", named_flask.as_str(), "flask"),
        ("flask", named_black.as_str(), "black"),
    ] {
        context
            .tool_install()
            .args([request, "--from", source, "--offline"])
            .assert()
            .code(2)
            .stdout("")
            .stderr(format!(
                "error: Package name (`{source_name}`) provided with `--from` does not match install request (`{request}`)\n"
            ));
    }
}
