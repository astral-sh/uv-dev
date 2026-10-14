//! Wheel selection across environments sharing a cache.

use anyhow::Result;
use assert_cmd::prelude::*;
use assert_fs::fixture::ChildPath;
use assert_fs::prelude::*;
use indoc::indoc;

use uv_test::packse::PackseServer;
use uv_test::packse::scenario::Scenario;
use uv_test::{site_packages_path, uv_snapshot};

/// Prefer the highest cached version even when an older version was cached more recently.
#[test]
fn cache_priority() {
    let context = uv_test::test_context!("3.12");
    let index = PackseServer::new("simple/single-package.toml");
    let index_url = index.index_url();
    uv_snapshot!(context.pip_install().arg("a==2.0.0").arg("--index-url").arg(&index_url), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + a==2.0.0
    ");
    uv_snapshot!(context.pip_install().arg("a==1.0.0").arg("--index-url").arg(&index_url), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - a==2.0.0
     + a==1.0.0
    ");
    context.assert_installed("a", "1.0.0");
    context.reset_venv();
    drop(index);

    uv_snapshot!(context.pip_install()
        .arg("a")
        .arg("--index-url")
        .arg(index_url)
        .arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + a==2.0.0
    ");
    context.assert_installed("a", "2.0.0");
}

/// A cached compatible wheel must not displace a more specific wheel selected by the resolver.
/// See astral-sh/uv#12273.
#[test]
fn avoid_cached_wheel() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"]);
    let scenario: Scenario = toml::from_str(indoc! {r#"
        name = "wheel-cache-compatibility"

        [root]
        [expected]
        satisfiable = true

        [packages.a.versions."1.0.0"]
        requires_python = ">=3.11"
        requires = ["b"]
        sdist = false
        wheel_tags = ["py311-none-any", "py312-none-any"]

        [packages.b.versions."1.0.0"]
        requires_python = ">=3.11"
        sdist = false
    "#})?;
    let index = PackseServer::from_scenario(&scenario);

    let python311 = context.temp_dir.child(".venv-3.11");
    context
        .venv()
        .args(["--python", "3.11", ".venv-3.11"])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.pip_install()
        .args(["a", "--python", ".venv-3.11", "--index-url"])
        .arg(index.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using Python 3.11.[X] environment at: .venv-3.11
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + a==1.0.0
     + b==1.0.0
    ");
    ChildPath::new(site_packages_path(python311.path(), "python3.11"))
        .child("a-1.0.0.dist-info/WHEEL")
        .assert(predicates::str::contains("Tag: py311-none-any\n"));

    // `py311` is compatible with both interpreters, but `py312` is more specific on Python 3.12.
    let python312 = context.temp_dir.child(".venv-3.12");
    context
        .venv()
        .args(["--python", "3.12", ".venv-3.12"])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.pip_install()
        .args(["a", "--python", ".venv-3.12", "--index-url"])
        .arg(index.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using Python 3.12.[X] environment at: .venv-3.12
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 2 packages in [TIME]
     + a==1.0.0
     + b==1.0.0
    ");
    ChildPath::new(site_packages_path(python312.path(), "python3.12"))
        .child("a-1.0.0.dist-info/WHEEL")
        .assert(predicates::str::contains("Tag: py312-none-any\n"));

    Ok(())
}
