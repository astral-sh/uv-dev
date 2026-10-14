//! Wheel selection across environments sharing a cache.

use anyhow::Result;
use assert_cmd::prelude::*;
use assert_fs::fixture::ChildPath;
use assert_fs::prelude::*;
use indoc::indoc;

use uv_test::packse::PackseServer;
use uv_test::packse::scenario::Scenario;
use uv_test::site_packages_path;

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

    // `py311` is compatible with both interpreters, but `py312` is more specific on Python 3.12.
    for (python, prepared, tag) in [
        ("3.11", "2 packages", "py311-none-any"),
        ("3.12", "1 package", "py312-none-any"),
    ] {
        let venv = context.temp_dir.child(format!(".venv-{python}"));
        context
            .venv()
            .args(["--python", python])
            .arg(venv.path())
            .assert()
            .success();
        context
            .pip_install()
            .arg("a")
            .arg("--python")
            .arg(venv.path())
            .arg("--index-url")
            .arg(index.index_url())
            .assert()
            .success()
            .stderr(predicates::str::contains(format!("Prepared {prepared}")));
        ChildPath::new(site_packages_path(venv.path(), &format!("python{python}")))
            .child("a-1.0.0.dist-info/WHEEL")
            .assert(predicates::str::contains(format!("Tag: {tag}\n")));
    }

    Ok(())
}
