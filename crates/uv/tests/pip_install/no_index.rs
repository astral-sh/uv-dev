//! Installation without access to package indexes.

use anyhow::Result;
use indoc::indoc;

use uv_test::packse::{PackseServer, scenario::Scenario};
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

/// A forced reinstall cannot use the installed distribution as an artifact source.
#[test]
fn reinstall_no_index() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let scenario = toml::from_str::<Scenario>(indoc! {r#"
        name = "reinstall-without-index"
        [root]
        [expected]
        satisfiable = true
        [packages.anyio.versions."4.3.0"]
        sdist = false
        requires = ["idna==3.6", "sniffio==1.3.1"]
        [packages.idna.versions."3.6"]
        sdist = false
        [packages.sniffio.versions."1.3.1"]
        sdist = false
    "#})?;
    let index = PackseServer::from_scenario(&scenario);

    uv_snapshot!(context.pip_install()
        .arg("anyio")
        .arg("--strict")
        .arg("--default-index")
        .arg(index.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + sniffio==1.3.1
    "
    );

    uv_snapshot!(context.pip_install()
        .arg("anyio")
        .arg("--no-index")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 1 package in [TIME]
    "
    );

    uv_snapshot!(context.pip_install()
        .arg("anyio")
        .arg("--no-index")
        .arg("--reinstall")
        .arg("--strict"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because anyio was not found in the provided package locations and you require anyio, we can conclude that your requirements are unsatisfiable.

    hint: Packages were unavailable because index lookups were disabled and no additional package locations were provided (try: `--find-links <uri>`)
    "
    );
    context.assert_installed("anyio", "4.3.0");

    Ok(())
}
