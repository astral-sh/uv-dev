use uv_test::uv_snapshot;

#[test]
fn list_conflicts_with_force_pep517() {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.filters(), context.build()
        .arg(".")
        .args(["--out-dir", "dist", "--list", "--force-pep517"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the argument '--list' cannot be used with '--force-pep517'

    Usage: uv build --cache-dir [CACHE_DIR] --out-dir <OUT_DIR> --exclude-newer <EXCLUDE_NEWER> <SRC>

    For more information, try '--help'.
    ");
}
