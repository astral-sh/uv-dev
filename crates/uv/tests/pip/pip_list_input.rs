use uv_test::uv_snapshot;

#[test]
fn list_outdated_freeze() {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.pip_list().arg("--outdated").arg("--format").arg("freeze"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `--outdated` cannot be used with `--format freeze`
    "
    );
}

#[test]
fn list_conflicting_editable_filters() {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.filters(), context.pip_list()
        .arg("--editable")
        .arg("--exclude-editable"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the argument '--editable' cannot be used with '--exclude-editable'

    Usage: uv pip list --cache-dir [CACHE_DIR] --editable --exclude-newer <EXCLUDE_NEWER>

    For more information, try '--help'.
    "
    );
}
