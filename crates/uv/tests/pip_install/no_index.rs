//! Installation without access to package indexes.

use uv_test::uv_snapshot;

#[test]
fn install_no_index() {
    let context = uv_test::test_context!("3.12");

    uv_snapshot!(context.pip_install()
        .arg("Flask")
        .arg("--no-index"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because flask was not found in the provided package locations and you require flask, we can conclude that your requirements are unsatisfiable.

    hint: Packages were unavailable because index lookups were disabled and no additional package locations were provided (try: `--find-links <uri>`)
    ");

    context.assert_not_installed("flask");
}

/// Report the requested version when the package is unavailable.
#[test]
fn install_no_index_version() {
    let context = uv_test::test_context!("3.12");

    uv_snapshot!(context.pip_install()
        .arg("Flask==3.0.0")
        .arg("--no-index"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because flask was not found in the provided package locations and you require flask==3.0.0, we can conclude that your requirements are unsatisfiable.

    hint: Packages were unavailable because index lookups were disabled and no additional package locations were provided (try: `--find-links <uri>`)
    ");

    context.assert_not_installed("flask");
}
