use std::time::Duration;

use anyhow::Result;
use assert_fs::prelude::*;
use indoc::formatdoc;
use insta::assert_snapshot;

use uv_test::packse::PackseServer;
use uv_test::packse::scenario::Scenario;
use uv_test::uv_snapshot;

// All of the tests in this file should use `tool.uv.conflicts` in some way.
//
// They are split from `lock.rs` somewhat arbitrarily. Mostly because there are
// a lot of them, and `lock.rs` was growing large enough as it is.

/// Conflict discovery can provisionally visit a package that is later excluded after all
/// transitive extras have been activated. Its dependencies must be evaluated under the package's
/// reachability marker during that preliminary traversal.
#[test]
fn extra_conflict_discovery_respects_parent_reachability() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context.temp_dir.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.optional-dependencies]
        feature = ["x[foo]"]

        [tool.uv]
        conflicts = [[
            { package = "x", extra = "foo" },
            { package = "q", extra = "bar" },
        ]]
        "#,
    )?;

    // This is the lock shape produced when `parent`'s reachability marker is removed from its
    // outgoing edge: the edge is only valid in the context of `parent`, but conflict discovery
    // traverses `parent` before it knows that `x[foo]` makes the package unreachable.
    context.temp_dir.child("uv.lock").write_str(
        r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        conflicts = [[
            { package = "q", extra = "bar" },
            { package = "x", extra = "foo" },
        ]]

        [[package]]
        name = "parent"
        source = { virtual = "parent" }
        dependencies = [
            { name = "q", extra = ["bar"] },
        ]

        [[package]]
        name = "project"
        source = { virtual = "." }
        dependencies = [
            { name = "parent", marker = "extra != 'extra-1-x-foo'" },
        ]

        [package.optional-dependencies]
        feature = [
            { name = "x", extra = ["foo"] },
        ]

        [package.metadata]
        provides-extras = ["feature"]

        [[package]]
        name = "q"
        source = { virtual = "q" }

        [package.optional-dependencies]
        bar = []

        [[package]]
        name = "x"
        source = { virtual = "x" }

        [package.optional-dependencies]
        foo = []
        "#,
    )?;

    uv_snapshot!(context.filters(), context.sync()
        .arg("--extra")
        .arg("feature")
        .arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked in [TIME]
    ");

    Ok(())
}

/// This tests a "basic" case for specifying conflicting extras.
///
/// Namely, we check that 1) without declaring them conflicting,
/// resolution fails, 2) declaring them conflicting, resolution
/// succeeds, 3) install succeeds, 4) install fails when requesting two
/// or more extras that are declared to conflict with each other.
///
/// This test was inspired by:
/// <https://github.com/astral-sh/uv/issues/8024>
#[test]
fn extra_basic() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // First we test that resolving with two extras that have
    // conflicting dependencies fails.
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [project.optional-dependencies]
        extra1 = ["sortedcontainers==2.3.0"]
        extra2 = ["sortedcontainers==2.4.0"]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because project[extra2] depends on sortedcontainers==2.4.0 and project[extra1] depends on sortedcontainers==2.3.0, we can conclude that project[extra1] and project[extra2] are incompatible.
             And because your project requires project[extra1] and project[extra2], we can conclude that your project's requirements are unsatisfiable.
    ");

    // And now with the same extra configuration, we tell uv about
    // the conflicting extras, which forces it to resolve each in
    // their own fork.
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [tool.uv]
        conflicts = [
            [
              { extra = "extra1" },
              { extra = "extra2" },
            ],
        ]

        [project.optional-dependencies]
        extra1 = ["sortedcontainers==2.3.0"]
        extra2 = ["sortedcontainers==2.4.0"]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        conflicts = [[
            { package = "project", extra = "extra1" },
            { package = "project", extra = "extra2" },
        ]]

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }

        [package.optional-dependencies]
        extra1 = [
            { name = "sortedcontainers", version = "2.3.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        extra2 = [
            { name = "sortedcontainers", version = "2.4.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]

        [package.metadata]
        requires-dist = [
            { name = "sortedcontainers", marker = "extra == 'extra1'", specifier = "==2.3.0" },
            { name = "sortedcontainers", marker = "extra == 'extra2'", specifier = "==2.4.0" },
        ]
        provides-extras = ["extra1", "extra2"]

        [[package]]
        name = "sortedcontainers"
        version = "2.3.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sortedcontainers-2.3.0.tar.gz", hash = "sha256:c7bfd220eb6dcc29774e2e05298edc2c5d62fa1086cc6409c800c7f413afe2a6", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sortedcontainers-2.3.0-py3-none-any.whl", hash = "sha256:3228c480e84b2b7a504f28bd68f73b8cfde1d1fddf8a47783b19a0dbcc93ffcf", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "sortedcontainers"
        version = "2.4.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sortedcontainers-2.4.0.tar.gz", hash = "sha256:fb6015d312cfaf15c65e0aeadac28191546cc25e48d41b126ecc322d5b116be2", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sortedcontainers-2.4.0-py3-none-any.whl", hash = "sha256:e90c0f20bb5c630bbf840e4ccc94441cd55ae8048cf55fc42df715d28b67cb80", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Re-run with `--locked`.
    uv_snapshot!(context.filters(), context.lock().arg("--locked"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");

    // Install from the lockfile.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked in [TIME]
    ");
    // Another install, but with one of the extras enabled.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen").arg("--extra=extra1"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + sortedcontainers==2.3.0
    ");
    // Another install, but with the other extra enabled.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen").arg("--extra=extra2"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - sortedcontainers==2.3.0
     + sortedcontainers==2.4.0
    ");
    // And finally, installing both extras should error.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen").arg("--all-extras"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Extras `extra1` and `extra2` are incompatible with the declared conflicts: {`project[extra1]`, `project[extra2]`}
    ");
    // As should exporting them.
    uv_snapshot!(context.filters(), context.export().arg("--frozen").arg("--all-extras"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Extras `extra1` and `extra2` are incompatible with the declared conflicts: {`project[extra1]`, `project[extra2]`}
    ");

    Ok(())
}

/// Like `lock_conflicting_extra_basic`, but defines three conflicting
/// extras instead of two.
#[test]
fn extra_basic_three_extras() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // First we test that resolving with two extras that have
    // conflicting dependencies fails.
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [project.optional-dependencies]
        extra1 = ["sortedcontainers==2.2.0"]
        extra2 = ["sortedcontainers==2.3.0"]
        project3 = ["sortedcontainers==2.4.0"]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because project[extra2] depends on sortedcontainers==2.3.0 and project[extra1] depends on sortedcontainers==2.2.0, we can conclude that project[extra1] and project[extra2] are incompatible.
             And because your project requires project[extra1] and project[extra2], we can conclude that your project's requirements are unsatisfiable.
    ");

    // And now with the same extra configuration, we tell uv about
    // the conflicting extras, which forces it to resolve each in
    // their own fork.
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [tool.uv]
        conflicts = [
            [
              { extra = "extra1" },
              { extra = "extra2" },
              { extra = "project3" },
            ],
        ]

        [project.optional-dependencies]
        extra1 = ["sortedcontainers==2.2.0"]
        extra2 = ["sortedcontainers==2.3.0"]
        project3 = ["sortedcontainers==2.4.0"]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        conflicts = [[
            { package = "project", extra = "extra1" },
            { package = "project", extra = "extra2" },
            { package = "project", extra = "project3" },
        ]]

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }

        [package.optional-dependencies]
        extra1 = [
            { name = "sortedcontainers", version = "2.2.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        extra2 = [
            { name = "sortedcontainers", version = "2.3.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        project3 = [
            { name = "sortedcontainers", version = "2.4.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]

        [package.metadata]
        requires-dist = [
            { name = "sortedcontainers", marker = "extra == 'extra1'", specifier = "==2.2.0" },
            { name = "sortedcontainers", marker = "extra == 'extra2'", specifier = "==2.3.0" },
            { name = "sortedcontainers", marker = "extra == 'project3'", specifier = "==2.4.0" },
        ]
        provides-extras = ["extra1", "extra2", "project3"]

        [[package]]
        name = "sortedcontainers"
        version = "2.2.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sortedcontainers-2.2.0.tar.gz", hash = "sha256:611ebf42c3112a9a03c6656bb260a53ae8fcd97126cc616389e8b63c24042495", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sortedcontainers-2.2.0-py3-none-any.whl", hash = "sha256:8a5673c4e7caa879f2cf31951ddd26cbf13c4ff0cbc829eb93b96666eeb54ec3", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "sortedcontainers"
        version = "2.3.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sortedcontainers-2.3.0.tar.gz", hash = "sha256:c7bfd220eb6dcc29774e2e05298edc2c5d62fa1086cc6409c800c7f413afe2a6", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sortedcontainers-2.3.0-py3-none-any.whl", hash = "sha256:3228c480e84b2b7a504f28bd68f73b8cfde1d1fddf8a47783b19a0dbcc93ffcf", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "sortedcontainers"
        version = "2.4.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sortedcontainers-2.4.0.tar.gz", hash = "sha256:fb6015d312cfaf15c65e0aeadac28191546cc25e48d41b126ecc322d5b116be2", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sortedcontainers-2.4.0-py3-none-any.whl", hash = "sha256:e90c0f20bb5c630bbf840e4ccc94441cd55ae8048cf55fc42df715d28b67cb80", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    Ok(())
}

/// This tests that extras don't conflict with one another when they are in
/// distinct groups of extras.
#[test]
fn extra_multiple_not_conflicting1() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [tool.uv]
        conflicts = [
            [
              { extra = "extra1" },
              { extra = "extra2" },
            ],
            [
              { extra = "project3" },
              { extra = "project4" },
            ],
        ]

        [project.optional-dependencies]
        extra1 = []
        extra2 = []
        project3 = []
        project4 = []
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    ");

    // Install from the lockfile.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked in [TIME]
    ");
    // extra1/extra2 conflict!
    uv_snapshot!(
        context.filters(),
        context.sync().arg("--frozen").arg("--extra=extra1").arg("--extra=extra2"),
        @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Extras `extra1` and `extra2` are incompatible with the declared conflicts: {`project[extra1]`, `project[extra2]`}
    ");
    // project3/project4 conflict!
    uv_snapshot!(
        context.filters(),
        context.sync().arg("--frozen").arg("--extra=project3").arg("--extra=project4"),
        @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Extras `project3` and `project4` are incompatible with the declared conflicts: {`project[project3]`, `project[project4]`}
    ");
    // ... but extra1/project3 does not.
    uv_snapshot!(
        context.filters(),
        context.sync().arg("--frozen").arg("--extra=extra1").arg("--extra=project3"),
        @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked in [TIME]
    ");
    // ... and neither does extra2/project3.
    uv_snapshot!(
        context.filters(),
        context.sync().arg("--frozen").arg("--extra=extra2").arg("--extra=project3"),
        @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked in [TIME]
    ");
    // And similarly, with project 4.
    uv_snapshot!(
        context.filters(),
        context.sync().arg("--frozen").arg("--extra=extra1").arg("--extra=project4"),
        @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked in [TIME]
    ");
    // ... and neither does extra2/project3.
    uv_snapshot!(
        context.filters(),
        context.sync().arg("--frozen").arg("--extra=extra2").arg("--extra=project4"),
        @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked in [TIME]
    ");

    Ok(())
}

/// This tests that if the user has conflicting extras, but puts them in two
/// distinct groups of extras, then resolution still fails. (Because the only
/// way to resolve them in different forks is to define the extras as directly
/// conflicting.)
#[test]
fn extra_multiple_not_conflicting2() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [project.optional-dependencies]
        extra1 = ["sortedcontainers==2.3.0"]
        extra2 = ["sortedcontainers==2.4.0"]
        project3 = ["sortedcontainers==2.3.0"]
        project4 = ["sortedcontainers==2.4.0"]
        "#,
    )?;

    // Fails, as expected.
    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because project[extra2] depends on sortedcontainers==2.4.0 and project[extra1] depends on sortedcontainers==2.3.0, we can conclude that project[extra1] and project[extra2] are incompatible.
             And because your project requires project[extra1] and project[extra2], we can conclude that your project's requirements are unsatisfiable.
    ");

    // If we define extra1/extra2 as conflicting and project3/project4
    // as conflicting, that still isn't enough! That's because extra1
    // conflicts with project4 and extra2 conflicts with project3.
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [tool.uv]
        conflicts = [
            [
              { extra = "extra1" },
              { extra = "extra2" },
            ],
            [
              { extra = "project3" },
              { extra = "project4" },
            ],
        ]

        [project.optional-dependencies]
        extra1 = ["sortedcontainers==2.3.0"]
        extra2 = ["sortedcontainers==2.4.0"]
        project3 = ["sortedcontainers==2.3.0"]
        project4 = ["sortedcontainers==2.4.0"]
        "#,
    )?;
    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies for split (included: project[extra2], project[project3]; excluded: project[extra1], project[project4])
      cause: Because project[project3] depends on sortedcontainers==2.3.0 and project[extra2] depends on sortedcontainers==2.4.0, we can conclude that project[extra2] and project[project3] are incompatible.
             And because your project requires project[extra2] and project[project3], we can conclude that your project's requirements are unsatisfiable.
    ");

    // One could try to declare all pairs of conflicting extras as
    // conflicting, but this doesn't quite work either. For example,
    // the first group of conflicting extra, extra1/extra2,
    // specifically allows project4 to be co-mingled with extra1 (and
    // similarly, project3 with extra2), which are conflicting.
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [tool.uv]
        conflicts = [
            [
              { extra = "extra1" },
              { extra = "extra2" },
            ],
            [
              { extra = "project3" },
              { extra = "project4" },
            ],
            [
              { extra = "extra1" },
              { extra = "project4" },
            ],
            [
              { extra = "extra2" },
              { extra = "project3" },
            ],
        ]

        [project.optional-dependencies]
        extra1 = ["sortedcontainers==2.3.0"]
        extra2 = ["sortedcontainers==2.4.0"]
        project3 = ["sortedcontainers==2.3.0"]
        project4 = ["sortedcontainers==2.4.0"]
        "#,
    )?;
    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");

    // We can also fix this by just putting them all in one big
    // group, even though extra1/project3 don't conflict and
    // extra2/project4 don't conflict.
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [tool.uv]
        conflicts = [
            [
              { extra = "extra1" },
              { extra = "extra2" },
              { extra = "project3" },
              { extra = "project4" },
            ],
        ]

        [project.optional-dependencies]
        extra1 = ["sortedcontainers==2.3.0"]
        extra2 = ["sortedcontainers==2.4.0"]
        project3 = ["sortedcontainers==2.3.0"]
        project4 = ["sortedcontainers==2.4.0"]
        "#,
    )?;
    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");

    Ok(())
}

/// This tests that we handle two independent sets of conflicting
/// extras correctly.
#[test]
fn extra_multiple_independent() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // If we don't declare any conflicting extras, then resolution
    // will of course fail.
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [project.optional-dependencies]
        extra1 = ["sortedcontainers==2.3.0"]
        extra2 = ["sortedcontainers==2.4.0"]
        project3 = ["anyio==4.1.0"]
        project4 = ["anyio==4.2.0"]
        "#,
    )?;
    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because project[extra2] depends on sortedcontainers==2.4.0 and project[extra1] depends on sortedcontainers==2.3.0, we can conclude that project[extra1] and project[extra2] are incompatible.
             And because your project requires project[extra1] and project[extra2], we can conclude that your project's requirements are unsatisfiable.
    ");

    // OK, responding to the error, we declare our anyio extras
    // as conflicting. But now we should see sortedcontainers as
    // conflicting.
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [tool.uv]
        conflicts = [
            [
              { extra = "project3" },
              { extra = "project4" },
            ],
        ]

        [project.optional-dependencies]
        extra1 = ["sortedcontainers==2.3.0"]
        extra2 = ["sortedcontainers==2.4.0"]
        project3 = ["anyio==4.1.0"]
        project4 = ["anyio==4.2.0"]
        "#,
    )?;
    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies for split (included: project[project4]; excluded: project[project3])
      cause: Because project[extra2] depends on sortedcontainers==2.4.0 and project[extra1] depends on sortedcontainers==2.3.0, we can conclude that project[extra1] and project[extra2] are incompatible.
             And because your project requires project[extra1] and project[extra2], we can conclude that your project's requirements are unsatisfiable.
    ");

    // Once we declare ALL our conflicting extras, resolution succeeds.
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [tool.uv]
        conflicts = [
            [
              { extra = "extra1" },
              { extra = "extra2" },
            ],
            [
              { extra = "project3" },
              { extra = "project4" },
            ],
        ]

        [project.optional-dependencies]
        extra1 = ["sortedcontainers==2.3.0"]
        extra2 = ["sortedcontainers==2.4.0"]
        project3 = ["anyio==4.1.0"]
        project4 = ["anyio==4.2.0"]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 7 packages in [TIME]
    ");

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        conflicts = [[
            { package = "project", extra = "extra1" },
            { package = "project", extra = "extra2" },
        ], [
            { package = "project", extra = "project3" },
            { package = "project", extra = "project4" },
        ]]

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "anyio"
        version = "4.1.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "idna" },
            { name = "sniffio" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/anyio-4.1.0.tar.gz", hash = "sha256:33450380e014961bf8ac6d7141ca53ebd9d228309b5314c7cdaf1cb63baaa1a8", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/anyio-4.1.0-py3-none-any.whl", hash = "sha256:9aa4b1ae9af480bc7b87ea2a42200e869488b397d5822509e9da70ecdc3572ad", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "anyio"
        version = "4.2.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "idna" },
            { name = "sniffio" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/anyio-4.2.0.tar.gz", hash = "sha256:80e775b23b8b841a7b662d27a5c363acebf4f765ebc5739ca0b8fdfbc48d0b62", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/anyio-4.2.0-py3-none-any.whl", hash = "sha256:0bd2e4849488564f98596bc752412e3c16b68fed34b535e415736cd874c4b157", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.6"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.6.tar.gz", hash = "sha256:9aae8f72192b28db0d56fcef130afe490d1538a8d1bf1700e6d219521421525f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.6-py3-none-any.whl", hash = "sha256:e80025850eafa8760055fd6f2f6e83f84bf13d4a844fe81abb2b499e3a3e8af0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }

        [package.optional-dependencies]
        extra1 = [
            { name = "sortedcontainers", version = "2.3.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        extra2 = [
            { name = "sortedcontainers", version = "2.4.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        project3 = [
            { name = "anyio", version = "4.1.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        project4 = [
            { name = "anyio", version = "4.2.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]

        [package.metadata]
        requires-dist = [
            { name = "anyio", marker = "extra == 'project3'", specifier = "==4.1.0" },
            { name = "anyio", marker = "extra == 'project4'", specifier = "==4.2.0" },
            { name = "sortedcontainers", marker = "extra == 'extra1'", specifier = "==2.3.0" },
            { name = "sortedcontainers", marker = "extra == 'extra2'", specifier = "==2.4.0" },
        ]
        provides-extras = ["extra1", "extra2", "project3", "project4"]

        [[package]]
        name = "sniffio"
        version = "1.3.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sniffio-1.3.1.tar.gz", hash = "sha256:ce520d2eb3c2be02f0c148dab5ba304e8705e1c7e7b4bec8a9146c464a597a6a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sniffio-1.3.1-py3-none-any.whl", hash = "sha256:2743fa2a853c508a2310882c0b4104631e0b0fcb855e00a912b7e3f27e6b3f05", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "sortedcontainers"
        version = "2.3.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sortedcontainers-2.3.0.tar.gz", hash = "sha256:c7bfd220eb6dcc29774e2e05298edc2c5d62fa1086cc6409c800c7f413afe2a6", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sortedcontainers-2.3.0-py3-none-any.whl", hash = "sha256:3228c480e84b2b7a504f28bd68f73b8cfde1d1fddf8a47783b19a0dbcc93ffcf", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "sortedcontainers"
        version = "2.4.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sortedcontainers-2.4.0.tar.gz", hash = "sha256:fb6015d312cfaf15c65e0aeadac28191546cc25e48d41b126ecc322d5b116be2", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sortedcontainers-2.4.0-py3-none-any.whl", hash = "sha256:e90c0f20bb5c630bbf840e4ccc94441cd55ae8048cf55fc42df715d28b67cb80", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    Ok(())
}

#[test]
fn extra_config_change_ignore_lockfile() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [tool.uv]
        conflicts = [
            [
              { extra = "extra1" },
              { extra = "extra2" },
            ],
        ]

        [project.optional-dependencies]
        extra1 = ["sortedcontainers==2.3.0"]
        extra2 = ["sortedcontainers==2.4.0"]
        "#,
    )?;
    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        conflicts = [[
            { package = "project", extra = "extra1" },
            { package = "project", extra = "extra2" },
        ]]

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }

        [package.optional-dependencies]
        extra1 = [
            { name = "sortedcontainers", version = "2.3.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        extra2 = [
            { name = "sortedcontainers", version = "2.4.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]

        [package.metadata]
        requires-dist = [
            { name = "sortedcontainers", marker = "extra == 'extra1'", specifier = "==2.3.0" },
            { name = "sortedcontainers", marker = "extra == 'extra2'", specifier = "==2.4.0" },
        ]
        provides-extras = ["extra1", "extra2"]

        [[package]]
        name = "sortedcontainers"
        version = "2.3.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sortedcontainers-2.3.0.tar.gz", hash = "sha256:c7bfd220eb6dcc29774e2e05298edc2c5d62fa1086cc6409c800c7f413afe2a6", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sortedcontainers-2.3.0-py3-none-any.whl", hash = "sha256:3228c480e84b2b7a504f28bd68f73b8cfde1d1fddf8a47783b19a0dbcc93ffcf", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "sortedcontainers"
        version = "2.4.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sortedcontainers-2.4.0.tar.gz", hash = "sha256:fb6015d312cfaf15c65e0aeadac28191546cc25e48d41b126ecc322d5b116be2", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sortedcontainers-2.4.0-py3-none-any.whl", hash = "sha256:e90c0f20bb5c630bbf840e4ccc94441cd55ae8048cf55fc42df715d28b67cb80", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Re-run with `--locked` to check it's okay.
    uv_snapshot!(context.filters(), context.lock().arg("--locked"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");

    // Now get rid of the conflicting group config, and check that `--locked`
    // fails.
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [project.optional-dependencies]
        extra1 = ["sortedcontainers==2.3.0"]
        extra2 = ["sortedcontainers==2.4.0"]
        "#,
    )?;
    // Re-run with `--locked`, which should now fail because of
    // the conflicting group config removal.
    uv_snapshot!(context.filters(), context.lock().arg("--locked"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because project[extra2] depends on sortedcontainers==2.4.0 and project[extra1] depends on sortedcontainers==2.3.0, we can conclude that project[extra1] and project[extra2] are incompatible.
             And because your project requires project[extra1] and project[extra2], we can conclude that your project's requirements are unsatisfiable.
    ");

    Ok(())
}

/// This tests that we report an error when a requirement unconditionally
/// enables a conflicting extra.
#[test]
fn extra_unconditional() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let root_pyproject_toml = context.temp_dir.child("pyproject.toml");
    root_pyproject_toml.write_str(
        r#"
        [project]
        name = "dummy"
        version = "0.1.0"
        requires-python = "==3.12.*"
        dependencies = [
          "proxy1[extra1,extra2]"
        ]

        [tool.uv.workspace]
        members = ["proxy1"]

        [tool.uv.sources]
        proxy1 = { workspace = true }
        "#,
    )?;

    let proxy1_pyproject_toml = context.temp_dir.child("proxy1").child("pyproject.toml");
    proxy1_pyproject_toml.write_str(
        r#"
        [project]
        name = "proxy1"
        version = "0.1.0"
        requires-python = "==3.12.*"
        dependencies = []

        [project.optional-dependencies]
        extra1 = ["anyio==4.1.0"]
        extra2 = ["anyio==4.2.0"]

        [tool.uv]
        conflicts = [
          [
            { extra = "extra1" },
            { extra = "extra2" },
          ],
        ]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    ");

    // This should error since we're enabling two conflicting extras.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Found conflicting extras `proxy1[extra1]` and `proxy1[extra2]` enabled simultaneously
    ");

    root_pyproject_toml.write_str(
        r#"
        [project]
        name = "dummy"
        version = "0.1.0"
        requires-python = "==3.12.*"
        dependencies = [
          "proxy1[extra1]"
        ]

        [tool.uv.workspace]
        members = ["proxy1"]

        [tool.uv.sources]
        proxy1 = { workspace = true }
        "#,
    )?;
    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    ");
    // This is fine because we are only enabling one
    // extra, and thus, there is no conflict.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==4.1.0
     + idna==3.6
     + proxy1==0.1.0 (from file://[TEMP_DIR]/proxy1)
     + sniffio==1.3.1
    ");

    // And same thing for the other extra.
    root_pyproject_toml.write_str(
        r#"
        [project]
        name = "dummy"
        version = "0.1.0"
        requires-python = "==3.12.*"
        dependencies = [
          "proxy1[extra2]"
        ]

        [tool.uv.workspace]
        members = ["proxy1"]

        [tool.uv.sources]
        proxy1 = { workspace = true }
        "#,
    )?;
    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    ");
    // This is fine because we are only enabling one
    // extra, and thus, there is no conflict.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - anyio==4.1.0
     + anyio==4.2.0
    ");

    Ok(())
}

/// A regression tests for unconditionally installing an extra that is
/// marked as conflicting. Previously, the `proxy1[extra1]` dependency
/// would be completely ignored here.
#[test]
fn extra_unconditional_non_conflicting() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let root_pyproject_toml = context.temp_dir.child("pyproject.toml");
    root_pyproject_toml.write_str(
        r#"
        [project]
        name = "dummy"
        version = "0.1.0"
        requires-python = "==3.12.*"
        dependencies = [
          "proxy1[extra1]"
        ]

        [tool.uv.workspace]
        members = ["proxy1"]

        [tool.uv.sources]
        proxy1 = { workspace = true }
        "#,
    )?;

    let proxy1_pyproject_toml = context.temp_dir.child("proxy1").child("pyproject.toml");
    proxy1_pyproject_toml.write_str(
        r#"
        [project]
        name = "proxy1"
        version = "0.1.0"
        requires-python = "==3.12.*"
        dependencies = []

        [project.optional-dependencies]
        extra1 = ["anyio==4.1.0"]
        extra2 = []
        extra3 = []

        [tool.uv]
        conflicts = [
          [
            { extra = "extra1" },
            { extra = "extra3" },
          ],
        ]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    ");

    // This *should* install `anyio==4.1.0`, but when this
    // test was initially written, it didn't. This was because
    // `uv sync` wasn't correctly propagating extras in a way
    // that would satisfy the conflict markers that got added
    // to the `proxy1[extra1]` dependency.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==4.1.0
     + idna==3.6
     + proxy1==0.1.0 (from file://[TEMP_DIR]/proxy1)
     + sniffio==1.3.1
    ");

    Ok(())
}

#[test]
fn extra_unconditional_in_optional() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let root_pyproject_toml = context.temp_dir.child("pyproject.toml");
    root_pyproject_toml.write_str(
        r#"
        [project]
        name = "foo"
        version = "0.1.0"
        description = "Add your description here"
        readme = "README.md"
        requires-python = ">=3.10.0"
        dependencies = []

        [tool.uv.workspace]
        members = ["proxy1"]

        [tool.uv.sources]
        proxy1 = { workspace = true }

        [project.optional-dependencies]
        x1 = ["proxy1[nested-x1]"]
        x2 = ["proxy1[nested-x2]"]
        "#,
    )?;

    let proxy1_pyproject_toml = context.temp_dir.child("proxy1").child("pyproject.toml");
    proxy1_pyproject_toml.write_str(
        r#"
        [project]
        name = "proxy1"
        version = "0.1.0"
        requires-python = ">=3.10.0"
        dependencies = []

        [project.optional-dependencies]
        nested-x1 = ["sortedcontainers==2.3.0"]
        nested-x2 = ["sortedcontainers==2.4.0"]

        [tool.uv]
        conflicts = [
          [
            { extra = "nested-x1" },
            { extra = "nested-x2" },
          ],
        ]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");

    // This shouldn't install anything.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked in [TIME]
    ");

    // This should install `sortedcontainers==2.3.0`.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen").arg("--extra=x1"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + proxy1==0.1.0 (from file://[TEMP_DIR]/proxy1)
     + sortedcontainers==2.3.0
    ");

    // This should install `sortedcontainers==2.4.0`.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen").arg("--extra=x2"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - sortedcontainers==2.3.0
     + sortedcontainers==2.4.0
    ");

    // This should error!
    uv_snapshot!(
        context.filters(),
        context.sync().arg("--frozen").arg("--extra=x1").arg("--extra=x2"),
        @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Found conflicting extras `proxy1[nested-x1]` and `proxy1[nested-x2]` enabled simultaneously
    ");

    Ok(())
}

#[test]
fn extra_unconditional_non_local_conflict() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let root_pyproject_toml = context.temp_dir.child("pyproject.toml");
    root_pyproject_toml.write_str(
        r#"
        [project]
        name = "foo"
        version = "0.1.0"
        description = "Add your description here"
        readme = "README.md"
        requires-python = ">=3.10.0"
        dependencies = ["a", "b"]

        [tool.uv.workspace]
        members = ["a", "b", "c"]

        [tool.uv.sources]
        a = { workspace = true }
        b = { workspace = true }
        c = { workspace = true }
        "#,
    )?;

    let a_pyproject_toml = context.temp_dir.child("a").child("pyproject.toml");
    a_pyproject_toml.write_str(
        r#"
        [project]
        name = "a"
        version = "0.1.0"
        requires-python = ">=3.10.0"
        dependencies = ["c[x1]"]

        [tool.uv.sources]
        c = { workspace = true }
        "#,
    )?;

    let b_pyproject_toml = context.temp_dir.child("b").child("pyproject.toml");
    b_pyproject_toml.write_str(
        r#"
        [project]
        name = "b"
        version = "0.1.0"
        requires-python = ">=3.10.0"
        dependencies = ["c[x2]"]

        [tool.uv.sources]
        c = { workspace = true }
        "#,
    )?;

    let c_pyproject_toml = context.temp_dir.child("c").child("pyproject.toml");
    c_pyproject_toml.write_str(
        r#"
        [project]
        name = "c"
        version = "0.1.0"
        requires-python = ">=3.10.0"
        dependencies = []

        [project.optional-dependencies]
        x1 = ["sortedcontainers==2.3.0"]
        x2 = ["sortedcontainers==2.4.0"]

        [tool.uv]
        conflicts = [
          [
            { extra = "x1" },
            { extra = "x2" },
          ],
        ]
        "#,
    )?;

    // Regrettably, this produces a lock file, and it is one
    // that can never be installed! Namely, because two different
    // conflicting extras are enabled unconditionally in all
    // configurations.
    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    ");

    // This should fail. If it doesn't and we generated a lock
    // file above, then this will likely result in the installation
    // of two different versions of the same package.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Found conflicting extras `c[x1]` and `c[x2]` enabled simultaneously
    ");

    Ok(())
}

/// This tests how we deal with mutually conflicting extras that span multiple
/// packages in a workspace.
#[test]
fn extra_nested_across_workspace() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let root_pyproject_toml = context.temp_dir.child("pyproject.toml");
    root_pyproject_toml.write_str(
        r#"
        [project]
        name = "dummy"
        version = "0.1.0"
        requires-python = "==3.12.*"

        [project.optional-dependencies]
        extra1 = [
          "proxy1[extra1]",
        ]
        extra2 = [
          "proxy1[extra2]"
        ]

        [tool.uv.sources]
        proxy1 = { path = "./proxy1" }
        dummysub =  { workspace = true }

        [tool.uv.workspace]
        members = ["dummysub"]

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"

        [tool.uv]
        conflicts = [
          [
            { extra = "extra1" },
            { extra = "extra2" },
          ],
        ]
        "#,
    )?;

    let sub_pyproject_toml = context.temp_dir.child("dummysub").child("pyproject.toml");
    sub_pyproject_toml.write_str(
        r#"
        [project]
        name = "dummysub"
        version = "0.1.0"
        requires-python = "==3.12.*"

        [project.optional-dependencies]
        extra1 = [
          "proxy1[extra1]",
        ]
        extra2 = [
          "proxy1[extra2]"
        ]

        [tool.uv.sources]
        proxy1 = { path = "../proxy1" }

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"

        [tool.uv]
        conflicts = [
          [
            { extra = "extra1" },
            { extra = "extra2" },
          ],
        ]
        "#,
    )?;

    let proxy1_pyproject_toml = context.temp_dir.child("proxy1").child("pyproject.toml");
    proxy1_pyproject_toml.write_str(
        r#"
        [project]
        name = "proxy1"
        version = "0.1.0"
        requires-python = "==3.12.*"
        dependencies = []

        [project.optional-dependencies]
        extra1 = ["anyio==4.1.0"]
        extra2 = ["anyio==4.2.0"]

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
        "#,
    )?;

    // In the scheme above, we declare that `dummy[extra1]` conflicts
    // with `dummy[extra2]`, and that `dummysub[extra1]` conflicts
    // with `dummysub[extra2]`. But we don't account for the fact that
    // `dummy[extra1]` conflicts with `dummysub[extra2]` and that
    // `dummy[extra2]` conflicts with `dummysub[extra1]`. So we end
    // up with a resolution failure.
    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies for split (included: dummy[extra2], dummysub[extra1]; excluded: dummy[extra1], dummysub[extra2])
      cause: Because dummy[extra2] depends on proxy1[extra2] and only proxy1[extra2]==0.1.0 is available, we can conclude that dummy[extra2] depends on proxy1[extra2]==0.1.0. (1)

             Because proxy1[extra1]==0.1.0 depends on anyio==4.1.0 and proxy1[extra2]==0.1.0 depends on anyio==4.2.0, we can conclude that proxy1[extra1]==0.1.0 and proxy1[extra2]==0.1.0 are incompatible.
             And because we know from (1) that dummy[extra2] depends on proxy1[extra2]==0.1.0, we can conclude that dummy[extra2] and proxy1[extra1]==0.1.0 are incompatible.
             And because only proxy1[extra1]==0.1.0 is available and dummysub[extra1] depends on proxy1[extra1], we can conclude that dummysub[extra1] and dummy[extra2] are incompatible.
             And because your workspace requires dummy[extra2] and dummysub[extra1], we can conclude that your workspace's requirements are unsatisfiable.
    ");

    // Now let's write out the full set of conflicts, taking
    // advantage of the optional `package` key.
    root_pyproject_toml.write_str(
        r#"
        [project]
        name = "dummy"
        version = "0.1.0"
        requires-python = "==3.12.*"

        [project.optional-dependencies]
        extra1 = [
          "proxy1[extra1]",
        ]
        extra2 = [
          "proxy1[extra2]"
        ]

        [tool.uv.sources]
        proxy1 = { path = "./proxy1" }
        dummysub =  { workspace = true }

        [tool.uv.workspace]
        members = ["dummysub"]

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"

        [tool.uv]
        conflicts = [
          [
            { extra = "extra1" },
            { extra = "extra2" },
          ],
          [
            { package = "dummysub", extra = "extra1" },
            { package = "dummysub", extra = "extra2" },
          ],
          [
            { extra = "extra1" },
            { package = "dummysub", extra = "extra2" },
          ],
          [
            { package = "dummysub", extra = "extra1" },
            { extra = "extra2" },
          ],
        ]
        "#,
    )?;
    // And we can remove the conflicts from `dummysub` since
    // there specified in `dummy`.
    sub_pyproject_toml.write_str(
        r#"
        [project]
        name = "dummysub"
        version = "0.1.0"
        requires-python = "==3.12.*"

        [project.optional-dependencies]
        extra1 = [
          "proxy1[extra1]",
        ]
        extra2 = [
          "proxy1[extra2]"
        ]

        [tool.uv.sources]
        proxy1 = { path = "../proxy1" }

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
        "#,
    )?;
    // And now things should work.
    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 7 packages in [TIME]
    ");

    Ok(())
}

/// The project declares conflicting extras, but one of the extras directly depends on the other.
#[test]
fn extra_depends_on_conflicting_extra() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "example"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.optional-dependencies]
        foo = ["sortedcontainers==2.3.0", "example[bar]"]
        bar = ["sortedcontainers==2.4.0"]

        [tool.uv]
        conflicts = [
          [
            { extra = "foo" },
            { extra = "bar" },
          ],
        ]

        [build-system]
        requires = ["setuptools>=42"]
        build-backend = "setuptools.build_meta"

        [tool.setuptools.packages.find]
        include = ["example"]
        "#,
    )?;

    // This should fail to resolve, because the extras are always required together and
    // `example[foo]` is unusable.
    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies for split (included: example[foo]; excluded: example[bar])
      cause: Because example[foo] depends on sortedcontainers==2.3.0 and sortedcontainers==2.4.0, we can conclude that example[foo]'s requirements are unsatisfiable.
             And because your project requires example[foo], we can conclude that your project's requirements are unsatisfiable.
    ");

    Ok(())
}

/// Like [`extra_depends_on_conflicting_extra`], but the conflict between the extras is mediated by
/// another package.
#[test]
fn extra_depends_on_conflicting_extra_transitive() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "example"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.optional-dependencies]
        foo = ["sortedcontainers==2.3.0", "indirection"]
        bar = ["sortedcontainers==2.4.0"]

        [tool.uv]
        conflicts = [
          [
            { extra = "foo" },
            { extra = "bar" },
          ],
        ]

        [tool.uv.sources]
        indirection = { workspace = true }

        [tool.uv.workspace]
        members = ["indirection"]

        [build-system]
        requires = ["setuptools>=42"]
        build-backend = "setuptools.build_meta"

        [tool.setuptools.packages.find]
        include = ["example"]
        "#,
    )?;

    // Create the indirection subproject
    let subproject_dir = context.temp_dir.child("indirection");
    subproject_dir.create_dir_all()?;

    let sub_pyproject_toml = subproject_dir.child("pyproject.toml");
    sub_pyproject_toml.write_str(
        r#"
        [project]
        name = "indirection"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["example[bar]"]

        [tool.uv.sources]
        example = { workspace = true }

        [build-system]
        requires = ["setuptools>=42"]
        build-backend = "setuptools.build_meta"
        "#,
    )?;

    // This succeeds, but probably shouldn't. There's an unconditional conflict in `example[foo]
    // -> indirection[bar] -> example[bar]`, which means `example[foo]` can never be used.
    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        conflicts = [[
            { package = "example", extra = "bar" },
            { package = "example", extra = "foo" },
        ]]

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [manifest]
        members = [
            "example",
            "indirection",
        ]

        [[package]]
        name = "example"
        version = "0.1.0"
        source = { editable = "." }

        [package.optional-dependencies]
        bar = [
            { name = "sortedcontainers", version = "2.4.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        foo = [
            { name = "indirection" },
            { name = "sortedcontainers", version = "2.3.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]

        [package.metadata]
        requires-dist = [
            { name = "indirection", marker = "extra == 'foo'", editable = "indirection" },
            { name = "sortedcontainers", marker = "extra == 'bar'", specifier = "==2.4.0" },
            { name = "sortedcontainers", marker = "extra == 'foo'", specifier = "==2.3.0" },
        ]
        provides-extras = ["foo", "bar"]

        [[package]]
        name = "indirection"
        version = "0.1.0"
        source = { editable = "indirection" }
        dependencies = [
            { name = "example" },
            { name = "example", extra = ["bar"], marker = "extra == 'extra-7-example-bar'" },
        ]

        [package.metadata]
        requires-dist = [{ name = "example", extras = ["bar"], editable = "." }]

        [[package]]
        name = "sortedcontainers"
        version = "2.3.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sortedcontainers-2.3.0.tar.gz", hash = "sha256:c7bfd220eb6dcc29774e2e05298edc2c5d62fa1086cc6409c800c7f413afe2a6", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sortedcontainers-2.3.0-py3-none-any.whl", hash = "sha256:3228c480e84b2b7a504f28bd68f73b8cfde1d1fddf8a47783b19a0dbcc93ffcf", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "sortedcontainers"
        version = "2.4.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sortedcontainers-2.4.0.tar.gz", hash = "sha256:fb6015d312cfaf15c65e0aeadac28191546cc25e48d41b126ecc322d5b116be2", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sortedcontainers-2.4.0-py3-none-any.whl", hash = "sha256:e90c0f20bb5c630bbf840e4ccc94441cd55ae8048cf55fc42df715d28b67cb80", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Install from the lockfile
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + example==0.1.0 (from file://[TEMP_DIR]/)
    ");

    // Install with `foo`
    uv_snapshot!(context.filters(), context.sync().arg("--frozen").arg("--extra").arg("foo"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Found conflicting extras `example[bar]` and `example[foo]` enabled simultaneously
    ");

    // Install the child package
    uv_snapshot!(context.filters(), context.sync().arg("--frozen").arg("--package").arg("indirection"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + indirection==0.1.0 (from file://[TEMP_DIR]/indirection)
     + sortedcontainers==2.4.0
    ");

    Ok(())
}

/// This tests a "basic" case for specifying conflicting groups.
#[test]
fn group_basic() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // First we test that resolving with two groups that have
    // conflicting dependencies fails.
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        description = "Add your description here"
        requires-python = ">=3.12"

        [dependency-groups]
        group1 = ["sortedcontainers==2.3.0"]
        group2 = ["sortedcontainers==2.4.0"]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because project:group2 depends on sortedcontainers==2.4.0 and project:group1 depends on sortedcontainers==2.3.0, we can conclude that project:group1 and project:group2 are incompatible.
             And because your project requires project:group1 and project:group2, we can conclude that your project's requirements are unsatisfiable.
    ");

    // And now with the same group configuration, we tell uv about
    // the conflicting groups, which forces it to resolve each in
    // their own fork.
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        description = "Add your description here"
        requires-python = ">=3.12"

        [tool.uv]
        conflicts = [
            [
              { group = "group1" },
              { group = "group2" },
            ],
        ]

        [dependency-groups]
        group1 = ["sortedcontainers==2.3.0"]
        group2 = ["sortedcontainers==2.4.0"]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        conflicts = [[
            { package = "project", group = "group1" },
            { package = "project", group = "group2" },
        ]]

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }

        [package.dev-dependencies]
        group1 = [
            { name = "sortedcontainers", version = "2.3.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        group2 = [
            { name = "sortedcontainers", version = "2.4.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]

        [package.metadata]

        [package.metadata.requires-dev]
        group1 = [{ name = "sortedcontainers", specifier = "==2.3.0" }]
        group2 = [{ name = "sortedcontainers", specifier = "==2.4.0" }]

        [[package]]
        name = "sortedcontainers"
        version = "2.3.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sortedcontainers-2.3.0.tar.gz", hash = "sha256:c7bfd220eb6dcc29774e2e05298edc2c5d62fa1086cc6409c800c7f413afe2a6", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sortedcontainers-2.3.0-py3-none-any.whl", hash = "sha256:3228c480e84b2b7a504f28bd68f73b8cfde1d1fddf8a47783b19a0dbcc93ffcf", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "sortedcontainers"
        version = "2.4.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sortedcontainers-2.4.0.tar.gz", hash = "sha256:fb6015d312cfaf15c65e0aeadac28191546cc25e48d41b126ecc322d5b116be2", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sortedcontainers-2.4.0-py3-none-any.whl", hash = "sha256:e90c0f20bb5c630bbf840e4ccc94441cd55ae8048cf55fc42df715d28b67cb80", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Re-run with `--locked`.
    uv_snapshot!(context.filters(), context.lock().arg("--locked"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");

    // Install from the lockfile.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked in [TIME]
    ");
    // Another install, but with one of the groups enabled.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen").arg("--group=group1"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + sortedcontainers==2.3.0
    ");
    // Another install, but with the other group enabled.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen").arg("--group=group2"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - sortedcontainers==2.3.0
     + sortedcontainers==2.4.0
    ");
    // And finally, installing both groups should error.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen").arg("--group=group1").arg("--group=group2"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Groups `group1` and `group2` are incompatible with the conflicts: {`project:group1`, `project:group2`}
    ");

    Ok(())
}

/// This tests a case of specifying conflicting groups, where some of the conflicts are enabled by
/// default.
#[test]
fn group_default() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // Tell uv about the conflicting groups, which forces it to resolve each in
    // their own fork.
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        description = "Add your description here"
        requires-python = ">=3.12"

        [tool.uv]
        conflicts = [
            [
              { group = "group1" },
              { group = "group2" },
            ],
        ]
        default-groups = ["group1"]

        [dependency-groups]
        group1 = ["sortedcontainers==2.3.0"]
        group2 = ["sortedcontainers==2.4.0"]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        conflicts = [[
            { package = "project", group = "group1" },
            { package = "project", group = "group2" },
        ]]

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }

        [package.dev-dependencies]
        group1 = [
            { name = "sortedcontainers", version = "2.3.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        group2 = [
            { name = "sortedcontainers", version = "2.4.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]

        [package.metadata]

        [package.metadata.requires-dev]
        group1 = [{ name = "sortedcontainers", specifier = "==2.3.0" }]
        group2 = [{ name = "sortedcontainers", specifier = "==2.4.0" }]

        [[package]]
        name = "sortedcontainers"
        version = "2.3.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sortedcontainers-2.3.0.tar.gz", hash = "sha256:c7bfd220eb6dcc29774e2e05298edc2c5d62fa1086cc6409c800c7f413afe2a6", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sortedcontainers-2.3.0-py3-none-any.whl", hash = "sha256:3228c480e84b2b7a504f28bd68f73b8cfde1d1fddf8a47783b19a0dbcc93ffcf", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "sortedcontainers"
        version = "2.4.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sortedcontainers-2.4.0.tar.gz", hash = "sha256:fb6015d312cfaf15c65e0aeadac28191546cc25e48d41b126ecc322d5b116be2", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sortedcontainers-2.4.0-py3-none-any.whl", hash = "sha256:e90c0f20bb5c630bbf840e4ccc94441cd55ae8048cf55fc42df715d28b67cb80", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Re-run with `--locked`.
    uv_snapshot!(context.filters(), context.lock().arg("--locked"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");

    // Install from the lockfile, which should include the `extra1` group by default.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + sortedcontainers==2.3.0
    ");

    // Another install, but with one of the groups enabled.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen").arg("--group=group1"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 1 package in [TIME]
    ");

    // Another install, but with the other group enabled. This should error, since `group1` is
    // enabled by default.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen").arg("--group=group2"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Groups `group1` (enabled by default) and `group2` are incompatible with the conflicts: {`project:group1`, `project:group2`}
    ");

    // If the group is explicitly requested, we should still fail, but shouldn't mark it as
    // "enabled by default".
    uv_snapshot!(context.filters(), context.sync().arg("--frozen").arg("--group=group1").arg("--group=group2"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Groups `group1` and `group2` are incompatible with the conflicts: {`project:group1`, `project:group2`}
    ");

    // If we install via `--all-groups`, we should also avoid marking the group as "enabled by
    // default".
    uv_snapshot!(context.filters(), context.sync().arg("--frozen").arg("--all-groups"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Groups `group1` and `group2` are incompatible with the conflicts: {`project:group1`, `project:group2`}
    ");

    // Disabling the default group should succeed.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen").arg("--no-group=group1").arg("--group=group2"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - sortedcontainers==2.3.0
     + sortedcontainers==2.4.0
    ");

    Ok(())
}

/// This tests conflicting groups in a virtual pyproject.toml
///
/// (One with no `[project]` which is allowed for specifically dependency-groups).
/// Currently this isn't supported, as we require a `PackageName` when representing
/// Conflicts internally.
#[test]
fn group_virtual() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // First we test that resolving with two groups that have
    // conflicting dependencies fails.
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [dependency-groups]
        group1 = ["sortedcontainers==2.3.0"]
        group2 = ["sortedcontainers==2.4.0"]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 1 (failure)
    ----- stderr -----
    warning: No `requires-python` value found in the workspace. Defaulting to `>=3.12`.
    error: No solution found when resolving dependencies
      cause: Because you require sortedcontainers==2.3.0 and sortedcontainers==2.4.0, we can conclude that your requirements are unsatisfiable.
    ");

    // And now with the same group configuration, we tell uv about
    // the conflicting groups, which forces it to resolve each in
    // their own fork.
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [tool.uv]
        conflicts = [
            [
              { group = "group1" },
              { group = "group2" },
            ],
        ]

        [dependency-groups]
        group1 = ["sortedcontainers==2.3.0"]
        group2 = ["sortedcontainers==2.4.0"]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock(), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Expected `package` field in conflicting entry: { group = "group1" }
    "#);

    Ok(())
}

/// Ref: <https://github.com/astral-sh/uv/issues/18428>
#[test]
fn groups_respect_supported_environments_when_filtering_wheels() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_exclude_newer("2025-09-28T00:00:00Z");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["markupsafe"]

        [dependency-groups]
        a = []
        b = []

        [tool.uv]
        environments = [
            "sys_platform == 'linux' and platform_machine == 'x86_64'",
        ]
        conflicts = [
            [
                { group = "a" },
                { group = "b" },
            ],
        ]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");

    let lock = context.read("uv.lock");

    // The `markupsafe` entry should only include wheels for Linux x86.
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock,
            @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        resolution-markers = [
            "platform_machine == 'x86_64' and sys_platform == 'linux'",
        ]
        supported-markers = [
            "platform_machine == 'x86_64' and sys_platform == 'linux'",
        ]
        conflicts = [[
            { package = "project", group = "a" },
            { package = "project", group = "b" },
        ]]

        [options]
        exclude-newer = "2025-09-28T00:00:00Z"

        [[package]]
        name = "markupsafe"
        version = "2.1.5"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/markupsafe-2.1.5.tar.gz", hash = "sha256:e38237d66e6760fe86fe38e4b6c70ff4eed7da50b15e8c0f2589f05850207f13", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/markupsafe-2.1.5-py3-none-any.whl", hash = "sha256:d0fe66b2745bbd943b48f3229667cf4506d88136363578b94e63d384e61d4984", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "markupsafe" },
        ]

        [package.metadata]
        requires-dist = [{ name = "markupsafe" }]

        [package.metadata.requires-dev]
        a = []
        b = []
        "#
        );
    });

    Ok(())
}

/// When using `tool.uv.environments`, do not repeat the marker of the `tool.uv.environments`
/// universe for all dependency edges, even when conflicts are involved.
#[test]
fn extra_conflict_environments_omit_redundant_markers() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_packse_index("packages/conflict-environments.toml")
        .with_exclude_newer("2025-09-28T00:00:00Z");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "bar"
        version = "0.1.0"
        requires-python = ">=3.10.0"
        dependencies = [
            "forked-package",
            "shared-package",
        ]

        [project.optional-dependencies]
        a = ["forked-package<2"]
        b = ["forked-package>=2"]

        [tool.uv]
        conflicts = [
            [{ extra = "a" }, { extra = "b" }],
        ]
        environments = [
            "sys_platform == 'darwin' and platform_machine == 'x86_64'",
            "sys_platform == 'linux' and platform_machine == 'x86_64'",
        ]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");

    let lock = context.read("uv.lock");

    // The `shared-package` dependency is shared by every fork, so it carries no marker,
    // while the forked `forked-package` dependencies keep theirs.
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock,
            @r#"
        version = 1
        revision = 3
        requires-python = ">=3.10.0"
        resolution-markers = [
            "platform_machine == 'x86_64' and sys_platform == 'darwin'",
            "platform_machine == 'x86_64' and sys_platform == 'linux'",
        ]
        supported-markers = [
            "platform_machine == 'x86_64' and sys_platform == 'darwin'",
            "platform_machine == 'x86_64' and sys_platform == 'linux'",
        ]
        conflicts = [[
            { package = "bar", extra = "a" },
            { package = "bar", extra = "b" },
        ]]

        [options]
        exclude-newer = "2025-09-28T00:00:00Z"

        [[package]]
        name = "bar"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "forked-package", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'extra-3-bar-a'" },
            { name = "forked-package", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'extra-3-bar-b' or extra != 'extra-3-bar-a'" },
            { name = "shared-package" },
        ]

        [package.optional-dependencies]
        a = [
            { name = "forked-package", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        b = [
            { name = "forked-package", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]

        [package.metadata]
        requires-dist = [
            { name = "forked-package" },
            { name = "forked-package", marker = "extra == 'a'", specifier = "<2" },
            { name = "forked-package", marker = "extra == 'b'", specifier = ">=2" },
            { name = "shared-package" },
        ]
        provides-extras = ["a", "b"]

        [[package]]
        name = "forked-package"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "platform_machine == 'x86_64' and sys_platform == 'darwin'",
            "platform_machine == 'x86_64' and sys_platform == 'linux'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/forked_package-1.0.0.tar.gz", hash = "sha256:9687e04510e0038e1cceed7220421fe32a21f717f9efed1336b0f1b5e94f4d5c", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/forked_package-1.0.0-py3-none-any.whl", hash = "sha256:a0863f9c1d05a36c66202292af879960b2fbba59b8acaf02b7188b3f19ff4098", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "forked-package"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "platform_machine == 'x86_64' and sys_platform == 'darwin'",
            "platform_machine == 'x86_64' and sys_platform == 'linux'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/forked_package-2.0.0.tar.gz", hash = "sha256:d47d999290e0d7ce36d5274aa4b0c8b0829ba34ef2f22da98c281c7d3d3a0907", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/forked_package-2.0.0-py3-none-any.whl", hash = "sha256:d8048bfe6b8ee82774dd31d7e4746d2aac0594915cdac5b9a73d17b35a9e302b", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "shared-package"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/shared_package-1.0.0.tar.gz", hash = "sha256:7a468f70abab5f82017f788e12f6c3d514c9c98e4ed67a6529f49a20d8c1735e", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/shared_package-1.0.0-py3-none-any.whl", hash = "sha256:1d72fa1ced818b5764e3c5d41153b394f33a7b5ea4532a86df82bed770f3317c", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    uv_snapshot!(context.filters(), context.lock().arg("--locked"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");

    Ok(())
}

/// This tests a case where we declare an extra and a group as conflicting.
#[test]
fn mixed() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // First we test that resolving with a conflicting extra
    // and group fails.
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        description = "Add your description here"
        requires-python = ">=3.12"

        [dependency-groups]
        group1 = ["sortedcontainers==2.3.0"]

        [project.optional-dependencies]
        extra1 = ["sortedcontainers==2.4.0"]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because project:group1 depends on sortedcontainers==2.3.0 and project[extra1] depends on sortedcontainers==2.4.0, we can conclude that project:group1 and project[extra1] are incompatible.
             And because your project requires project[extra1] and project:group1, we can conclude that your project's requirements are unsatisfiable.
    ");

    // And now with the same extra/group configuration, we tell uv
    // about the conflicting groups, which forces it to resolve each in
    // their own fork.
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        description = "Add your description here"
        requires-python = ">=3.12"

        [tool.uv]
        conflicts = [
            [
              { group = "group1" },
              { extra = "extra1" },
            ],
        ]

        [dependency-groups]
        group1 = ["sortedcontainers==2.3.0"]

        [project.optional-dependencies]
        extra1 = ["sortedcontainers==2.4.0"]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        conflicts = [[
            { package = "project", extra = "extra1" },
            { package = "project", group = "group1" },
        ]]

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }

        [package.optional-dependencies]
        extra1 = [
            { name = "sortedcontainers", version = "2.4.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]

        [package.dev-dependencies]
        group1 = [
            { name = "sortedcontainers", version = "2.3.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]

        [package.metadata]
        requires-dist = [{ name = "sortedcontainers", marker = "extra == 'extra1'", specifier = "==2.4.0" }]
        provides-extras = ["extra1"]

        [package.metadata.requires-dev]
        group1 = [{ name = "sortedcontainers", specifier = "==2.3.0" }]

        [[package]]
        name = "sortedcontainers"
        version = "2.3.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sortedcontainers-2.3.0.tar.gz", hash = "sha256:c7bfd220eb6dcc29774e2e05298edc2c5d62fa1086cc6409c800c7f413afe2a6", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sortedcontainers-2.3.0-py3-none-any.whl", hash = "sha256:3228c480e84b2b7a504f28bd68f73b8cfde1d1fddf8a47783b19a0dbcc93ffcf", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "sortedcontainers"
        version = "2.4.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sortedcontainers-2.4.0.tar.gz", hash = "sha256:fb6015d312cfaf15c65e0aeadac28191546cc25e48d41b126ecc322d5b116be2", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sortedcontainers-2.4.0-py3-none-any.whl", hash = "sha256:e90c0f20bb5c630bbf840e4ccc94441cd55ae8048cf55fc42df715d28b67cb80", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Re-run with `--locked`.
    uv_snapshot!(context.filters(), context.lock().arg("--locked"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");

    // Install from the lockfile.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked in [TIME]
    ");
    // Another install, but with the group enabled.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen").arg("--group=group1"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + sortedcontainers==2.3.0
    ");
    // Another install, but with the extra enabled.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen").arg("--extra=extra1"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - sortedcontainers==2.3.0
     + sortedcontainers==2.4.0
    ");
    // And finally, installing both the group and the extra should fail.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen").arg("--group=group1").arg("--extra=extra1"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Extra `extra1` and group `group1` are incompatible with the declared conflicts: {`project[extra1]`, `project:group1`}
    ");

    Ok(())
}

/// See <https://github.com/astral-sh/uv/issues/19106>
#[test]
fn group_activates_self_extra() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.optional-dependencies]
        dev = ["anyio"]
        summarize = ["anyio", "idna==3.6"]
        foo = ["idna==3.5"]

        [dependency-groups]
        dev = ["project[dev]"]

        [tool.uv]
        conflicts = [
          [{ extra = "dev" }, { extra = "summarize" }],
          [{ extra = "foo" }, { extra = "summarize" }],
        ]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    ");

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        conflicts = [[
            { package = "project", extra = "dev" },
            { package = "project", extra = "summarize" },
        ], [
            { package = "project", extra = "foo" },
            { package = "project", extra = "summarize" },
        ]]

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "anyio"
        version = "4.3.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "idna", version = "3.5", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'extra-7-project-dev' or (extra == 'extra-7-project-foo' and extra == 'extra-7-project-summarize')" },
            { name = "idna", version = "3.6", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'extra-7-project-summarize'" },
            { name = "sniffio" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/anyio-4.3.0.tar.gz", hash = "sha256:13a6d97fa30ec110d85e3949a30c92306f0178135048329f54a335c3dade753a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/anyio-4.3.0-py3-none-any.whl", hash = "sha256:c4f443e7e5a2c003b1534688207e85dbd11960efb66d4d6a4e7693fdfc6f5b33", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.5"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.5.tar.gz", hash = "sha256:b52e0d5cd15ab27c7a8fc6d6567950aea07184f1982eb244a816febdf79845d8", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.5-py3-none-any.whl", hash = "sha256:4331936a57b12ca883491bf69b71d44c498a77aed0af0b2b1e28f1b5d0706f88", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.6"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.6.tar.gz", hash = "sha256:9aae8f72192b28db0d56fcef130afe490d1538a8d1bf1700e6d219521421525f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.6-py3-none-any.whl", hash = "sha256:e80025850eafa8760055fd6f2f6e83f84bf13d4a844fe81abb2b499e3a3e8af0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }

        [package.optional-dependencies]
        dev = [
            { name = "anyio" },
        ]
        foo = [
            { name = "idna", version = "3.5", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        summarize = [
            { name = "anyio" },
            { name = "idna", version = "3.6", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]

        [package.dev-dependencies]
        dev = [
            { name = "project" },
            { name = "project", extra = ["dev"], marker = "extra == 'extra-7-project-dev' or (extra == 'extra-7-project-foo' and extra == 'extra-7-project-summarize')" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "anyio", marker = "extra == 'dev'" },
            { name = "anyio", marker = "extra == 'summarize'" },
            { name = "idna", marker = "extra == 'foo'", specifier = "==3.5" },
            { name = "idna", marker = "extra == 'summarize'", specifier = "==3.6" },
        ]
        provides-extras = ["dev", "summarize", "foo"]

        [package.metadata.requires-dev]
        dev = [{ name = "project", extras = ["dev"] }]

        [[package]]
        name = "sniffio"
        version = "1.3.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sniffio-1.3.1.tar.gz", hash = "sha256:ce520d2eb3c2be02f0c148dab5ba304e8705e1c7e7b4bec8a9146c464a597a6a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sniffio-1.3.1-py3-none-any.whl", hash = "sha256:2743fa2a853c508a2310882c0b4104631e0b0fcb855e00a912b7e3f27e6b3f05", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Re-run with `--locked`.
    uv_snapshot!(context.filters(), context.lock().arg("--locked"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    ");

    // Activating the `dev` group (the default) should install `idna==3.5` via `anyio`'s
    // conflict-gated edge, because the group itself enables the `dev` self-extra.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.5
     + sniffio==1.3.1
    ");

    // Enabling the extra explicitly should produce the same environment.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen").arg("--extra=dev"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 3 packages in [TIME]
    ");

    Ok(())
}

/// See [`group_activates_self_extra`]
///
/// This covers the non-project workspace case, which uses a separate code path.
#[test]
fn group_activates_self_extra_non_project_workspace() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let root_pyproject_toml = context.temp_dir.child("pyproject.toml");
    root_pyproject_toml.write_str(
        r#"
        [tool.uv.workspace]
        members = ["pkg1"]

        [tool.uv.sources]
        pkg1 = { workspace = true }

        [dependency-groups]
        dev = ["pkg1[dev]"]

        [tool.uv]
        conflicts = [
          [{ package = "pkg1", extra = "dev" }, { package = "pkg1", extra = "summarize" }],
          [{ package = "pkg1", extra = "foo" }, { package = "pkg1", extra = "summarize" }],
        ]
        "#,
    )?;

    let pkg1_pyproject_toml = context.temp_dir.child("pkg1").child("pyproject.toml");
    pkg1_pyproject_toml.write_str(
        r#"
        [project]
        name = "pkg1"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.optional-dependencies]
        dev = ["anyio"]
        summarize = ["anyio", "idna==3.6"]
        foo = ["idna==3.5"]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    ");

    // Activating the `dev` group (the default) installs `idna==3.5` via `anyio`'s
    // conflict-gated edge, because the group references `pkg1[dev]`.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==4.3.0
     + idna==3.5
     + pkg1==0.1.0 (from file://[TEMP_DIR]/pkg1)
     + sniffio==1.3.1
    ");

    // Enabling the extra explicitly produces the same environment.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen").arg("--extra=dev"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 4 packages in [TIME]
    ");

    Ok(())
}

#[test]
fn multiple_sources_index_disjoint_extras() -> Result<()> {
    let cu118_index =
        uv_test::packse::PackseServer::new("packages/sync-multiple-sources-index.toml");
    let cu124_index =
        uv_test::packse::PackseServer::new("packages/sync-multiple-sources-index.toml");
    let cu118_index_url = cu118_index.index_url();
    let cu124_index_url = cu124_index.index_url();
    let context = uv_test::test_context!("3.12")
        .with_packse_index("packages/sync-multiple-sources-index.toml")
        .with_exclude_newer("2025-01-30T00:00Z");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.optional-dependencies]
        cu118 = ["jinja2==3.1.2"]
        cu124 = ["jinja2==3.1.3"]

        [tool.uv]
        constraint-dependencies = ["markupsafe<3"]
        conflicts = [
            [
                {{ extra = "cu118" }},
                {{ extra = "cu124" }},
            ],
        ]

        [tool.uv.sources]
        jinja2 = [
            {{ index = "torch-cu118", extra = "cu118" }},
            {{ index = "torch-cu124", extra = "cu124" }},
        ]

        [[tool.uv.index]]
        name = "torch-cu118"
        url = "{cu118_index_url}"
        explicit = true

        [[tool.uv.index]]
        name = "torch-cu124"
        url = "{cu124_index_url}"
        explicit = true
        "#})?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");

    let lock = fs_err::read_to_string(context.temp_dir.join("uv.lock")).unwrap();
    let mut lock_filters = vec![
        (cu118_index_url.as_str(), "http://[TORCH-CU118]/simple/"),
        (cu124_index_url.as_str(), "http://[TORCH-CU124]/simple/"),
    ];
    lock_filters.extend(context.filters());

    insta::with_settings!({
        filters => lock_filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        conflicts = [[
            { package = "project", extra = "cu118" },
            { package = "project", extra = "cu124" },
        ]]

        [options]
        exclude-newer = "2025-01-30T00:00:00Z"

        [manifest]
        constraints = [{ name = "markupsafe", specifier = "<3" }]

        [[package]]
        name = "jinja2"
        version = "3.1.2"
        source = { registry = "http://[TORCH-CU118]/simple/" }
        dependencies = [
            { name = "markupsafe" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/jinja2-3.1.2.tar.gz", hash = "sha256:0bcc298fd1dec3c794f9c2297e92aca84212ba7fab2d4a07c16d57f9109c25df", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/jinja2-3.1.2-py3-none-any.whl", hash = "sha256:9df664d9fbd96dc6d4c1d513e82f141a80ff7713e1a46144a38d45984bdfd62c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "jinja2"
        version = "3.1.3"
        source = { registry = "http://[TORCH-CU124]/simple/" }
        dependencies = [
            { name = "markupsafe" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/jinja2-3.1.3.tar.gz", hash = "sha256:b2ececb7e299649721da5cfb6e178bdc3b4aeef39ac8bdc791bf52d9b68c1bba", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/jinja2-3.1.3-py3-none-any.whl", hash = "sha256:26ae100d7dfd6c1e4c532b44f127adfcc5b836f87b9643af33fc01e2406556bb", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "markupsafe"
        version = "2.1.5"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/markupsafe-2.1.5.tar.gz", hash = "sha256:e38237d66e6760fe86fe38e4b6c70ff4eed7da50b15e8c0f2589f05850207f13", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/markupsafe-2.1.5-py3-none-any.whl", hash = "sha256:d0fe66b2745bbd943b48f3229667cf4506d88136363578b94e63d384e61d4984", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }

        [package.optional-dependencies]
        cu118 = [
            { name = "jinja2", version = "3.1.2", source = { registry = "http://[TORCH-CU118]/simple/" } },
        ]
        cu124 = [
            { name = "jinja2", version = "3.1.3", source = { registry = "http://[TORCH-CU124]/simple/" } },
        ]

        [package.metadata]
        requires-dist = [
            { name = "jinja2", marker = "extra == 'cu118'", specifier = "==3.1.2", index = "http://[TORCH-CU118]/simple/", conflict = { package = "project", extra = "cu118" } },
            { name = "jinja2", marker = "extra == 'cu124'", specifier = "==3.1.3", index = "http://[TORCH-CU124]/simple/", conflict = { package = "project", extra = "cu124" } },
        ]
        provides-extras = ["cu118", "cu124"]
        "#
        );
    });

    // Re-run with `--locked`.
    uv_snapshot!(context.filters(), context.lock().arg("--locked"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");

    Ok(())
}

#[test]
fn multiple_sources_index_disjoint_groups() -> Result<()> {
    let cu118_index =
        uv_test::packse::PackseServer::new("packages/sync-multiple-sources-index.toml");
    let cu124_index =
        uv_test::packse::PackseServer::new("packages/sync-multiple-sources-index.toml");
    let cu118_index_url = cu118_index.index_url();
    let cu124_index_url = cu124_index.index_url();
    let context = uv_test::test_context!("3.12")
        .with_packse_index("packages/sync-multiple-sources-index.toml")
        .with_exclude_newer("2025-01-30T00:00Z");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [dependency-groups]
        cu118 = ["jinja2==3.1.2"]
        cu124 = ["jinja2==3.1.3"]

        [tool.uv]
        constraint-dependencies = ["markupsafe<3"]
        conflicts = [
            [
                {{ group = "cu118" }},
                {{ group = "cu124" }},
            ],
        ]

        [tool.uv.sources]
        jinja2 = [
            {{ index = "torch-cu118", group = "cu118" }},
            {{ index = "torch-cu124", group = "cu124" }},
        ]

        [[tool.uv.index]]
        name = "torch-cu118"
        url = "{cu118_index_url}"
        explicit = true

        [[tool.uv.index]]
        name = "torch-cu124"
        url = "{cu124_index_url}"
        explicit = true
        "#})?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");

    let lock = fs_err::read_to_string(context.temp_dir.join("uv.lock")).unwrap();
    let mut lock_filters = vec![
        (cu118_index_url.as_str(), "http://[TORCH-CU118]/simple/"),
        (cu124_index_url.as_str(), "http://[TORCH-CU124]/simple/"),
    ];
    lock_filters.extend(context.filters());

    insta::with_settings!({
        filters => lock_filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        conflicts = [[
            { package = "project", group = "cu118" },
            { package = "project", group = "cu124" },
        ]]

        [options]
        exclude-newer = "2025-01-30T00:00:00Z"

        [manifest]
        constraints = [{ name = "markupsafe", specifier = "<3" }]

        [[package]]
        name = "jinja2"
        version = "3.1.2"
        source = { registry = "http://[TORCH-CU118]/simple/" }
        dependencies = [
            { name = "markupsafe" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/jinja2-3.1.2.tar.gz", hash = "sha256:0bcc298fd1dec3c794f9c2297e92aca84212ba7fab2d4a07c16d57f9109c25df", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/jinja2-3.1.2-py3-none-any.whl", hash = "sha256:9df664d9fbd96dc6d4c1d513e82f141a80ff7713e1a46144a38d45984bdfd62c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "jinja2"
        version = "3.1.3"
        source = { registry = "http://[TORCH-CU124]/simple/" }
        dependencies = [
            { name = "markupsafe" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/jinja2-3.1.3.tar.gz", hash = "sha256:b2ececb7e299649721da5cfb6e178bdc3b4aeef39ac8bdc791bf52d9b68c1bba", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/jinja2-3.1.3-py3-none-any.whl", hash = "sha256:26ae100d7dfd6c1e4c532b44f127adfcc5b836f87b9643af33fc01e2406556bb", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "markupsafe"
        version = "2.1.5"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/markupsafe-2.1.5.tar.gz", hash = "sha256:e38237d66e6760fe86fe38e4b6c70ff4eed7da50b15e8c0f2589f05850207f13", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/markupsafe-2.1.5-py3-none-any.whl", hash = "sha256:d0fe66b2745bbd943b48f3229667cf4506d88136363578b94e63d384e61d4984", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }

        [package.dev-dependencies]
        cu118 = [
            { name = "jinja2", version = "3.1.2", source = { registry = "http://[TORCH-CU118]/simple/" } },
        ]
        cu124 = [
            { name = "jinja2", version = "3.1.3", source = { registry = "http://[TORCH-CU124]/simple/" } },
        ]

        [package.metadata]

        [package.metadata.requires-dev]
        cu118 = [{ name = "jinja2", specifier = "==3.1.2", index = "http://[TORCH-CU118]/simple/", conflict = { package = "project", group = "cu118" } }]
        cu124 = [{ name = "jinja2", specifier = "==3.1.3", index = "http://[TORCH-CU124]/simple/", conflict = { package = "project", group = "cu124" } }]
        "#
        );
    });

    // Re-run with `--locked`.
    uv_snapshot!(context.filters(), context.lock().arg("--locked"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");

    Ok(())
}

#[test]
fn multiple_sources_index_disjoint_extras_with_extra() -> Result<()> {
    let cu118_index =
        uv_test::packse::PackseServer::new("packages/sync-multiple-sources-index.toml");
    let cu124_index =
        uv_test::packse::PackseServer::new("packages/sync-multiple-sources-index.toml");
    let cu118_index_url = cu118_index.index_url();
    let cu124_index_url = cu124_index.index_url();
    let context = uv_test::test_context!("3.12")
        .with_packse_index("packages/sync-multiple-sources-index.toml")
        .with_exclude_newer("2025-01-30T00:00Z");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.optional-dependencies]
        cu118 = ["jinja2[i18n]==3.1.2"]
        cu124 = ["jinja2[i18n]==3.1.3"]

        [tool.uv]
        constraint-dependencies = ["markupsafe<3"]
        conflicts = [
            [
                {{ extra = "cu118" }},
                {{ extra = "cu124" }},
            ],
        ]

        [tool.uv.sources]
        jinja2 = [
            {{ index = "torch-cu118", extra = "cu118" }},
            {{ index = "torch-cu124", extra = "cu124" }},
        ]

        [[tool.uv.index]]
        name = "torch-cu118"
        url = "{cu118_index_url}"
        explicit = true

        [[tool.uv.index]]
        name = "torch-cu124"
        url = "{cu124_index_url}"
        explicit = true
        "#})?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    ");

    let lock = fs_err::read_to_string(context.temp_dir.join("uv.lock")).unwrap();
    let mut lock_filters = vec![
        (cu118_index_url.as_str(), "http://[TORCH-CU118]/simple/"),
        (cu124_index_url.as_str(), "http://[TORCH-CU124]/simple/"),
    ];
    lock_filters.extend(context.filters());

    insta::with_settings!({
        filters => lock_filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        conflicts = [[
            { package = "project", extra = "cu118" },
            { package = "project", extra = "cu124" },
        ]]

        [options]
        exclude-newer = "2025-01-30T00:00:00Z"

        [manifest]
        constraints = [{ name = "markupsafe", specifier = "<3" }]

        [[package]]
        name = "babel"
        version = "2.16.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/babel-2.16.0.tar.gz", hash = "sha256:7ea36c622fb6f152022de1cc793821d621f592a6e36c44f9ac837dfc75ac8de2", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/babel-2.16.0-py3-none-any.whl", hash = "sha256:f710f834a3355f4d7edb7678c44a621c28e614bed6614c04d7e2a72e119a212c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "jinja2"
        version = "3.1.2"
        source = { registry = "http://[TORCH-CU118]/simple/" }
        dependencies = [
            { name = "markupsafe" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/jinja2-3.1.2.tar.gz", hash = "sha256:0bcc298fd1dec3c794f9c2297e92aca84212ba7fab2d4a07c16d57f9109c25df", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/jinja2-3.1.2-py3-none-any.whl", hash = "sha256:9df664d9fbd96dc6d4c1d513e82f141a80ff7713e1a46144a38d45984bdfd62c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [package.optional-dependencies]
        i18n = [
            { name = "babel" },
        ]

        [[package]]
        name = "jinja2"
        version = "3.1.3"
        source = { registry = "http://[TORCH-CU124]/simple/" }
        dependencies = [
            { name = "markupsafe" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/jinja2-3.1.3.tar.gz", hash = "sha256:b2ececb7e299649721da5cfb6e178bdc3b4aeef39ac8bdc791bf52d9b68c1bba", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/jinja2-3.1.3-py3-none-any.whl", hash = "sha256:26ae100d7dfd6c1e4c532b44f127adfcc5b836f87b9643af33fc01e2406556bb", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [package.optional-dependencies]
        i18n = [
            { name = "babel" },
        ]

        [[package]]
        name = "markupsafe"
        version = "2.1.5"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/markupsafe-2.1.5.tar.gz", hash = "sha256:e38237d66e6760fe86fe38e4b6c70ff4eed7da50b15e8c0f2589f05850207f13", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/markupsafe-2.1.5-py3-none-any.whl", hash = "sha256:d0fe66b2745bbd943b48f3229667cf4506d88136363578b94e63d384e61d4984", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }

        [package.optional-dependencies]
        cu118 = [
            { name = "jinja2", version = "3.1.2", source = { registry = "http://[TORCH-CU118]/simple/" }, extra = ["i18n"], marker = "extra == 'extra-7-project-cu118'" },
        ]
        cu124 = [
            { name = "jinja2", version = "3.1.3", source = { registry = "http://[TORCH-CU124]/simple/" }, extra = ["i18n"], marker = "extra == 'extra-7-project-cu124'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "jinja2", extras = ["i18n"], marker = "extra == 'cu118'", specifier = "==3.1.2", index = "http://[TORCH-CU118]/simple/", conflict = { package = "project", extra = "cu118" } },
            { name = "jinja2", extras = ["i18n"], marker = "extra == 'cu124'", specifier = "==3.1.3", index = "http://[TORCH-CU124]/simple/", conflict = { package = "project", extra = "cu124" } },
        ]
        provides-extras = ["cu118", "cu124"]
        "#
        );
    });

    // Re-run with `--locked`.
    uv_snapshot!(context.filters(), context.lock().arg("--locked"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    ");

    Ok(())
}

#[test]
fn multiple_sources_index_disjoint_extras_with_marker() -> Result<()> {
    let mut indexes = [
        PackseServer::new("packages/sync-multiple-sources-index.toml"),
        PackseServer::new("packages/sync-multiple-sources-index.toml"),
        PackseServer::new("packages/sync-multiple-sources-index.toml"),
    ];
    // Registry URLs break ties between identical package versions in the lockfile.
    indexes.sort_by_key(PackseServer::index_url);
    let [default_index, cu118_index, cu124_index] = indexes;
    let cu118_index_url = cu118_index.index_url();
    let cu124_index_url = cu124_index.index_url();
    let context = uv_test::test_context!("3.12")
        .with_default_index(&default_index.index_url())
        .with_exclude_newer("2025-01-30T00:00Z");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.optional-dependencies]
        cu118 = ["jinja2==3.1.2"]
        cu124 = ["jinja2==3.1.3"]

        [tool.uv]
        constraint-dependencies = ["markupsafe<3"]
        conflicts = [
            [
                {{ extra = "cu118" }},
                {{ extra = "cu124" }},
            ],
        ]

        [tool.uv.sources]
        jinja2 = [
            {{ index = "torch-cu118", extra = "cu118", marker = "sys_platform == 'darwin'" }},
            {{ index = "torch-cu124", extra = "cu124" }},
        ]

        [[tool.uv.index]]
        name = "torch-cu118"
        url = "{cu118_index_url}"
        explicit = true

        [[tool.uv.index]]
        name = "torch-cu124"
        url = "{cu124_index_url}"
        explicit = true
        "#})?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    ");

    let lock = fs_err::read_to_string(context.temp_dir.join("uv.lock")).unwrap();
    let mut lock_filters = vec![
        (cu118_index_url.as_str(), "http://[TORCH-CU118]/simple/"),
        (cu124_index_url.as_str(), "http://[TORCH-CU124]/simple/"),
    ];
    lock_filters.extend(context.filters());

    insta::with_settings!({
        filters => lock_filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        resolution-markers = [
            "extra != 'extra-7-project-cu118' and extra == 'extra-7-project-cu124'",
            "sys_platform == 'darwin' and extra == 'extra-7-project-cu118' and extra != 'extra-7-project-cu124'",
            "sys_platform != 'darwin' and extra == 'extra-7-project-cu118' and extra != 'extra-7-project-cu124'",
            "extra != 'extra-7-project-cu118' and extra != 'extra-7-project-cu124'",
        ]
        conflicts = [[
            { package = "project", extra = "cu118" },
            { package = "project", extra = "cu124" },
        ]]

        [options]
        exclude-newer = "2025-01-30T00:00:00Z"

        [manifest]
        constraints = [{ name = "markupsafe", specifier = "<3" }]

        [[package]]
        name = "jinja2"
        version = "3.1.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform != 'darwin'",
        ]
        dependencies = [
            { name = "markupsafe", marker = "sys_platform != 'darwin'" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/jinja2-3.1.2.tar.gz", hash = "sha256:0bcc298fd1dec3c794f9c2297e92aca84212ba7fab2d4a07c16d57f9109c25df", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/jinja2-3.1.2-py3-none-any.whl", hash = "sha256:9df664d9fbd96dc6d4c1d513e82f141a80ff7713e1a46144a38d45984bdfd62c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "jinja2"
        version = "3.1.2"
        source = { registry = "http://[TORCH-CU118]/simple/" }
        resolution-markers = [
            "sys_platform == 'darwin'",
        ]
        dependencies = [
            { name = "markupsafe", marker = "sys_platform == 'darwin'" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/jinja2-3.1.2.tar.gz", hash = "sha256:0bcc298fd1dec3c794f9c2297e92aca84212ba7fab2d4a07c16d57f9109c25df", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/jinja2-3.1.2-py3-none-any.whl", hash = "sha256:9df664d9fbd96dc6d4c1d513e82f141a80ff7713e1a46144a38d45984bdfd62c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "jinja2"
        version = "3.1.3"
        source = { registry = "http://[TORCH-CU124]/simple/" }
        dependencies = [
            { name = "markupsafe" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/jinja2-3.1.3.tar.gz", hash = "sha256:b2ececb7e299649721da5cfb6e178bdc3b4aeef39ac8bdc791bf52d9b68c1bba", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/jinja2-3.1.3-py3-none-any.whl", hash = "sha256:26ae100d7dfd6c1e4c532b44f127adfcc5b836f87b9643af33fc01e2406556bb", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "markupsafe"
        version = "2.1.5"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/markupsafe-2.1.5.tar.gz", hash = "sha256:e38237d66e6760fe86fe38e4b6c70ff4eed7da50b15e8c0f2589f05850207f13", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/markupsafe-2.1.5-py3-none-any.whl", hash = "sha256:d0fe66b2745bbd943b48f3229667cf4506d88136363578b94e63d384e61d4984", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }

        [package.optional-dependencies]
        cu118 = [
            { name = "jinja2", version = "3.1.2", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "(sys_platform != 'darwin' and extra == 'extra-7-project-cu118') or (extra == 'extra-7-project-cu118' and extra == 'extra-7-project-cu124')" },
            { name = "jinja2", version = "3.1.2", source = { registry = "http://[TORCH-CU118]/simple/" }, marker = "(sys_platform == 'darwin' and extra == 'extra-7-project-cu118') or (extra == 'extra-7-project-cu118' and extra == 'extra-7-project-cu124')" },
        ]
        cu124 = [
            { name = "jinja2", version = "3.1.3", source = { registry = "http://[TORCH-CU124]/simple/" } },
        ]

        [package.metadata]
        requires-dist = [
            { name = "jinja2", marker = "sys_platform == 'darwin' and extra == 'cu118'", specifier = "==3.1.2", index = "http://[TORCH-CU118]/simple/", conflict = { package = "project", extra = "cu118" } },
            { name = "jinja2", marker = "sys_platform != 'darwin' and extra == 'cu118'", specifier = "==3.1.2" },
            { name = "jinja2", marker = "extra == 'cu124'", specifier = "==3.1.3", index = "http://[TORCH-CU124]/simple/", conflict = { package = "project", extra = "cu124" } },
        ]
        provides-extras = ["cu118", "cu124"]
        "#
        );
    });

    // Re-run with `--locked`.
    uv_snapshot!(context.filters(), context.lock().arg("--locked"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    ");

    Ok(())
}

/// Tests that forks excluding both conflicting extras are handled correctly.
///
/// This previously failed where running `uv sync` wouldn't install anything,
/// despite `sniffio` being an unconditional dependency.
#[test]
fn non_optional_dependency_extra() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.11"
        dependencies = [
          "sniffio>=1",
        ]

        [project.optional-dependencies]
        x1 = ["idna==3.5"]
        x2 = ["idna==3.6"]

        [tool.uv]
        conflicts = [
          [
            {package = "project", extra = "x1"},
            {package = "project", extra = "x2"},
          ],
        ]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.sync(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + sniffio==1.3.1
    ");

    Ok(())
}

/// Like `non_optional_dependency_extra`, but for groups.
///
/// This test never regressed, but we added it here to ensure it doesn't.
#[test]
fn non_optional_dependency_group() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.11"
        dependencies = [
          "sniffio>=1",
        ]

        [dependency-groups]
        g1 = ["idna==3.5"]
        g2 = ["idna==3.6"]

        [tool.uv]
        conflicts = [
          [
            {package = "project", group = "g1"},
            {package = "project", group = "g2"},
          ],
        ]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.sync(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + sniffio==1.3.1
    ");

    Ok(())
}

/// Like `non_optional_dependency_extra`, but for extras and groups mixed
/// together.
///
/// This test never regressed, but we added it here to ensure it doesn't.
#[test]
fn non_optional_dependency_mixed() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.11"
        dependencies = [
          "sniffio>=1",
        ]

        [project.optional-dependencies]
        x1 = ["idna==3.5"]

        [dependency-groups]
        x2 = ["idna==3.6"]

        [tool.uv]
        conflicts = [
          [
            {package = "project", extra = "x1"},
            {package = "project", group = "x2"},
          ],
        ]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.sync(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + sniffio==1.3.1
    ");

    Ok(())
}

/// This tests a case where there are three extras, with two conflicting (`foo`
/// and `bar`), and the third (`baz`) containing a dependent (`anyio`) of
/// both dependencies (`idna==3.5` and `idna==3.6`) listed in the conflicting
/// extras.
///
/// This is a regression test for a more minimal case than was reported[1].
/// Specifically, this would produce an ambiguous lock file where both
/// `idna==3.5` and `idna==3.6` could be installed in some circumstances.
///
/// [1]: <https://github.com/astral-sh/uv/issues/9289>
#[test]
fn shared_optional_dependency_extra1() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.optional-dependencies]
        foo = [
          "idna==3.5",
        ]
        bar = [
          "idna==3.6",
        ]
        baz = [
          "anyio",
        ]

        [tool.uv]
        conflicts = [
          [
            { extra = "foo" },
            { extra = "bar" },
          ],
        ]
        "#,
    )?;

    // This shouldn't install two versions of `idna`, only one, `idna==3.5`.
    uv_snapshot!(context.filters(), context.sync().arg("--extra=baz").arg("--extra=foo"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.5
     + sniffio==1.3.1
    ");

    let lock = fs_err::read_to_string(context.temp_dir.join("uv.lock")).unwrap();
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        conflicts = [[
            { package = "project", extra = "bar" },
            { package = "project", extra = "foo" },
        ]]

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "anyio"
        version = "4.3.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "idna", version = "3.5", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'extra-7-project-foo'" },
            { name = "idna", version = "3.6", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'extra-7-project-bar' or extra != 'extra-7-project-foo'" },
            { name = "sniffio" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/anyio-4.3.0.tar.gz", hash = "sha256:13a6d97fa30ec110d85e3949a30c92306f0178135048329f54a335c3dade753a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/anyio-4.3.0-py3-none-any.whl", hash = "sha256:c4f443e7e5a2c003b1534688207e85dbd11960efb66d4d6a4e7693fdfc6f5b33", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.5"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.5.tar.gz", hash = "sha256:b52e0d5cd15ab27c7a8fc6d6567950aea07184f1982eb244a816febdf79845d8", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.5-py3-none-any.whl", hash = "sha256:4331936a57b12ca883491bf69b71d44c498a77aed0af0b2b1e28f1b5d0706f88", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.6"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.6.tar.gz", hash = "sha256:9aae8f72192b28db0d56fcef130afe490d1538a8d1bf1700e6d219521421525f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.6-py3-none-any.whl", hash = "sha256:e80025850eafa8760055fd6f2f6e83f84bf13d4a844fe81abb2b499e3a3e8af0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }

        [package.optional-dependencies]
        bar = [
            { name = "idna", version = "3.6", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        baz = [
            { name = "anyio" },
        ]
        foo = [
            { name = "idna", version = "3.5", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]

        [package.metadata]
        requires-dist = [
            { name = "anyio", marker = "extra == 'baz'" },
            { name = "idna", marker = "extra == 'bar'", specifier = "==3.6" },
            { name = "idna", marker = "extra == 'foo'", specifier = "==3.5" },
        ]
        provides-extras = ["foo", "bar", "baz"]

        [[package]]
        name = "sniffio"
        version = "1.3.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sniffio-1.3.1.tar.gz", hash = "sha256:ce520d2eb3c2be02f0c148dab5ba304e8705e1c7e7b4bec8a9146c464a597a6a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sniffio-1.3.1-py3-none-any.whl", hash = "sha256:2743fa2a853c508a2310882c0b4104631e0b0fcb855e00a912b7e3f27e6b3f05", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    Ok(())
}

/// This is like `shared_optional_dependency_extra1`, but for groups.
///
/// Ref <https://github.com/astral-sh/uv/issues/9289>
#[test]
fn shared_optional_dependency_group1() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [dependency-groups]
        foo = [
          "idna==3.5",
        ]
        bar = [
          "idna==3.6",
        ]
        baz = [
          "anyio",
        ]

        [tool.uv]
        conflicts = [
          [
            { group = "foo" },
            { group = "bar" },
          ],
        ]
        "#,
    )?;

    // This shouldn't install two versions of `idna`, only one, `idna==3.5`.
    uv_snapshot!(context.filters(), context.sync().arg("--group=baz").arg("--group=foo"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.5
     + sniffio==1.3.1
    ");

    let lock = fs_err::read_to_string(context.temp_dir.join("uv.lock")).unwrap();
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        conflicts = [[
            { package = "project", group = "bar" },
            { package = "project", group = "foo" },
        ]]

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "anyio"
        version = "4.3.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "idna", version = "3.5", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'group-7-project-foo'" },
            { name = "idna", version = "3.6", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'group-7-project-bar' or extra != 'group-7-project-foo'" },
            { name = "sniffio" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/anyio-4.3.0.tar.gz", hash = "sha256:13a6d97fa30ec110d85e3949a30c92306f0178135048329f54a335c3dade753a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/anyio-4.3.0-py3-none-any.whl", hash = "sha256:c4f443e7e5a2c003b1534688207e85dbd11960efb66d4d6a4e7693fdfc6f5b33", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.5"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.5.tar.gz", hash = "sha256:b52e0d5cd15ab27c7a8fc6d6567950aea07184f1982eb244a816febdf79845d8", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.5-py3-none-any.whl", hash = "sha256:4331936a57b12ca883491bf69b71d44c498a77aed0af0b2b1e28f1b5d0706f88", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.6"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.6.tar.gz", hash = "sha256:9aae8f72192b28db0d56fcef130afe490d1538a8d1bf1700e6d219521421525f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.6-py3-none-any.whl", hash = "sha256:e80025850eafa8760055fd6f2f6e83f84bf13d4a844fe81abb2b499e3a3e8af0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }

        [package.dev-dependencies]
        bar = [
            { name = "idna", version = "3.6", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        baz = [
            { name = "anyio" },
        ]
        foo = [
            { name = "idna", version = "3.5", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]

        [package.metadata]

        [package.metadata.requires-dev]
        bar = [{ name = "idna", specifier = "==3.6" }]
        baz = [{ name = "anyio" }]
        foo = [{ name = "idna", specifier = "==3.5" }]

        [[package]]
        name = "sniffio"
        version = "1.3.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sniffio-1.3.1.tar.gz", hash = "sha256:ce520d2eb3c2be02f0c148dab5ba304e8705e1c7e7b4bec8a9146c464a597a6a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sniffio-1.3.1-py3-none-any.whl", hash = "sha256:2743fa2a853c508a2310882c0b4104631e0b0fcb855e00a912b7e3f27e6b3f05", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    Ok(())
}

/// This is like `shared_optional_dependency_extra1`, but for extras/groups.
///
/// Ref <https://github.com/astral-sh/uv/issues/9289>
#[test]
fn shared_optional_dependency_mixed1() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.optional-dependencies]
        foo = [
          "idna==3.5",
        ]

        [dependency-groups]
        bar = [
          "idna==3.6",
        ]
        baz = [
          "anyio",
        ]

        [tool.uv]
        conflicts = [
          [
            { extra = "foo" },
            { group = "bar" },
          ],
        ]
        "#,
    )?;

    // This shouldn't install two versions of `idna`, only one, `idna==3.5`.
    uv_snapshot!(context.filters(), context.sync().arg("--group=baz").arg("--extra=foo"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.5
     + sniffio==1.3.1
    ");

    let lock = fs_err::read_to_string(context.temp_dir.join("uv.lock")).unwrap();
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        conflicts = [[
            { package = "project", extra = "foo" },
            { package = "project", group = "bar" },
        ]]

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "anyio"
        version = "4.3.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "idna", version = "3.5", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'extra-7-project-foo'" },
            { name = "idna", version = "3.6", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'group-7-project-bar' or extra != 'extra-7-project-foo'" },
            { name = "sniffio" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/anyio-4.3.0.tar.gz", hash = "sha256:13a6d97fa30ec110d85e3949a30c92306f0178135048329f54a335c3dade753a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/anyio-4.3.0-py3-none-any.whl", hash = "sha256:c4f443e7e5a2c003b1534688207e85dbd11960efb66d4d6a4e7693fdfc6f5b33", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.5"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.5.tar.gz", hash = "sha256:b52e0d5cd15ab27c7a8fc6d6567950aea07184f1982eb244a816febdf79845d8", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.5-py3-none-any.whl", hash = "sha256:4331936a57b12ca883491bf69b71d44c498a77aed0af0b2b1e28f1b5d0706f88", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.6"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.6.tar.gz", hash = "sha256:9aae8f72192b28db0d56fcef130afe490d1538a8d1bf1700e6d219521421525f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.6-py3-none-any.whl", hash = "sha256:e80025850eafa8760055fd6f2f6e83f84bf13d4a844fe81abb2b499e3a3e8af0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }

        [package.optional-dependencies]
        foo = [
            { name = "idna", version = "3.5", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]

        [package.dev-dependencies]
        bar = [
            { name = "idna", version = "3.6", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        baz = [
            { name = "anyio" },
        ]

        [package.metadata]
        requires-dist = [{ name = "idna", marker = "extra == 'foo'", specifier = "==3.5" }]
        provides-extras = ["foo"]

        [package.metadata.requires-dev]
        bar = [{ name = "idna", specifier = "==3.6" }]
        baz = [{ name = "anyio" }]

        [[package]]
        name = "sniffio"
        version = "1.3.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sniffio-1.3.1.tar.gz", hash = "sha256:ce520d2eb3c2be02f0c148dab5ba304e8705e1c7e7b4bec8a9146c464a597a6a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sniffio-1.3.1-py3-none-any.whl", hash = "sha256:2743fa2a853c508a2310882c0b4104631e0b0fcb855e00a912b7e3f27e6b3f05", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    Ok(())
}

/// Another variation on `shared_optional_dependency_extra1`, but with
/// a slightly different outcome. In this case, when one of the extras
/// is enabled, the `sniffio` dependency was not installed.
///
/// Regression test for: <https://github.com/astral-sh/uv/issues/9289>
/// Regression test for: <https://github.com/astral-sh/uv/issues/9622>
/// Regression test for: <https://github.com/astral-sh/uv/issues/9640>
#[test]
fn shared_optional_dependency_extra2() -> Result<()> {
    let context = uv_test::test_context!("3.11");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.11,<3.12"
        dependencies = []

        [project.optional-dependencies]
        foo = [
          "idna==3.5",
          "anyio",
        ]
        bar = [
          "idna==3.6",
          "anyio",
        ]

        [tool.uv]
        conflicts = [
          [
            { extra = "foo" },
            { extra = "bar" },
          ],
        ]
        "#,
    )?;

    // This shouldn't install two versions of `idna`, only one, `idna==3.6`.
    uv_snapshot!(context.filters(), context.sync().arg("--extra=bar"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    let lock = fs_err::read_to_string(context.temp_dir.join("uv.lock")).unwrap();
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = "==3.11.*"
        conflicts = [[
            { package = "project", extra = "bar" },
            { package = "project", extra = "foo" },
        ]]

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "anyio"
        version = "4.3.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "idna", version = "3.5", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'extra-7-project-foo'" },
            { name = "idna", version = "3.6", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'extra-7-project-bar'" },
            { name = "sniffio" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/anyio-4.3.0.tar.gz", hash = "sha256:13a6d97fa30ec110d85e3949a30c92306f0178135048329f54a335c3dade753a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/anyio-4.3.0-py3-none-any.whl", hash = "sha256:c4f443e7e5a2c003b1534688207e85dbd11960efb66d4d6a4e7693fdfc6f5b33", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.5"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.5.tar.gz", hash = "sha256:b52e0d5cd15ab27c7a8fc6d6567950aea07184f1982eb244a816febdf79845d8", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.5-py3-none-any.whl", hash = "sha256:4331936a57b12ca883491bf69b71d44c498a77aed0af0b2b1e28f1b5d0706f88", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.6"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.6.tar.gz", hash = "sha256:9aae8f72192b28db0d56fcef130afe490d1538a8d1bf1700e6d219521421525f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.6-py3-none-any.whl", hash = "sha256:e80025850eafa8760055fd6f2f6e83f84bf13d4a844fe81abb2b499e3a3e8af0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }

        [package.optional-dependencies]
        bar = [
            { name = "anyio" },
            { name = "idna", version = "3.6", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        foo = [
            { name = "anyio" },
            { name = "idna", version = "3.5", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]

        [package.metadata]
        requires-dist = [
            { name = "anyio", marker = "extra == 'bar'" },
            { name = "anyio", marker = "extra == 'foo'" },
            { name = "idna", marker = "extra == 'bar'", specifier = "==3.6" },
            { name = "idna", marker = "extra == 'foo'", specifier = "==3.5" },
        ]
        provides-extras = ["foo", "bar"]

        [[package]]
        name = "sniffio"
        version = "1.3.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sniffio-1.3.1.tar.gz", hash = "sha256:ce520d2eb3c2be02f0c148dab5ba304e8705e1c7e7b4bec8a9146c464a597a6a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sniffio-1.3.1-py3-none-any.whl", hash = "sha256:2743fa2a853c508a2310882c0b4104631e0b0fcb855e00a912b7e3f27e6b3f05", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    Ok(())
}

/// Like `shared_optional_dependency_extra2`, but for groups.
///
/// Regression test for: <https://github.com/astral-sh/uv/issues/9289>
/// Regression test for: <https://github.com/astral-sh/uv/issues/9622>
/// Regression test for: <https://github.com/astral-sh/uv/issues/9640>
#[test]
fn shared_optional_dependency_group2() -> Result<()> {
    let context = uv_test::test_context!("3.11");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.11,<3.12"
        dependencies = []

        [dependency-groups]
        foo = [
          "idna==3.5",
          "anyio",
        ]
        bar = [
          "idna==3.6",
          "anyio",
        ]

        [tool.uv]
        conflicts = [
          [
            { group = "foo" },
            { group = "bar" },
          ],
        ]
        "#,
    )?;

    // This shouldn't install two versions of `idna`, only one, `idna==3.6`.
    uv_snapshot!(context.filters(), context.sync().arg("--group=bar"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    let lock = fs_err::read_to_string(context.temp_dir.join("uv.lock")).unwrap();
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = "==3.11.*"
        conflicts = [[
            { package = "project", group = "bar" },
            { package = "project", group = "foo" },
        ]]

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "anyio"
        version = "4.3.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "idna", version = "3.5", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'group-7-project-foo'" },
            { name = "idna", version = "3.6", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'group-7-project-bar'" },
            { name = "sniffio" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/anyio-4.3.0.tar.gz", hash = "sha256:13a6d97fa30ec110d85e3949a30c92306f0178135048329f54a335c3dade753a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/anyio-4.3.0-py3-none-any.whl", hash = "sha256:c4f443e7e5a2c003b1534688207e85dbd11960efb66d4d6a4e7693fdfc6f5b33", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.5"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.5.tar.gz", hash = "sha256:b52e0d5cd15ab27c7a8fc6d6567950aea07184f1982eb244a816febdf79845d8", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.5-py3-none-any.whl", hash = "sha256:4331936a57b12ca883491bf69b71d44c498a77aed0af0b2b1e28f1b5d0706f88", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.6"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.6.tar.gz", hash = "sha256:9aae8f72192b28db0d56fcef130afe490d1538a8d1bf1700e6d219521421525f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.6-py3-none-any.whl", hash = "sha256:e80025850eafa8760055fd6f2f6e83f84bf13d4a844fe81abb2b499e3a3e8af0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }

        [package.dev-dependencies]
        bar = [
            { name = "anyio" },
            { name = "idna", version = "3.6", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        foo = [
            { name = "anyio" },
            { name = "idna", version = "3.5", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]

        [package.metadata]

        [package.metadata.requires-dev]
        bar = [
            { name = "anyio" },
            { name = "idna", specifier = "==3.6" },
        ]
        foo = [
            { name = "anyio" },
            { name = "idna", specifier = "==3.5" },
        ]

        [[package]]
        name = "sniffio"
        version = "1.3.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sniffio-1.3.1.tar.gz", hash = "sha256:ce520d2eb3c2be02f0c148dab5ba304e8705e1c7e7b4bec8a9146c464a597a6a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sniffio-1.3.1-py3-none-any.whl", hash = "sha256:2743fa2a853c508a2310882c0b4104631e0b0fcb855e00a912b7e3f27e6b3f05", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    Ok(())
}

/// Like `shared_optional_dependency_extra2`, but for extras/groups.
///
/// Regression test for: <https://github.com/astral-sh/uv/issues/9289>
/// Regression test for: <https://github.com/astral-sh/uv/issues/9622>
/// Regression test for: <https://github.com/astral-sh/uv/issues/9640>
#[test]
fn shared_optional_dependency_mixed2() -> Result<()> {
    let context = uv_test::test_context!("3.11");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.11,<3.12"
        dependencies = []

        [project.optional-dependencies]
        foo = [
          "idna==3.5",
          "anyio",
        ]

        [dependency-groups]
        bar = [
          "idna==3.6",
          "anyio",
        ]

        [tool.uv]
        conflicts = [
          [
            { extra = "foo" },
            { group = "bar" },
          ],
        ]
        "#,
    )?;

    // This shouldn't install two versions of `idna`, only one, `idna==3.6`.
    uv_snapshot!(context.filters(), context.sync().arg("--group=bar"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    let lock = fs_err::read_to_string(context.temp_dir.join("uv.lock")).unwrap();
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = "==3.11.*"
        conflicts = [[
            { package = "project", extra = "foo" },
            { package = "project", group = "bar" },
        ]]

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "anyio"
        version = "4.3.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "idna", version = "3.5", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'extra-7-project-foo'" },
            { name = "idna", version = "3.6", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'group-7-project-bar'" },
            { name = "sniffio" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/anyio-4.3.0.tar.gz", hash = "sha256:13a6d97fa30ec110d85e3949a30c92306f0178135048329f54a335c3dade753a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/anyio-4.3.0-py3-none-any.whl", hash = "sha256:c4f443e7e5a2c003b1534688207e85dbd11960efb66d4d6a4e7693fdfc6f5b33", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.5"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.5.tar.gz", hash = "sha256:b52e0d5cd15ab27c7a8fc6d6567950aea07184f1982eb244a816febdf79845d8", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.5-py3-none-any.whl", hash = "sha256:4331936a57b12ca883491bf69b71d44c498a77aed0af0b2b1e28f1b5d0706f88", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.6"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.6.tar.gz", hash = "sha256:9aae8f72192b28db0d56fcef130afe490d1538a8d1bf1700e6d219521421525f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.6-py3-none-any.whl", hash = "sha256:e80025850eafa8760055fd6f2f6e83f84bf13d4a844fe81abb2b499e3a3e8af0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }

        [package.optional-dependencies]
        foo = [
            { name = "anyio" },
            { name = "idna", version = "3.5", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]

        [package.dev-dependencies]
        bar = [
            { name = "anyio" },
            { name = "idna", version = "3.6", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]

        [package.metadata]
        requires-dist = [
            { name = "anyio", marker = "extra == 'foo'" },
            { name = "idna", marker = "extra == 'foo'", specifier = "==3.5" },
        ]
        provides-extras = ["foo"]

        [package.metadata.requires-dev]
        bar = [
            { name = "anyio" },
            { name = "idna", specifier = "==3.6" },
        ]

        [[package]]
        name = "sniffio"
        version = "1.3.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sniffio-1.3.1.tar.gz", hash = "sha256:ce520d2eb3c2be02f0c148dab5ba304e8705e1c7e7b4bec8a9146c464a597a6a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sniffio-1.3.1-py3-none-any.whl", hash = "sha256:2743fa2a853c508a2310882c0b4104631e0b0fcb855e00a912b7e3f27e6b3f05", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    Ok(())
}

/// Like `shared_optional_dependency_extra1`, but puts the dependent
/// in the list of production dependencies instead of as an optional
/// dependency.
///
/// Regression test for: <https://github.com/astral-sh/uv/issues/9289>
#[test]
fn shared_dependency_extra() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["anyio"]

        [project.optional-dependencies]
        foo = [
          "idna==3.5",
        ]
        bar = [
          "idna==3.6",
        ]

        [tool.uv]
        conflicts = [
          [
            { extra = "foo" },
            { extra = "bar" },
          ],
        ]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.sync(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    let lock = fs_err::read_to_string(context.temp_dir.join("uv.lock")).unwrap();
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        conflicts = [[
            { package = "project", extra = "bar" },
            { package = "project", extra = "foo" },
        ]]

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "anyio"
        version = "4.3.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "idna", version = "3.5", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'extra-7-project-foo'" },
            { name = "idna", version = "3.6", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'extra-7-project-bar' or extra != 'extra-7-project-foo'" },
            { name = "sniffio" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/anyio-4.3.0.tar.gz", hash = "sha256:13a6d97fa30ec110d85e3949a30c92306f0178135048329f54a335c3dade753a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/anyio-4.3.0-py3-none-any.whl", hash = "sha256:c4f443e7e5a2c003b1534688207e85dbd11960efb66d4d6a4e7693fdfc6f5b33", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.5"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.5.tar.gz", hash = "sha256:b52e0d5cd15ab27c7a8fc6d6567950aea07184f1982eb244a816febdf79845d8", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.5-py3-none-any.whl", hash = "sha256:4331936a57b12ca883491bf69b71d44c498a77aed0af0b2b1e28f1b5d0706f88", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.6"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.6.tar.gz", hash = "sha256:9aae8f72192b28db0d56fcef130afe490d1538a8d1bf1700e6d219521421525f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.6-py3-none-any.whl", hash = "sha256:e80025850eafa8760055fd6f2f6e83f84bf13d4a844fe81abb2b499e3a3e8af0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "anyio" },
        ]

        [package.optional-dependencies]
        bar = [
            { name = "idna", version = "3.6", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        foo = [
            { name = "idna", version = "3.5", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]

        [package.metadata]
        requires-dist = [
            { name = "anyio" },
            { name = "idna", marker = "extra == 'bar'", specifier = "==3.6" },
            { name = "idna", marker = "extra == 'foo'", specifier = "==3.5" },
        ]
        provides-extras = ["foo", "bar"]

        [[package]]
        name = "sniffio"
        version = "1.3.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sniffio-1.3.1.tar.gz", hash = "sha256:ce520d2eb3c2be02f0c148dab5ba304e8705e1c7e7b4bec8a9146c464a597a6a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sniffio-1.3.1-py3-none-any.whl", hash = "sha256:2743fa2a853c508a2310882c0b4104631e0b0fcb855e00a912b7e3f27e6b3f05", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // This shouldn't install two versions of `idna`, only one, `idna==3.5`.
    // So this should remove `idna==3.6` installed above.
    uv_snapshot!(context.filters(), context.sync().arg("--extra=foo"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - idna==3.6
     + idna==3.5
    ");

    uv_snapshot!(context.filters(), context.sync().arg("--extra=bar"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - idna==3.5
     + idna==3.6
    ");

    uv_snapshot!(context.filters(), context.sync(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Checked 3 packages in [TIME]
    ");

    Ok(())
}

/// Like `shared_dependency_extra`, but for groups.
///
/// Regression test for: <https://github.com/astral-sh/uv/issues/9289>
#[test]
fn shared_dependency_group() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["anyio"]

        [dependency-groups]
        foo = [
          "idna==3.5",
        ]
        bar = [
          "idna==3.6",
        ]

        [tool.uv]
        conflicts = [
          [
            { group = "foo" },
            { group = "bar" },
          ],
        ]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.sync(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    let lock = fs_err::read_to_string(context.temp_dir.join("uv.lock")).unwrap();
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        conflicts = [[
            { package = "project", group = "bar" },
            { package = "project", group = "foo" },
        ]]

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "anyio"
        version = "4.3.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "idna", version = "3.5", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'group-7-project-foo'" },
            { name = "idna", version = "3.6", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'group-7-project-bar' or extra != 'group-7-project-foo'" },
            { name = "sniffio" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/anyio-4.3.0.tar.gz", hash = "sha256:13a6d97fa30ec110d85e3949a30c92306f0178135048329f54a335c3dade753a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/anyio-4.3.0-py3-none-any.whl", hash = "sha256:c4f443e7e5a2c003b1534688207e85dbd11960efb66d4d6a4e7693fdfc6f5b33", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.5"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.5.tar.gz", hash = "sha256:b52e0d5cd15ab27c7a8fc6d6567950aea07184f1982eb244a816febdf79845d8", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.5-py3-none-any.whl", hash = "sha256:4331936a57b12ca883491bf69b71d44c498a77aed0af0b2b1e28f1b5d0706f88", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.6"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.6.tar.gz", hash = "sha256:9aae8f72192b28db0d56fcef130afe490d1538a8d1bf1700e6d219521421525f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.6-py3-none-any.whl", hash = "sha256:e80025850eafa8760055fd6f2f6e83f84bf13d4a844fe81abb2b499e3a3e8af0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "anyio" },
        ]

        [package.dev-dependencies]
        bar = [
            { name = "idna", version = "3.6", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        foo = [
            { name = "idna", version = "3.5", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]

        [package.metadata]
        requires-dist = [{ name = "anyio" }]

        [package.metadata.requires-dev]
        bar = [{ name = "idna", specifier = "==3.6" }]
        foo = [{ name = "idna", specifier = "==3.5" }]

        [[package]]
        name = "sniffio"
        version = "1.3.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sniffio-1.3.1.tar.gz", hash = "sha256:ce520d2eb3c2be02f0c148dab5ba304e8705e1c7e7b4bec8a9146c464a597a6a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sniffio-1.3.1-py3-none-any.whl", hash = "sha256:2743fa2a853c508a2310882c0b4104631e0b0fcb855e00a912b7e3f27e6b3f05", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // This shouldn't install two versions of `idna`, only one, `idna==3.5`.
    // So this should remove `idna==3.6` installed above.
    uv_snapshot!(context.filters(), context.sync().arg("--group=foo"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - idna==3.6
     + idna==3.5
    ");

    uv_snapshot!(context.filters(), context.sync().arg("--group=bar"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - idna==3.5
     + idna==3.6
    ");

    uv_snapshot!(context.filters(), context.sync(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Checked 3 packages in [TIME]
    ");

    Ok(())
}

/// Like `shared_dependency_extra`, but for extras/groups.
///
/// Regression test for: <https://github.com/astral-sh/uv/issues/9289>
#[test]
fn shared_dependency_mixed() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["anyio"]

        [project.optional-dependencies]
        foo = [
          "idna==3.5",
        ]

        [dependency-groups]
        bar = [
          "idna==3.6",
        ]

        [tool.uv]
        conflicts = [
          [
            { extra = "foo" },
            { group = "bar" },
          ],
        ]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.sync(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    let lock = fs_err::read_to_string(context.temp_dir.join("uv.lock")).unwrap();
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        conflicts = [[
            { package = "project", extra = "foo" },
            { package = "project", group = "bar" },
        ]]

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "anyio"
        version = "4.3.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "idna", version = "3.5", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'extra-7-project-foo'" },
            { name = "idna", version = "3.6", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'group-7-project-bar' or extra != 'extra-7-project-foo'" },
            { name = "sniffio" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/anyio-4.3.0.tar.gz", hash = "sha256:13a6d97fa30ec110d85e3949a30c92306f0178135048329f54a335c3dade753a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/anyio-4.3.0-py3-none-any.whl", hash = "sha256:c4f443e7e5a2c003b1534688207e85dbd11960efb66d4d6a4e7693fdfc6f5b33", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.5"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.5.tar.gz", hash = "sha256:b52e0d5cd15ab27c7a8fc6d6567950aea07184f1982eb244a816febdf79845d8", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.5-py3-none-any.whl", hash = "sha256:4331936a57b12ca883491bf69b71d44c498a77aed0af0b2b1e28f1b5d0706f88", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.6"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.6.tar.gz", hash = "sha256:9aae8f72192b28db0d56fcef130afe490d1538a8d1bf1700e6d219521421525f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.6-py3-none-any.whl", hash = "sha256:e80025850eafa8760055fd6f2f6e83f84bf13d4a844fe81abb2b499e3a3e8af0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "anyio" },
        ]

        [package.optional-dependencies]
        foo = [
            { name = "idna", version = "3.5", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]

        [package.dev-dependencies]
        bar = [
            { name = "idna", version = "3.6", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]

        [package.metadata]
        requires-dist = [
            { name = "anyio" },
            { name = "idna", marker = "extra == 'foo'", specifier = "==3.5" },
        ]
        provides-extras = ["foo"]

        [package.metadata.requires-dev]
        bar = [{ name = "idna", specifier = "==3.6" }]

        [[package]]
        name = "sniffio"
        version = "1.3.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sniffio-1.3.1.tar.gz", hash = "sha256:ce520d2eb3c2be02f0c148dab5ba304e8705e1c7e7b4bec8a9146c464a597a6a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sniffio-1.3.1-py3-none-any.whl", hash = "sha256:2743fa2a853c508a2310882c0b4104631e0b0fcb855e00a912b7e3f27e6b3f05", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // This shouldn't install two versions of `idna`, only one, `idna==3.5`.
    // So this should remove `idna==3.6` installed above.
    uv_snapshot!(context.filters(), context.sync().arg("--extra=foo"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - idna==3.6
     + idna==3.5
    ");

    uv_snapshot!(context.filters(), context.sync().arg("--group=bar"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - idna==3.5
     + idna==3.6
    ");

    uv_snapshot!(context.filters(), context.sync(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Checked 3 packages in [TIME]
    ");

    Ok(())
}

/// This test ensures that when `--extra=foo` is given in the CLI, it is
/// appropriately namespaced to the correct package. That is, it doesn't
/// erroneously enable _every_ extra named `foo`, but only the top-level extra
/// named `foo`.
///
/// This isn't a regression test from `main`, but is a regression test for a
/// bug found in ongoing work. (Where I wasn't properly namespacing extras with
/// their corresponding package names.)
///
/// Ref <https://github.com/astral-sh/uv/issues/9289>
#[test]
fn extras_are_namespaced() -> Result<()> {
    let context = uv_test::test_context!("3.11");

    let root_pyproject_toml = context.temp_dir.child("pyproject.toml");
    root_pyproject_toml.write_str(
        r#"
[project]
name = "project"
version = "0.1.0"
requires-python = ">=3.11,<3.12"
dependencies = [
  "proxy1",
  "anyio>=4",
]

[tool.uv.workspace]
members = ["proxy1"]

[project.optional-dependencies]
x1 = ["idna==3.6"]

[tool.uv.sources]
proxy1 = { workspace = true }

[tool.uv]
conflicts = [
  [
    {package = "project", extra = "x1"},
    {package = "proxy1", extra = "x2"},
    {package = "proxy1", extra = "x3"},
  ],
]
        "#,
    )?;

    let proxy1_pyproject_toml = context.temp_dir.child("proxy1").child("pyproject.toml");
    proxy1_pyproject_toml.write_str(
        r#"
        [project]
        name = "proxy1"
        version = "0.1.0"
        requires-python = ">=3.11,<3.12"
        dependencies = []

        [project.optional-dependencies]
        x2 = ["idna==3.4"]
        x3 = ["idna==3.5"]
        "#,
    )?;

    // Error out, as x2 extra is only on the child.
    uv_snapshot!(context.filters(), context.sync().arg("--extra=x2"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 7 packages in [TIME]
    error: Extra `x2` is not defined in the `optional-dependencies` table for `project`
    ");

    uv_snapshot!(context.filters(), context.sync(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 7 packages in [TIME]
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + proxy1==0.1.0 (from file://[TEMP_DIR]/proxy1)
     + sniffio==1.3.1
    ");

    let lock = fs_err::read_to_string(context.temp_dir.join("uv.lock")).unwrap();
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = "==3.11.*"
        conflicts = [[
            { package = "project", extra = "x1" },
            { package = "proxy1", extra = "x2" },
            { package = "proxy1", extra = "x3" },
        ]]

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [manifest]
        members = [
            "project",
            "proxy1",
        ]

        [[package]]
        name = "anyio"
        version = "4.3.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "idna", version = "3.4", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'extra-6-proxy1-x2' or (extra == 'extra-6-proxy1-x3' and extra == 'extra-7-project-x1')" },
            { name = "idna", version = "3.5", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'extra-6-proxy1-x3' or (extra == 'extra-6-proxy1-x2' and extra == 'extra-7-project-x1')" },
            { name = "idna", version = "3.6", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'extra-7-project-x1' or (extra == 'extra-6-proxy1-x2' and extra == 'extra-6-proxy1-x3') or (extra != 'extra-6-proxy1-x2' and extra != 'extra-6-proxy1-x3')" },
            { name = "sniffio" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/anyio-4.3.0.tar.gz", hash = "sha256:13a6d97fa30ec110d85e3949a30c92306f0178135048329f54a335c3dade753a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/anyio-4.3.0-py3-none-any.whl", hash = "sha256:c4f443e7e5a2c003b1534688207e85dbd11960efb66d4d6a4e7693fdfc6f5b33", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.4"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.4.tar.gz", hash = "sha256:fa04479ae8ef727bc1261e8db7e7347dbef7a50e56e7fa5d9de7b18b43ab6d42", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.4-py3-none-any.whl", hash = "sha256:f1f209736fbedfb02f5abdcd1d04682bee585684dc5a944a896d763a12322b77", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.5"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.5.tar.gz", hash = "sha256:b52e0d5cd15ab27c7a8fc6d6567950aea07184f1982eb244a816febdf79845d8", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.5-py3-none-any.whl", hash = "sha256:4331936a57b12ca883491bf69b71d44c498a77aed0af0b2b1e28f1b5d0706f88", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.6"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.6.tar.gz", hash = "sha256:9aae8f72192b28db0d56fcef130afe490d1538a8d1bf1700e6d219521421525f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.6-py3-none-any.whl", hash = "sha256:e80025850eafa8760055fd6f2f6e83f84bf13d4a844fe81abb2b499e3a3e8af0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "anyio" },
            { name = "proxy1" },
        ]

        [package.optional-dependencies]
        x1 = [
            { name = "idna", version = "3.6", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]

        [package.metadata]
        requires-dist = [
            { name = "anyio", specifier = ">=4" },
            { name = "idna", marker = "extra == 'x1'", specifier = "==3.6" },
            { name = "proxy1", editable = "proxy1" },
        ]
        provides-extras = ["x1"]

        [[package]]
        name = "proxy1"
        version = "0.1.0"
        source = { editable = "proxy1" }

        [package.optional-dependencies]
        x2 = [
            { name = "idna", version = "3.4", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        x3 = [
            { name = "idna", version = "3.5", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]

        [package.metadata]
        requires-dist = [
            { name = "idna", marker = "extra == 'x2'", specifier = "==3.4" },
            { name = "idna", marker = "extra == 'x3'", specifier = "==3.5" },
        ]
        provides-extras = ["x2", "x3"]

        [[package]]
        name = "sniffio"
        version = "1.3.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sniffio-1.3.1.tar.gz", hash = "sha256:ce520d2eb3c2be02f0c148dab5ba304e8705e1c7e7b4bec8a9146c464a597a6a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sniffio-1.3.1-py3-none-any.whl", hash = "sha256:2743fa2a853c508a2310882c0b4104631e0b0fcb855e00a912b7e3f27e6b3f05", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    Ok(())
}

/// This tests a case I stumbled on while working on [1] where conflict markers
/// were written when they didn't need to be.
///
/// That is, the conflict markers written here were correct, but redundant with
/// the fact that `cu118` and `cu124` could not be enabled simultaneously.
/// In other words, this is a regression test for the conflict marker
/// simplification I did as part of fixing [1].
///
/// [1]: <https://github.com/astral-sh/uv/issues/9289>
#[test]
fn jinja_no_conflict_markers1() -> Result<()> {
    let cu118_index =
        uv_test::packse::PackseServer::new("packages/sync-multiple-sources-index.toml");
    let cu124_index =
        uv_test::packse::PackseServer::new("packages/sync-multiple-sources-index.toml");
    let cu118_index_url = cu118_index.index_url();
    let cu124_index_url = cu124_index.index_url();
    let context = uv_test::test_context!("3.12")
        .with_packse_index("packages/sync-multiple-sources-index.toml")
        .with_exclude_newer("2025-01-30T00:00Z");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.optional-dependencies]
        cu118 = ["jinja2[i18n]==3.1.2"]
        cu124 = ["jinja2[i18n]==3.1.3"]

        [tool.uv]
        constraint-dependencies = ["markupsafe<3"]
        conflicts = [
            [
                {{ extra = "cu118" }},
                {{ extra = "cu124" }},
            ],
        ]

        [tool.uv.sources]
        jinja2 = [
            {{ index = "torch-cu118", extra = "cu118" }},
            {{ index = "torch-cu124", extra = "cu124" }},
        ]

        [[tool.uv.index]]
        name = "torch-cu118"
        url = "{cu118_index_url}"
        explicit = true

        [[tool.uv.index]]
        name = "torch-cu124"
        url = "{cu124_index_url}"
        explicit = true
        "#})?;

    uv_snapshot!(context.filters(), context.sync(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Checked in [TIME]
    ");

    let lock = fs_err::read_to_string(context.temp_dir.join("uv.lock")).unwrap();
    let mut lock_filters = vec![
        (cu118_index_url.as_str(), "http://[TORCH-CU118]/simple/"),
        (cu124_index_url.as_str(), "http://[TORCH-CU124]/simple/"),
    ];
    lock_filters.extend(context.filters());

    insta::with_settings!({
        filters => lock_filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        conflicts = [[
            { package = "project", extra = "cu118" },
            { package = "project", extra = "cu124" },
        ]]

        [options]
        exclude-newer = "2025-01-30T00:00:00Z"

        [manifest]
        constraints = [{ name = "markupsafe", specifier = "<3" }]

        [[package]]
        name = "babel"
        version = "2.16.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/babel-2.16.0.tar.gz", hash = "sha256:7ea36c622fb6f152022de1cc793821d621f592a6e36c44f9ac837dfc75ac8de2", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/babel-2.16.0-py3-none-any.whl", hash = "sha256:f710f834a3355f4d7edb7678c44a621c28e614bed6614c04d7e2a72e119a212c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "jinja2"
        version = "3.1.2"
        source = { registry = "http://[TORCH-CU118]/simple/" }
        dependencies = [
            { name = "markupsafe" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/jinja2-3.1.2.tar.gz", hash = "sha256:0bcc298fd1dec3c794f9c2297e92aca84212ba7fab2d4a07c16d57f9109c25df", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/jinja2-3.1.2-py3-none-any.whl", hash = "sha256:9df664d9fbd96dc6d4c1d513e82f141a80ff7713e1a46144a38d45984bdfd62c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [package.optional-dependencies]
        i18n = [
            { name = "babel" },
        ]

        [[package]]
        name = "jinja2"
        version = "3.1.3"
        source = { registry = "http://[TORCH-CU124]/simple/" }
        dependencies = [
            { name = "markupsafe" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/jinja2-3.1.3.tar.gz", hash = "sha256:b2ececb7e299649721da5cfb6e178bdc3b4aeef39ac8bdc791bf52d9b68c1bba", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/jinja2-3.1.3-py3-none-any.whl", hash = "sha256:26ae100d7dfd6c1e4c532b44f127adfcc5b836f87b9643af33fc01e2406556bb", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [package.optional-dependencies]
        i18n = [
            { name = "babel" },
        ]

        [[package]]
        name = "markupsafe"
        version = "2.1.5"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/markupsafe-2.1.5.tar.gz", hash = "sha256:e38237d66e6760fe86fe38e4b6c70ff4eed7da50b15e8c0f2589f05850207f13", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/markupsafe-2.1.5-py3-none-any.whl", hash = "sha256:d0fe66b2745bbd943b48f3229667cf4506d88136363578b94e63d384e61d4984", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }

        [package.optional-dependencies]
        cu118 = [
            { name = "jinja2", version = "3.1.2", source = { registry = "http://[TORCH-CU118]/simple/" }, extra = ["i18n"], marker = "extra == 'extra-7-project-cu118'" },
        ]
        cu124 = [
            { name = "jinja2", version = "3.1.3", source = { registry = "http://[TORCH-CU124]/simple/" }, extra = ["i18n"], marker = "extra == 'extra-7-project-cu124'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "jinja2", extras = ["i18n"], marker = "extra == 'cu118'", specifier = "==3.1.2", index = "http://[TORCH-CU118]/simple/", conflict = { package = "project", extra = "cu118" } },
            { name = "jinja2", extras = ["i18n"], marker = "extra == 'cu124'", specifier = "==3.1.3", index = "http://[TORCH-CU124]/simple/", conflict = { package = "project", extra = "cu124" } },
        ]
        provides-extras = ["cu118", "cu124"]
        "#
        );
    });

    Ok(())
}

/// Like `jinja_no_conflict_markers1`, but includes a PEP 508 marker
/// to spice things up. As with `jinja_no_conflict_markers1`, we
/// shouldn't see any conflict markers in the lock file here.
#[test]
fn jinja_no_conflict_markers2() -> Result<()> {
    let mut indexes = [
        PackseServer::new("packages/sync-multiple-sources-index.toml"),
        PackseServer::new("packages/sync-multiple-sources-index.toml"),
        PackseServer::new("packages/sync-multiple-sources-index.toml"),
    ];
    // Registry URLs break ties between identical package versions in the lockfile.
    indexes.sort_by_key(PackseServer::index_url);
    let [default_index, cu118_index, cu124_index] = indexes;
    let cu118_index_url = cu118_index.index_url();
    let cu124_index_url = cu124_index.index_url();
    let context = uv_test::test_context!("3.12")
        .with_default_index(&default_index.index_url())
        .with_exclude_newer("2025-01-30T00:00Z");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.optional-dependencies]
        cu118 = ["jinja2==3.1.2"]
        cu124 = ["jinja2==3.1.3"]

        [tool.uv]
        constraint-dependencies = ["markupsafe<3"]
        conflicts = [
            [
                {{ extra = "cu118" }},
                {{ extra = "cu124" }},
            ],
        ]

        [tool.uv.sources]
        jinja2 = [
            {{ index = "torch-cu118", extra = "cu118", marker = "sys_platform == 'darwin'" }},
            {{ index = "torch-cu124", extra = "cu124" }},
        ]

        [[tool.uv.index]]
        name = "torch-cu118"
        url = "{cu118_index_url}"
        explicit = true

        [[tool.uv.index]]
        name = "torch-cu124"
        url = "{cu124_index_url}"
        explicit = true
        "#})?;

    uv_snapshot!(context.filters(), context.sync(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Checked in [TIME]
    ");

    let lock = fs_err::read_to_string(context.temp_dir.join("uv.lock")).unwrap();
    let mut lock_filters = vec![
        (cu118_index_url.as_str(), "http://[TORCH-CU118]/simple/"),
        (cu124_index_url.as_str(), "http://[TORCH-CU124]/simple/"),
    ];
    lock_filters.extend(context.filters());

    insta::with_settings!({
        filters => lock_filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        resolution-markers = [
            "extra != 'extra-7-project-cu118' and extra == 'extra-7-project-cu124'",
            "sys_platform == 'darwin' and extra == 'extra-7-project-cu118' and extra != 'extra-7-project-cu124'",
            "sys_platform != 'darwin' and extra == 'extra-7-project-cu118' and extra != 'extra-7-project-cu124'",
            "extra != 'extra-7-project-cu118' and extra != 'extra-7-project-cu124'",
        ]
        conflicts = [[
            { package = "project", extra = "cu118" },
            { package = "project", extra = "cu124" },
        ]]

        [options]
        exclude-newer = "2025-01-30T00:00:00Z"

        [manifest]
        constraints = [{ name = "markupsafe", specifier = "<3" }]

        [[package]]
        name = "jinja2"
        version = "3.1.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform != 'darwin'",
        ]
        dependencies = [
            { name = "markupsafe", marker = "sys_platform != 'darwin'" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/jinja2-3.1.2.tar.gz", hash = "sha256:0bcc298fd1dec3c794f9c2297e92aca84212ba7fab2d4a07c16d57f9109c25df", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/jinja2-3.1.2-py3-none-any.whl", hash = "sha256:9df664d9fbd96dc6d4c1d513e82f141a80ff7713e1a46144a38d45984bdfd62c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "jinja2"
        version = "3.1.2"
        source = { registry = "http://[TORCH-CU118]/simple/" }
        resolution-markers = [
            "sys_platform == 'darwin'",
        ]
        dependencies = [
            { name = "markupsafe", marker = "sys_platform == 'darwin'" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/jinja2-3.1.2.tar.gz", hash = "sha256:0bcc298fd1dec3c794f9c2297e92aca84212ba7fab2d4a07c16d57f9109c25df", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/jinja2-3.1.2-py3-none-any.whl", hash = "sha256:9df664d9fbd96dc6d4c1d513e82f141a80ff7713e1a46144a38d45984bdfd62c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "jinja2"
        version = "3.1.3"
        source = { registry = "http://[TORCH-CU124]/simple/" }
        dependencies = [
            { name = "markupsafe" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/jinja2-3.1.3.tar.gz", hash = "sha256:b2ececb7e299649721da5cfb6e178bdc3b4aeef39ac8bdc791bf52d9b68c1bba", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/jinja2-3.1.3-py3-none-any.whl", hash = "sha256:26ae100d7dfd6c1e4c532b44f127adfcc5b836f87b9643af33fc01e2406556bb", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "markupsafe"
        version = "2.1.5"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/markupsafe-2.1.5.tar.gz", hash = "sha256:e38237d66e6760fe86fe38e4b6c70ff4eed7da50b15e8c0f2589f05850207f13", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/markupsafe-2.1.5-py3-none-any.whl", hash = "sha256:d0fe66b2745bbd943b48f3229667cf4506d88136363578b94e63d384e61d4984", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }

        [package.optional-dependencies]
        cu118 = [
            { name = "jinja2", version = "3.1.2", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "(sys_platform != 'darwin' and extra == 'extra-7-project-cu118') or (extra == 'extra-7-project-cu118' and extra == 'extra-7-project-cu124')" },
            { name = "jinja2", version = "3.1.2", source = { registry = "http://[TORCH-CU118]/simple/" }, marker = "(sys_platform == 'darwin' and extra == 'extra-7-project-cu118') or (extra == 'extra-7-project-cu118' and extra == 'extra-7-project-cu124')" },
        ]
        cu124 = [
            { name = "jinja2", version = "3.1.3", source = { registry = "http://[TORCH-CU124]/simple/" } },
        ]

        [package.metadata]
        requires-dist = [
            { name = "jinja2", marker = "sys_platform == 'darwin' and extra == 'cu118'", specifier = "==3.1.2", index = "http://[TORCH-CU118]/simple/", conflict = { package = "project", extra = "cu118" } },
            { name = "jinja2", marker = "sys_platform != 'darwin' and extra == 'cu118'", specifier = "==3.1.2" },
            { name = "jinja2", marker = "extra == 'cu124'", specifier = "==3.1.3", index = "http://[TORCH-CU124]/simple/", conflict = { package = "project", extra = "cu124" } },
        ]
        provides-extras = ["cu118", "cu124"]
        "#
        );
    });

    Ok(())
}

/// This tests a somewhat pathological case where a package has an extra whose
/// name corresponds to uv's conflicting extra encoding of another extra. That
/// is, an extra `foo` and an extra `extra-3-pkg-foo`.
///
/// In theory these could collide and cause problems. But in practice, we don't
/// involve the `extra == "foo"` marker in the same places, I believe, as we do
/// `extra == "extra-3-pkg-foo"`.
///
/// Ref: <https://github.com/astral-sh/uv/pull/9370#discussion_r1876083284>
#[test]
fn collision_extra() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "pkg"
        version = "0.1.0"
        description = "Add your description here"
        readme = "README.md"
        requires-python = ">=3.12"
        dependencies = ["anyio"]

        [project.optional-dependencies]
        foo = ["idna==3.5"]
        bar = ["idna==3.6"]
        extra-3-pkg-foo = ["sortedcontainers>=2"]

        [tool.uv]
        conflicts = [
          [
            { extra = "foo" },
            { extra = "bar" },
          ],
        ]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    ");

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock,
            @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        conflicts = [[
            { package = "pkg", extra = "bar" },
            { package = "pkg", extra = "foo" },
        ]]

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "anyio"
        version = "4.3.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "idna", version = "3.5", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'extra-3-pkg-foo'" },
            { name = "idna", version = "3.6", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'extra-3-pkg-bar' or extra != 'extra-3-pkg-foo'" },
            { name = "sniffio" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/anyio-4.3.0.tar.gz", hash = "sha256:13a6d97fa30ec110d85e3949a30c92306f0178135048329f54a335c3dade753a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/anyio-4.3.0-py3-none-any.whl", hash = "sha256:c4f443e7e5a2c003b1534688207e85dbd11960efb66d4d6a4e7693fdfc6f5b33", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.5"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.5.tar.gz", hash = "sha256:b52e0d5cd15ab27c7a8fc6d6567950aea07184f1982eb244a816febdf79845d8", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.5-py3-none-any.whl", hash = "sha256:4331936a57b12ca883491bf69b71d44c498a77aed0af0b2b1e28f1b5d0706f88", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.6"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.6.tar.gz", hash = "sha256:9aae8f72192b28db0d56fcef130afe490d1538a8d1bf1700e6d219521421525f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.6-py3-none-any.whl", hash = "sha256:e80025850eafa8760055fd6f2f6e83f84bf13d4a844fe81abb2b499e3a3e8af0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "pkg"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "anyio" },
        ]

        [package.optional-dependencies]
        bar = [
            { name = "idna", version = "3.6", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        extra-3-pkg-foo = [
            { name = "sortedcontainers" },
        ]
        foo = [
            { name = "idna", version = "3.5", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]

        [package.metadata]
        requires-dist = [
            { name = "anyio" },
            { name = "idna", marker = "extra == 'bar'", specifier = "==3.6" },
            { name = "idna", marker = "extra == 'foo'", specifier = "==3.5" },
            { name = "sortedcontainers", marker = "extra == 'extra-3-pkg-foo'", specifier = ">=2" },
        ]
        provides-extras = ["foo", "bar", "extra-3-pkg-foo"]

        [[package]]
        name = "sniffio"
        version = "1.3.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sniffio-1.3.1.tar.gz", hash = "sha256:ce520d2eb3c2be02f0c148dab5ba304e8705e1c7e7b4bec8a9146c464a597a6a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sniffio-1.3.1-py3-none-any.whl", hash = "sha256:2743fa2a853c508a2310882c0b4104631e0b0fcb855e00a912b7e3f27e6b3f05", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "sortedcontainers"
        version = "2.4.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sortedcontainers-2.4.0.tar.gz", hash = "sha256:fb6015d312cfaf15c65e0aeadac28191546cc25e48d41b126ecc322d5b116be2", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sortedcontainers-2.4.0-py3-none-any.whl", hash = "sha256:e90c0f20bb5c630bbf840e4ccc94441cd55ae8048cf55fc42df715d28b67cb80", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    // The extra `extra-3-pkg-foo` is meant to collide with the encoded
    // extra name generated by the extra `foo`. When `foo` is enabled,
    // we expect to see `idna==3.5`, but when `extra-3-pkg-foo` is enabled,
    // we don't. Instead, we should just see `anyio` and `sortedcontainers`
    // installed.
    uv_snapshot!(
        context.filters(),
        context.sync().arg("--frozen").arg("--extra=extra-3-pkg-foo"),
        @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + sortedcontainers==2.4.0
    "
    );

    // Verify that activating `foo` does result in `idna==3.5`.
    uv_snapshot!(
        context.filters(),
        context.sync().arg("--frozen").arg("--extra=foo"),
        @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Uninstalled 2 packages in [TIME]
    Installed 1 package in [TIME]
     - idna==3.6
     + idna==3.5
     - sortedcontainers==2.4.0
    "
    );

    // And that activating both is fine and dandy. We get `idna==3.5`
    // and `sortedcontainers`.
    uv_snapshot!(
        context.filters(),
        context.sync().arg("--frozen").arg("--extra=extra-3-pkg-foo").arg("--extra=foo"),
        @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed 1 package in [TIME]
     + sortedcontainers==2.4.0
    "
    );

    Ok(())
}

/// This tests that uv's graph traversal to determine which extras are always
/// enabled is working properly. In particular, this catches a regression
/// Konsti found on the initial work adding conflict markers where not all
/// extras/groups would be propagated through the graph traversal.
///
/// In this case, there are a number of conflict markers that are removed
/// entirely from the lock file of this particular case.
///
/// Ref: <https://github.com/astral-sh/uv/pull/9370#discussion_r1875958904>
#[test]
fn extra_inferences() -> Result<()> {
    let context =
        uv_test::test_context!("3.12").with_packse_index("packages/extra-inferences.toml");

    context.temp_dir.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "pkg"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["inference-root==1.0.2"]

        [project.optional-dependencies]
        x1 = ["inference-engine==2.5.0"]
        x2 = ["inference-engine==2.6.0"]

        [tool.uv]
        conflicts = [[{ extra = "x1" }, { extra = "x2" }]]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 13 packages in [TIME]
    ");

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock,
            @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        conflicts = [[
            { package = "pkg", extra = "x1" },
            { package = "pkg", extra = "x2" },
        ]]

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "inference-common"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/inference_common-1.0.0.tar.gz", hash = "sha256:d667939bf8e56c38b9d9de5ca6a176cb641c5b9989572fa0e07ba74629eb1a56", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/inference_common-1.0.0-py3-none-any.whl", hash = "sha256:af42e334d8e9d002d8e2c034ea78a3b89f97287968083a6aacf5c4df78a3895d", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "inference-convergence"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "inference-common", marker = "extra == 'extra-3-pkg-x1'" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/inference_convergence-1.0.0.tar.gz", hash = "sha256:342c006537d1ed338c43fd948795464b01063b75b318f957bb88614e0c9890fc", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/inference_convergence-1.0.0-py3-none-any.whl", hash = "sha256:a57d88f838c0640e33ca6bbf45cc80864371eaae5f15e71cc85572bf025d67a4", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "inference-convergence"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "inference-common" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/inference_convergence-2.0.0.tar.gz", hash = "sha256:0177ce1fee900c164aee0268cea0a9eab42b0f56b1a2c0de27660f31cbe34f84", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/inference_convergence-2.0.0-py3-none-any.whl", hash = "sha256:4ec3ef098f8343430dfff1e222dad08469931753e514a282af506fcd19adca4a", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "inference-direct"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "inference-convergence", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'extra-3-pkg-x1'" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/inference_direct-1.0.0.tar.gz", hash = "sha256:1a4911f4457a50a1050a1208bce0d0093e8200cc56f7d09eceb47bbb96b2fa98", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/inference_direct-1.0.0-py3-none-any.whl", hash = "sha256:362e49b87b6d9e40ddf5f5eba3655865eb12d85a4e01ffb20962e711352bcf2a", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "inference-direct"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "inference-convergence", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        sdist = { url = "http://[LOCALHOST]/files/inference_direct-2.0.0.tar.gz", hash = "sha256:8ecda96930dfb9f6795ab0048324c802d889d8995825365189be6193ff9bae0f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/inference_direct-2.0.0-py3-none-any.whl", hash = "sha256:bee6e57d821b43f8760ec97d33b12e3956f5b3afb74bb820d1c722b15dc36f9d", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "inference-engine"
        version = "2.5.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "inference-direct", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'extra-3-pkg-x1'" },
            { name = "inference-indirect", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'extra-3-pkg-x1'" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/inference_engine-2.5.0.tar.gz", hash = "sha256:3d8c26c6115c515075238f79ba9e28f34a4a57f301cc32107929168ad7c1ce7c", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/inference_engine-2.5.0-py3-none-any.whl", hash = "sha256:c93173fdea04cdbc5ebbdd5f0fa574878c8c78d60820881ce32eadfb310d0338", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "inference-engine"
        version = "2.6.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "inference-direct", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" } },
            { name = "inference-indirect", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        sdist = { url = "http://[LOCALHOST]/files/inference_engine-2.6.0.tar.gz", hash = "sha256:b93fa13495f3ca2bfc1f777cdd34860a7c7ef639e02bc9940ee3b9a49871f75d", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/inference_engine-2.6.0-py3-none-any.whl", hash = "sha256:5e13bde56b68fe3350348ae7688171c922c97ddfa5fa47aa367ed35e8c0d2feb", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "inference-indirect"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "inference-middle", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'extra-3-pkg-x1'" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/inference_indirect-1.0.0.tar.gz", hash = "sha256:e1bccbfa9808bb1daf7828252d7fc2da1ca294e89c81cd8a6ece5d85f5421da1", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/inference_indirect-1.0.0-py3-none-any.whl", hash = "sha256:7faf78d79793d97d1c2baa51a3d0b3780ab6c9930e59966996bba4f2d90651a0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "inference-indirect"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "inference-middle", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        sdist = { url = "http://[LOCALHOST]/files/inference_indirect-2.0.0.tar.gz", hash = "sha256:bd8091eeafb31c8ab7d56b92979217a156ed18a022b73f7afcf40be1697315f5", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/inference_indirect-2.0.0-py3-none-any.whl", hash = "sha256:fd4889edff3e5ad5d14b5e7c518ae4a91e0e9a7bdc116f03de7ac0318f9caa45", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "inference-middle"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "inference-convergence", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'extra-3-pkg-x1'" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/inference_middle-1.0.0.tar.gz", hash = "sha256:89f4d536c9185861cd27b59db3e5e3fffe94bb43a3204d1227b5fb894a112d3c", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/inference_middle-1.0.0-py3-none-any.whl", hash = "sha256:b47a7528bfe7de2502b1bad191b6987e4004504e11fc1802ceaf88a886f80ef5", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "inference-middle"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "inference-convergence", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        sdist = { url = "http://[LOCALHOST]/files/inference_middle-2.0.0.tar.gz", hash = "sha256:a7baa84aa9f26298776a4ae680b0c3a531796084ab270c00d22dc525f2fba5bc", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/inference_middle-2.0.0-py3-none-any.whl", hash = "sha256:03096d77783b55bd6d4f447c57526fbddd33482070ec96f34314009b11c0699b", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "inference-root"
        version = "1.0.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "inference-common" },
            { name = "inference-engine", version = "2.5.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'extra-3-pkg-x1'" },
            { name = "inference-engine", version = "2.6.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'extra-3-pkg-x2' or extra != 'extra-3-pkg-x1'" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/inference_root-1.0.2.tar.gz", hash = "sha256:8896b334dff8442c34a63b76514df3382f2b57266ecd1202ee79e418af7a81b1", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/inference_root-1.0.2-py3-none-any.whl", hash = "sha256:311cc7d1b4d7c42d02d2ce5ef3abf87cef6c766240101ef58223c62acc206b4d", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "pkg"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "inference-root" },
        ]

        [package.optional-dependencies]
        x1 = [
            { name = "inference-engine", version = "2.5.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        x2 = [
            { name = "inference-engine", version = "2.6.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]

        [package.metadata]
        requires-dist = [
            { name = "inference-engine", marker = "extra == 'x1'", specifier = "==2.5.0" },
            { name = "inference-engine", marker = "extra == 'x2'", specifier = "==2.6.0" },
            { name = "inference-root", specifier = "==1.0.2" },
        ]
        provides-extras = ["x1", "x2"]
        "#
        );
    });

    Ok(())
}

/// This is a regression test[1] for repeated resolution markers in the
/// lock file.
///
/// Before this test was fixed, the `resolution-markers` in the lock file
/// below looked like this:
///
/// ```text
/// resolution-markers = [
///     "sys_platform != 'linux'",
///     "sys_platform == 'linux'",
///     "sys_platform != 'linux'",
///     "sys_platform == 'linux'",
/// ]
/// ```
///
/// [1]: <https://github.com/astral-sh/uv/issues/9296>
#[test]
fn deduplicate_resolution_markers() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "pkg"
        version = "0.1.0"
        description = "Add your description here"
        readme = "README.md"
        requires-python = ">=3.12"
        dependencies = []

        [project.optional-dependencies]
        x1 = [
          "idna==3.5 ; sys_platform != 'linux'",
          "idna==3.6 ; sys_platform == 'linux'",
        ]
        x2 = [
          "markupsafe==2.0.0 ; sys_platform != 'linux'",
          "markupsafe==2.1.0 ; sys_platform == 'linux'",
        ]

        [tool.uv]
        conflicts = [
          [
            { extra = "x1" },
            { extra = "x2" },
          ],
        ]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    ");

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock,
            @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        resolution-markers = [
            "sys_platform != 'linux' and extra != 'extra-3-pkg-x1' and extra == 'extra-3-pkg-x2'",
            "sys_platform == 'linux' and extra != 'extra-3-pkg-x1' and extra == 'extra-3-pkg-x2'",
            "sys_platform != 'linux' and extra == 'extra-3-pkg-x1' and extra != 'extra-3-pkg-x2'",
            "sys_platform == 'linux' and extra == 'extra-3-pkg-x1' and extra != 'extra-3-pkg-x2'",
            "extra != 'extra-3-pkg-x1' and extra != 'extra-3-pkg-x2'",
        ]
        conflicts = [[
            { package = "pkg", extra = "x1" },
            { package = "pkg", extra = "x2" },
        ]]

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "idna"
        version = "3.5"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform != 'linux'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/idna-3.5.tar.gz", hash = "sha256:b52e0d5cd15ab27c7a8fc6d6567950aea07184f1982eb244a816febdf79845d8", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.5-py3-none-any.whl", hash = "sha256:4331936a57b12ca883491bf69b71d44c498a77aed0af0b2b1e28f1b5d0706f88", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.6"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform == 'linux'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/idna-3.6.tar.gz", hash = "sha256:9aae8f72192b28db0d56fcef130afe490d1538a8d1bf1700e6d219521421525f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.6-py3-none-any.whl", hash = "sha256:e80025850eafa8760055fd6f2f6e83f84bf13d4a844fe81abb2b499e3a3e8af0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "markupsafe"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform != 'linux'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/markupsafe-2.0.0.tar.gz", hash = "sha256:04145b6f20caefdbf02ccacca12eb1e9b95ca6a272a4e5634190ed3866136863", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/markupsafe-2.0.0-py3-none-any.whl", hash = "sha256:cfa1d4defffc6173b263dacd329662664e5062fd49025f24597e72ceb9de50b6", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "markupsafe"
        version = "2.1.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform == 'linux'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/markupsafe-2.1.0.tar.gz", hash = "sha256:91e4c40a0a8629301204ffd17f29baf62f7593709490429e9022f41448296370", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/markupsafe-2.1.0-py3-none-any.whl", hash = "sha256:d779b8c51e3aa71c96d85460f1c022f5261d26ea0ee2e28e2cca468e9ec63915", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "pkg"
        version = "0.1.0"
        source = { virtual = "." }

        [package.optional-dependencies]
        x1 = [
            { name = "idna", version = "3.5", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "(sys_platform != 'linux' and extra == 'extra-3-pkg-x1') or (extra == 'extra-3-pkg-x1' and extra == 'extra-3-pkg-x2')" },
            { name = "idna", version = "3.6", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "(sys_platform == 'linux' and extra == 'extra-3-pkg-x1') or (extra == 'extra-3-pkg-x1' and extra == 'extra-3-pkg-x2')" },
        ]
        x2 = [
            { name = "markupsafe", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "(sys_platform != 'linux' and extra == 'extra-3-pkg-x2') or (extra == 'extra-3-pkg-x1' and extra == 'extra-3-pkg-x2')" },
            { name = "markupsafe", version = "2.1.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "(sys_platform == 'linux' and extra == 'extra-3-pkg-x2') or (extra == 'extra-3-pkg-x1' and extra == 'extra-3-pkg-x2')" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "idna", marker = "sys_platform == 'linux' and extra == 'x1'", specifier = "==3.6" },
            { name = "idna", marker = "sys_platform != 'linux' and extra == 'x1'", specifier = "==3.5" },
            { name = "markupsafe", marker = "sys_platform == 'linux' and extra == 'x2'", specifier = "==2.1.0" },
            { name = "markupsafe", marker = "sys_platform != 'linux' and extra == 'x2'", specifier = "==2.0.0" },
        ]
        provides-extras = ["x1", "x2"]
        "#
        );
    });

    Ok(())
}

/// A regression test for a case where one could get multiple versions of
/// `torch` installed into the same environment.
///
/// This occurred because of a bug in conflict marker simplification, which in
/// turn lead to a buggy lock file: the unconditional path to `torchmetrics`
/// must contribute an empty set of activated extras when its `torch` dependency
/// is reached through either conflicting extra.
///
/// Ref: <https://github.com/astral-sh/uv/issues/11133>
#[test]
fn incorrect_extra_simplification_leads_to_multiple_torch_packages() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let scenario = toml::from_str::<Scenario>(
        r#"
        name = "multiple-torch-packages"

        [root]
        requires = ["core"]

        [expected]
        satisfiable = true

        [packages.core.versions."1.0.0"]
        requires = ["torchmetrics"]
        sdist = false

        [packages.chgnet.versions."1.0.0"]
        requires = ["torch==2.5.1"]
        sdist = false

        [packages.matgl.versions."1.0.0"]
        requires = ["torchmetrics", "torch==2.2.1"]
        sdist = false

        [packages.torchmetrics.versions."1.0.0"]
        requires = ["torch"]
        sdist = false

        [packages.torch.versions."2.2.1"]
        sdist = false

        [packages.torch.versions."2.5.1"]
        sdist = false
        "#,
    )?;
    let server = PackseServer::from_scenario(&scenario);

    context.temp_dir.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "test"
        version = "0.0.1"
        requires-python = ">=3.12"
        dependencies = ["core"]

        [project.optional-dependencies]
        chgnet = ["chgnet"]
        m3gnet = ["matgl"]

        [tool.uv]
        conflicts = [[
            { extra = "chgnet" },
            { extra = "m3gnet" },
        ]]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock().arg("--index-url").arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 7 packages in [TIME]
    ");

    // The `torchmetrics` edges must assign mutually exclusive markers to each `torch` version.
    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        conflicts = [[
            { package = "test", extra = "chgnet" },
            { package = "test", extra = "m3gnet" },
        ]]

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "chgnet"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "torch", version = "2.5.1", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/chgnet-1.0.0-py3-none-any.whl", hash = "sha256:0624e3b2ca2dce5fd1deaeb46f1b0874bcae3a52e6195e51b4170248d2da226e", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "core"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "torchmetrics" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/core-1.0.0-py3-none-any.whl", hash = "sha256:38af74a9c2b7884d0bd7790aa8381760ba83142cc5f4eaeb8569ddd5dddbc6dc", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "matgl"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "torch", version = "2.2.1", source = { registry = "http://[LOCALHOST]/simple/" } },
            { name = "torchmetrics" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/matgl-1.0.0-py3-none-any.whl", hash = "sha256:9cfc97251373de7d4bd796a607de50168a6856e8905f0be617f8cf5c08412bbe", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "test"
        version = "0.0.1"
        source = { virtual = "." }
        dependencies = [
            { name = "core" },
        ]

        [package.optional-dependencies]
        chgnet = [
            { name = "chgnet" },
        ]
        m3gnet = [
            { name = "matgl" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "chgnet", marker = "extra == 'chgnet'" },
            { name = "core" },
            { name = "matgl", marker = "extra == 'm3gnet'" },
        ]
        provides-extras = ["chgnet", "m3gnet"]

        [[package]]
        name = "torch"
        version = "2.2.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        wheels = [
            { url = "http://[LOCALHOST]/files/torch-2.2.1-py3-none-any.whl", hash = "sha256:066d97f76fad128af8fdebc9d89fdcdda734d7dbddd90734296982fde2cf732e", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "torch"
        version = "2.5.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        wheels = [
            { url = "http://[LOCALHOST]/files/torch-2.5.1-py3-none-any.whl", hash = "sha256:81e34ae72fccd7c18277d3e50095ef16c7b63669057524893e4cb8d33af9f6f7", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "torchmetrics"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "torch", version = "2.2.1", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'extra-4-test-m3gnet'" },
            { name = "torch", version = "2.5.1", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'extra-4-test-chgnet' or extra != 'extra-4-test-m3gnet'" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/torchmetrics-1.0.0-py3-none-any.whl", hash = "sha256:9379de50e5f490053ffe6300105c17b504f064ea66a5ac95321f1c559e7d08a7", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#);
    });

    // The `chgnet` extra must select only `torch==2.5.1`.
    uv_snapshot!(context.filters(), context.sync()
        .arg("--frozen")
        .arg("--extra")
        .arg("chgnet"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + chgnet==1.0.0
     + core==1.0.0
     + torch==2.5.1
     + torchmetrics==1.0.0
    ");

    // Switching to `m3gnet` must replace `torch==2.5.1` with `torch==2.2.1`.
    uv_snapshot!(context.filters(), context.sync()
        .arg("--frozen")
        .arg("--extra")
        .arg("m3gnet"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 2 packages in [TIME]
    Uninstalled 2 packages in [TIME]
    Installed 2 packages in [TIME]
     - chgnet==1.0.0
     + matgl==1.0.0
     - torch==2.5.1
     + torch==2.2.1
    ");

    Ok(())
}

/// A regression test for a case where duplicate `torch` and `sympy`
/// dependencies were being installed into the same environment.
///
/// This was a resolution bug, as the lock file generated overlapping conflict
/// markers for some of the dependency edges.
///
/// It turned out that the conflict marker simplification was using the wrong
/// inferences when simplifying. For an edge, it was using the source edge's
/// inferences instead of the target edge's inferences. In particular, the
/// non-conflicting `sevennet` extra can be combined with `m3gnet`, so neither
/// of the ambiguous `e3nn` edges may be simplified unconditionally.
///
/// Ref: <https://github.com/astral-sh/uv/issues/11479>
#[test]
fn duplicate_torch_and_sympy_because_of_wrong_inferences() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let scenario = toml::from_str::<Scenario>(
        r#"
        name = "duplicate-torch-and-sympy"

        [root]
        requires = ["core"]

        [expected]
        satisfiable = true

        [packages.core.versions."1.0.0"]
        requires = ["e3nn"]
        sdist = false

        [packages.e3nn.versions."1.0.0"]
        requires = ["torch", "sympy"]
        sdist = false

        [packages.sevenn.versions."1.0.0"]
        requires = ["e3nn"]
        sdist = false

        [packages.chgnet.versions."1.0.0"]
        requires = ["torch==2.5.1", "sympy==1.13.1"]
        sdist = false

        [packages.alignn.versions."1.0.0"]
        requires = ["torch==2.2.1", "sympy==1.13.3"]
        sdist = false

        [packages.matgl.versions."1.0.0"]
        requires = ["torch==2.2.1", "sympy==1.13.3"]
        sdist = false

        [packages.torch.versions."2.2.1"]
        sdist = false

        [packages.torch.versions."2.5.1"]
        sdist = false

        [packages.sympy.versions."1.13.1"]
        sdist = false

        [packages.sympy.versions."1.13.3"]
        sdist = false
        "#,
    )?;
    let server = PackseServer::from_scenario(&scenario);

    context.temp_dir.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "test"
        version = "0.0.1"
        requires-python = ">=3.12"
        dependencies = ["core"]

        [project.optional-dependencies]
        chgnet = ["chgnet"]
        sevennet = ["sevenn"]
        all = ["sevenn", "chgnet"]
        alignn = ["alignn"]
        m3gnet = ["matgl"]

        [tool.uv]
        conflicts = [
            [{ extra = "chgnet" }, { extra = "alignn" }],
            [{ extra = "chgnet" }, { extra = "m3gnet" }],
            [{ extra = "all" }, { extra = "alignn" }],
            [{ extra = "all" }, { extra = "m3gnet" }],
        ]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock().arg("--index-url").arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 11 packages in [TIME]
    ");

    // Each `e3nn` edge must select exactly one `torch` and one `sympy` version per extra.
    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        conflicts = [[
            { package = "test", extra = "alignn" },
            { package = "test", extra = "chgnet" },
        ], [
            { package = "test", extra = "chgnet" },
            { package = "test", extra = "m3gnet" },
        ], [
            { package = "test", extra = "alignn" },
            { package = "test", extra = "all" },
        ], [
            { package = "test", extra = "all" },
            { package = "test", extra = "m3gnet" },
        ]]

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "alignn"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "sympy", version = "1.13.3", source = { registry = "http://[LOCALHOST]/simple/" } },
            { name = "torch", version = "2.2.1", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/alignn-1.0.0-py3-none-any.whl", hash = "sha256:1c1fd15ac36dbc6fc89e7e99e4d3d6086d06e6915b3ea30f8cdd958018e2e7c0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "chgnet"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "sympy", version = "1.13.1", source = { registry = "http://[LOCALHOST]/simple/" } },
            { name = "torch", version = "2.5.1", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/chgnet-1.0.0-py3-none-any.whl", hash = "sha256:f231496bd503d80c43ed3525eec941327e9a311f1013a3572b5fe45f59e7d5a9", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "core"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "e3nn" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/core-1.0.0-py3-none-any.whl", hash = "sha256:40b05c6c46611936f86513f04683a669237aa79d5845417658ae8474b6eeb3f5", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "e3nn"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "sympy", version = "1.13.1", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'extra-4-test-all' or extra == 'extra-4-test-chgnet' or (extra != 'extra-4-test-alignn' and extra != 'extra-4-test-m3gnet')" },
            { name = "sympy", version = "1.13.3", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'extra-4-test-alignn' or extra == 'extra-4-test-m3gnet'" },
            { name = "torch", version = "2.2.1", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'extra-4-test-alignn' or extra == 'extra-4-test-m3gnet'" },
            { name = "torch", version = "2.5.1", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'extra-4-test-all' or extra == 'extra-4-test-chgnet' or (extra != 'extra-4-test-alignn' and extra != 'extra-4-test-m3gnet')" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/e3nn-1.0.0-py3-none-any.whl", hash = "sha256:7e918bb178360c8c131281e4ef728672d65dcc6dfdd03babdbb88fe45b986395", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "matgl"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "sympy", version = "1.13.3", source = { registry = "http://[LOCALHOST]/simple/" } },
            { name = "torch", version = "2.2.1", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/matgl-1.0.0-py3-none-any.whl", hash = "sha256:7a6ddbc39c025bb6ce9414f1c03bcfadc50ae6aacf4aaf5ca41302099063f0a7", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "sevenn"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "e3nn" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/sevenn-1.0.0-py3-none-any.whl", hash = "sha256:4faff02ab91311b6568b9409ed059bcedfc9f3cbd9db761e392d120a563b33a8", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "sympy"
        version = "1.13.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sympy-1.13.1-py3-none-any.whl", hash = "sha256:ee2e0d7edcdcf457864a9355f6c99e3bccd7e06b0f779224007d51740093d5ab", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "sympy"
        version = "1.13.3"
        source = { registry = "http://[LOCALHOST]/simple/" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sympy-1.13.3-py3-none-any.whl", hash = "sha256:24e81112441f7742f3f2d6ec9bb1dd26586bfa834ea664c73501e0a8bdaffda2", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "test"
        version = "0.0.1"
        source = { virtual = "." }
        dependencies = [
            { name = "core" },
        ]

        [package.optional-dependencies]
        alignn = [
            { name = "alignn" },
        ]
        all = [
            { name = "chgnet" },
            { name = "sevenn" },
        ]
        chgnet = [
            { name = "chgnet" },
        ]
        m3gnet = [
            { name = "matgl" },
        ]
        sevennet = [
            { name = "sevenn" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "alignn", marker = "extra == 'alignn'" },
            { name = "chgnet", marker = "extra == 'all'" },
            { name = "chgnet", marker = "extra == 'chgnet'" },
            { name = "core" },
            { name = "matgl", marker = "extra == 'm3gnet'" },
            { name = "sevenn", marker = "extra == 'all'" },
            { name = "sevenn", marker = "extra == 'sevennet'" },
        ]
        provides-extras = ["chgnet", "sevennet", "all", "alignn", "m3gnet"]

        [[package]]
        name = "torch"
        version = "2.2.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        wheels = [
            { url = "http://[LOCALHOST]/files/torch-2.2.1-py3-none-any.whl", hash = "sha256:066d97f76fad128af8fdebc9d89fdcdda734d7dbddd90734296982fde2cf732e", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "torch"
        version = "2.5.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        wheels = [
            { url = "http://[LOCALHOST]/files/torch-2.5.1-py3-none-any.whl", hash = "sha256:81e34ae72fccd7c18277d3e50095ef16c7b63669057524893e4cb8d33af9f6f7", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#);
    });

    // The `all` extra must select `torch==2.5.1` and `sympy==1.13.1`.
    uv_snapshot!(context.filters(), context.sync()
        .arg("--frozen")
        .arg("--extra")
        .arg("all"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 6 packages in [TIME]
    Installed 6 packages in [TIME]
     + chgnet==1.0.0
     + core==1.0.0
     + e3nn==1.0.0
     + sevenn==1.0.0
     + sympy==1.13.1
     + torch==2.5.1
    ");

    // `sevennet` and `m3gnet` can coexist and must select the other version of each package.
    uv_snapshot!(context.filters(), context.sync()
        .arg("--frozen")
        .arg("--extra")
        .arg("sevennet")
        .arg("--extra")
        .arg("m3gnet"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 3 packages in [TIME]
    Uninstalled 3 packages in [TIME]
    Installed 3 packages in [TIME]
     - chgnet==1.0.0
     + matgl==1.0.0
     - sympy==1.13.1
     + sympy==1.13.3
     - torch==2.5.1
     + torch==2.2.1
    ");

    Ok(())
}

#[test]
fn overlapping_resolution_markers() -> Result<()> {
    let cpu = uv_test::packse::PackseServer::new("packages/local-platform-cpu.toml");
    let context =
        uv_test::test_context!("3.10").with_packse_index("packages/local-platform-registry.toml");

    context.temp_dir.child("pyproject.toml").write_str(
        &r#"
        [project]
        name = "marker-project"
        version = "0.1.0"
        requires-python = "==3.10.*"
        dependencies = ["platform-monitor==0.17.6"]

        [project.optional-dependencies]
        cpu = ["platform-engine==2.2.2"]
        cu118 = ["platform-engine==2.2.2"]

        [tool.uv]
        conflicts = [[{ extra = "cpu" }, { extra = "cu118" }]]

        [tool.uv.sources]
        platform-engine = [
          { index = "cpu", extra = "cpu", marker = "platform_system != 'Darwin'" },
        ]

        [[tool.uv.index]]
        name = "cpu"
        url = "[CPU_INDEX]"
        explicit = true
        "#
        .replace("[CPU_INDEX]", &cpu.index_url()),
    )?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 7 packages in [TIME]
    ");

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock,
            @r#"
        version = 1
        revision = 3
        requires-python = "==3.10.*"
        resolution-markers = [
            "extra != 'extra-14-marker-project-cpu' and extra == 'extra-14-marker-project-cu118'",
            "(platform_machine != 'aarch64' and sys_platform == 'linux' and extra == 'extra-14-marker-project-cpu' and extra != 'extra-14-marker-project-cu118') or (platform_python_implementation != 'CPython' and sys_platform == 'linux' and extra == 'extra-14-marker-project-cpu' and extra != 'extra-14-marker-project-cu118') or (sys_platform != 'darwin' and sys_platform != 'linux' and extra == 'extra-14-marker-project-cpu' and extra != 'extra-14-marker-project-cu118')",
            "platform_machine == 'aarch64' and platform_python_implementation == 'CPython' and sys_platform == 'linux' and extra == 'extra-14-marker-project-cpu' and extra != 'extra-14-marker-project-cu118'",
            "sys_platform == 'darwin' and extra == 'extra-14-marker-project-cpu' and extra != 'extra-14-marker-project-cu118'",
            "extra != 'extra-14-marker-project-cpu' and extra != 'extra-14-marker-project-cu118'",
        ]
        conflicts = [[
            { package = "marker-project", extra = "cpu" },
            { package = "marker-project", extra = "cu118" },
        ]]

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "marker-project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "platform-monitor" },
        ]

        [package.optional-dependencies]
        cpu = [
            { name = "platform-engine", version = "2.2.2", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "(platform_machine == 'aarch64' and platform_python_implementation == 'CPython' and sys_platform == 'linux' and extra == 'extra-14-marker-project-cpu') or (platform_machine != 'aarch64' and extra == 'extra-14-marker-project-cpu' and extra == 'extra-14-marker-project-cu118') or (platform_python_implementation != 'CPython' and extra == 'extra-14-marker-project-cpu' and extra == 'extra-14-marker-project-cu118') or (sys_platform != 'linux' and extra == 'extra-14-marker-project-cpu' and extra == 'extra-14-marker-project-cu118')" },
            { name = "platform-engine", version = "2.2.2", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "(sys_platform == 'darwin' and extra == 'extra-14-marker-project-cpu') or (extra == 'extra-14-marker-project-cpu' and extra == 'extra-14-marker-project-cu118')" },
            { name = "platform-engine", version = "2.2.2+cpu", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "(platform_machine != 'aarch64' and sys_platform == 'linux' and extra == 'extra-14-marker-project-cpu') or (platform_python_implementation != 'CPython' and sys_platform == 'linux' and extra == 'extra-14-marker-project-cpu') or (sys_platform != 'darwin' and sys_platform != 'linux' and extra == 'extra-14-marker-project-cpu') or (sys_platform == 'darwin' and extra == 'extra-14-marker-project-cpu' and extra == 'extra-14-marker-project-cu118') or (sys_platform == 'linux' and extra == 'extra-14-marker-project-cpu' and extra == 'extra-14-marker-project-cu118')" },
        ]
        cu118 = [
            { name = "platform-engine", version = "2.2.2", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]

        [package.metadata]
        requires-dist = [
            { name = "platform-engine", marker = "sys_platform == 'darwin' and extra == 'cpu'", specifier = "==2.2.2" },
            { name = "platform-engine", marker = "sys_platform != 'darwin' and extra == 'cpu'", specifier = "==2.2.2", index = "http://[LOCALHOST]/simple/", conflict = { package = "marker-project", extra = "cpu" } },
            { name = "platform-engine", marker = "extra == 'cu118'", specifier = "==2.2.2" },
            { name = "platform-monitor", specifier = "==0.17.6" },
        ]
        provides-extras = ["cpu", "cu118"]

        [[package]]
        name = "platform-accelerator"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "platform-common", marker = "extra == 'extra-14-marker-project-cu118'" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/platform_accelerator-1.0.0.tar.gz", hash = "sha256:99cb70939efdcd8c9d62affdf838191667008883624650be195df7944d079a5b", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/platform_accelerator-1.0.0-py3-none-any.whl", hash = "sha256:3cbd0e242b7c7503fbcd0f3447ccbb37cc1bbc57587271c6697f1c1f0817cdb2", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "platform-common"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/platform_common-1.0.0.tar.gz", hash = "sha256:e47e38af2a9edda3b3ae7cffc51bfe27f8f2ab334853a1a7420652b71089f068", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/platform_common-1.0.0-py3-none-any.whl", hash = "sha256:4865a77f820cd56f78356bb6608f6462aa64562c764f52204eb66d0058efdf8c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "platform-engine"
        version = "2.2.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "platform_machine == 'aarch64' and platform_python_implementation == 'CPython' and sys_platform == 'linux'",
        ]
        dependencies = [
            { name = "platform-common", marker = "platform_machine == 'aarch64' and platform_python_implementation == 'CPython' and sys_platform == 'linux'" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/platform_engine-2.2.2-cp310-cp310-manylinux_2_17_aarch64.whl", hash = "sha256:1851623908186ef497f3e624dceef1efd9f2520998e1b1d8a48d357f286be123", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "platform-engine"
        version = "2.2.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "extra != 'extra-14-marker-project-cpu' and extra == 'extra-14-marker-project-cu118'",
            "sys_platform == 'darwin' and extra == 'extra-14-marker-project-cpu' and extra != 'extra-14-marker-project-cu118'",
        ]
        dependencies = [
            { name = "platform-accelerator", marker = "(platform_machine == 'x86_64' and sys_platform == 'linux' and extra == 'extra-14-marker-project-cu118') or (platform_machine != 'x86_64' and extra == 'extra-14-marker-project-cpu' and extra == 'extra-14-marker-project-cu118') or (sys_platform != 'linux' and extra == 'extra-14-marker-project-cpu' and extra == 'extra-14-marker-project-cu118')" },
            { name = "platform-common", marker = "(sys_platform != 'darwin' and extra == 'extra-14-marker-project-cu118') or (sys_platform == 'darwin' and extra == 'extra-14-marker-project-cpu') or (extra != 'extra-14-marker-project-cpu' and extra == 'extra-14-marker-project-cu118')" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/platform_engine-2.2.2-py3-none-macosx_10_9_x86_64.whl", hash = "sha256:565df48d30191d181ef4424d965f3b6aff69df3f3222f4f28108eb6419068912", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/platform_engine-2.2.2-py3-none-macosx_11_0_arm64.whl", hash = "sha256:92128833c1f4778f1b96efacd093e05e618133038db7a41f2bf6f3220a531da9", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/platform_engine-2.2.2-py3-none-manylinux_2_17_aarch64.whl", hash = "sha256:40a0d378de0520f37c9c6ec4f6fcb9e24718121d4d44e698a21e3501e0802816", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/platform_engine-2.2.2-py3-none-manylinux_2_17_x86_64.whl", hash = "sha256:214eb51b236ffae087d62f6d47da7e57b385fe6fa793820eb0f0053bae065a60", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/platform_engine-2.2.2-py3-none-win_amd64.whl", hash = "sha256:f56507cda4b3a4fc86ed714ef22a43c681077a08c9c1cc879cf25601c2bdd4ca", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "platform-engine"
        version = "2.2.2+cpu"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "(platform_machine != 'aarch64' and sys_platform == 'linux') or (platform_python_implementation != 'CPython' and sys_platform == 'linux') or (sys_platform != 'darwin' and sys_platform != 'linux')",
        ]
        dependencies = [
            { name = "platform-common", marker = "(platform_machine != 'aarch64' and sys_platform == 'linux') or (platform_python_implementation != 'CPython' and sys_platform == 'linux') or (sys_platform != 'darwin' and sys_platform != 'linux')" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/platform_engine-2.2.2+cpu-cp310-cp310-linux_x86_64.whl", hash = "sha256:e5f9fc6c9d92edd9c359541595aeb6b51aeec1d5cb8df4f4bcd1cf43a643b74c", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/platform_engine-2.2.2+cpu-cp310-cp310-win_amd64.whl", hash = "sha256:f5a7a0931ec7ce0d2746e6837ff222e657279c4fd842ce7a6ea1b056ce1a3c7d", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "platform-monitor"
        version = "0.17.6"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "platform-common" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/platform_monitor-0.17.6.tar.gz", hash = "sha256:8b72ca0528c202bdf10a843b83aa7942d22978764e0fe17f7085d684a6a3637f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/platform_monitor-0.17.6-py3-none-any.whl", hash = "sha256:d10cecb71c1344e9a945d13ee2350f3d0cc14dc49ffc8ec5e93c8d6259347192", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    Ok(())
}

/// Ref: <https://github.com/astral-sh/uv/issues/17732>
#[test]
fn conditional_sources_keep_default_platform_specific_transitive_dependencies() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_exclude_newer("2025-02-06T00:00Z");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "test-torch"
        version = "0.1.0"
        requires-python = "==3.12.*"
        dependencies = ["torch>=2.6.0,<2.7.0"]

        [project.optional-dependencies]
        cpu = ["torch>=2.6.0,<2.7.0"]
        cu124 = ["torch>=2.6.0,<2.7.0"]

        [tool.uv]
        conflicts = [
            [{ extra = "cpu" }, { extra = "cu124" }],
        ]

        [[tool.uv.index]]
        name = "pytorch-cu124"
        url = "https://astral-sh.github.io/pytorch-mirror/whl/cu124"
        default = true

        [[tool.uv.index]]
        name = "pytorch-cpu"
        url = "https://astral-sh.github.io/pytorch-mirror/whl/cpu"
        explicit = true

        [tool.uv.sources]
        torch = [
            { index = "pytorch-cpu", extra = "cpu" },
        ]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 27 packages in [TIME]
    ");

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock,
            @r#"
        version = 1
        revision = 3
        requires-python = "==3.12.*"
        resolution-markers = [
            "extra != 'extra-10-test-torch-cpu' and extra == 'extra-10-test-torch-cu124'",
            "sys_platform != 'darwin' and extra == 'extra-10-test-torch-cpu' and extra != 'extra-10-test-torch-cu124'",
            "sys_platform == 'darwin' and extra == 'extra-10-test-torch-cpu' and extra != 'extra-10-test-torch-cu124'",
            "extra != 'extra-10-test-torch-cpu' and extra != 'extra-10-test-torch-cu124'",
        ]
        conflicts = [[
            { package = "test-torch", extra = "cpu" },
            { package = "test-torch", extra = "cu124" },
        ]]

        [options]
        exclude-newer = "2025-02-06T00:00:00Z"

        [[package]]
        name = "filelock"
        version = "3.13.1"
        source = { registry = "https://astral-sh.github.io/pytorch-mirror/whl/cu124" }
        wheels = [
            { url = "https://download.pytorch.org/whl/filelock-3.13.1-py3-none-any.whl", upload-time = "2025-01-29T22:50:56.834Z" },
        ]

        [[package]]
        name = "fsspec"
        version = "2024.6.1"
        source = { registry = "https://astral-sh.github.io/pytorch-mirror/whl/cu124" }
        wheels = [
            { url = "https://download.pytorch.org/whl/fsspec-2024.6.1-py3-none-any.whl", upload-time = "2025-01-29T22:50:56.915Z" },
        ]

        [[package]]
        name = "jinja2"
        version = "3.1.4"
        source = { registry = "https://astral-sh.github.io/pytorch-mirror/whl/cu124" }
        dependencies = [
            { name = "markupsafe" },
        ]
        wheels = [
            { url = "https://download.pytorch.org/whl/Jinja2-3.1.4-py3-none-any.whl", upload-time = "2025-01-29T22:50:57.275Z" },
        ]

        [[package]]
        name = "markupsafe"
        version = "2.1.5"
        source = { registry = "https://astral-sh.github.io/pytorch-mirror/whl/cu124" }
        sdist = { url = "https://download.pytorch.org/whl/MarkupSafe-2.1.5.tar.gz", upload-time = "2025-01-29T22:50:57.539Z" }
        wheels = [
            { url = "https://download.pytorch.org/whl/MarkupSafe-2.1.5-cp312-cp312-macosx_10_9_universal2.whl", hash = "sha256:8dec4936e9c3100156f8a2dc89c4b88d5c435175ff03413b443469c7c8c5f4d1", upload-time = "2025-01-29T22:50:57.538Z" },
            { url = "https://download.pytorch.org/whl/MarkupSafe-2.1.5-cp312-cp312-macosx_10_9_x86_64.whl", hash = "sha256:3c6b973f22eb18a789b1460b4b91bf04ae3f0c4234a0a6aa6b0a92f6f7b951d4", upload-time = "2025-01-29T22:50:57.539Z" },
            { url = "https://download.pytorch.org/whl/MarkupSafe-2.1.5-cp312-cp312-manylinux_2_17_aarch64.manylinux2014_aarch64.whl", hash = "sha256:ac07bad82163452a6884fe8fa0963fb98c2346ba78d779ec06bd7a6262132aee", upload-time = "2025-01-29T22:50:57.539Z" },
            { url = "https://download.pytorch.org/whl/MarkupSafe-2.1.5-cp312-cp312-manylinux_2_17_x86_64.manylinux2014_x86_64.whl", hash = "sha256:f5dfb42c4604dddc8e4305050aa6deb084540643ed5804d7455b5df8fe16f5e5", upload-time = "2025-01-29T22:50:57.539Z" },
            { url = "https://download.pytorch.org/whl/MarkupSafe-2.1.5-cp312-cp312-manylinux_2_5_i686.manylinux1_i686.manylinux_2_17_i686.manylinux2014_i686.whl", hash = "sha256:ea3d8a3d18833cf4304cd2fc9cbb1efe188ca9b5efef2bdac7adc20594a0e46b", upload-time = "2025-01-29T22:50:57.539Z" },
            { url = "https://download.pytorch.org/whl/MarkupSafe-2.1.5-cp312-cp312-win_amd64.whl", hash = "sha256:823b65d8706e32ad2df51ed89496147a42a2a6e01c13cfb6ffb8b1e92bc910bb", upload-time = "2025-01-29T22:50:57.539Z" },
        ]

        [[package]]
        name = "mpmath"
        version = "1.3.0"
        source = { registry = "https://astral-sh.github.io/pytorch-mirror/whl/cu124" }
        wheels = [
            { url = "https://download.pytorch.org/whl/mpmath-1.3.0-py3-none-any.whl", hash = "sha256:a0b2b9fe80bbcd81a6647ff13108738cfb482d481d826cc0e02f5b35e5c88d2c", upload-time = "2025-01-29T22:50:57.683Z" },
        ]

        [[package]]
        name = "networkx"
        version = "3.3"
        source = { registry = "https://astral-sh.github.io/pytorch-mirror/whl/cu124" }
        wheels = [
            { url = "https://download.pytorch.org/whl/networkx-3.3-py3-none-any.whl", upload-time = "2025-01-29T22:50:57.843Z" },
        ]

        [[package]]
        name = "nvidia-cublas-cu12"
        version = "12.4.5.8"
        source = { registry = "https://astral-sh.github.io/pytorch-mirror/whl/cu124" }
        wheels = [
            { url = "https://download.pytorch.org/whl/cu124/nvidia_cublas_cu12-12.4.5.8-py3-none-manylinux2014_aarch64.whl", hash = "sha256:0f8aa1706812e00b9f19dfe0cdb3999b092ccb8ca168c0db5b8ea712456fd9b3", upload-time = "2025-01-29T22:51:38.991Z" },
            { url = "https://download.pytorch.org/whl/cu124/nvidia_cublas_cu12-12.4.5.8-py3-none-manylinux2014_x86_64.whl", hash = "sha256:2fc8da60df463fdefa81e323eef2e36489e1c94335b5358bcb38360adf75ac9b", upload-time = "2025-01-29T22:51:38.991Z" },
            { url = "https://download.pytorch.org/whl/cu124/nvidia_cublas_cu12-12.4.5.8-py3-none-win_amd64.whl", hash = "sha256:5a796786da89203a0657eda402bcdcec6180254a8ac22d72213abc42069522dc", upload-time = "2025-01-29T22:51:38.992Z" },
        ]

        [[package]]
        name = "nvidia-cuda-cupti-cu12"
        version = "12.4.127"
        source = { registry = "https://astral-sh.github.io/pytorch-mirror/whl/cu124" }
        wheels = [
            { url = "https://download.pytorch.org/whl/cu124/nvidia_cuda_cupti_cu12-12.4.127-py3-none-manylinux2014_aarch64.whl", hash = "sha256:79279b35cf6f91da114182a5ce1864997fd52294a87a16179ce275773799458a", upload-time = "2025-01-29T22:51:39.068Z" },
            { url = "https://download.pytorch.org/whl/cu124/nvidia_cuda_cupti_cu12-12.4.127-py3-none-manylinux2014_x86_64.whl", hash = "sha256:9dec60f5ac126f7bb551c055072b69d85392b13311fcc1bcda2202d172df30fb", upload-time = "2025-01-29T22:51:39.068Z" },
            { url = "https://download.pytorch.org/whl/cu124/nvidia_cuda_cupti_cu12-12.4.127-py3-none-win_amd64.whl", hash = "sha256:5688d203301ab051449a2b1cb6690fbe90d2b372f411521c86018b950f3d7922", upload-time = "2025-01-29T22:51:39.069Z" },
        ]

        [[package]]
        name = "nvidia-cuda-nvrtc-cu12"
        version = "12.4.127"
        source = { registry = "https://astral-sh.github.io/pytorch-mirror/whl/cu124" }
        wheels = [
            { url = "https://download.pytorch.org/whl/cu124/nvidia_cuda_nvrtc_cu12-12.4.127-py3-none-manylinux2014_aarch64.whl", hash = "sha256:0eedf14185e04b76aa05b1fea04133e59f465b6f960c0cbf4e37c3cb6b0ea198", upload-time = "2025-01-29T22:51:39.138Z" },
            { url = "https://download.pytorch.org/whl/cu124/nvidia_cuda_nvrtc_cu12-12.4.127-py3-none-manylinux2014_x86_64.whl", hash = "sha256:a178759ebb095827bd30ef56598ec182b85547f1508941a3d560eb7ea1fbf338", upload-time = "2025-01-29T22:51:39.138Z" },
            { url = "https://download.pytorch.org/whl/cu124/nvidia_cuda_nvrtc_cu12-12.4.127-py3-none-win_amd64.whl", hash = "sha256:a961b2f1d5f17b14867c619ceb99ef6fcec12e46612711bcec78eb05068a60ec", upload-time = "2025-01-29T22:51:39.139Z" },
        ]

        [[package]]
        name = "nvidia-cuda-runtime-cu12"
        version = "12.4.127"
        source = { registry = "https://astral-sh.github.io/pytorch-mirror/whl/cu124" }
        wheels = [
            { url = "https://download.pytorch.org/whl/cu124/nvidia_cuda_runtime_cu12-12.4.127-py3-none-manylinux2014_aarch64.whl", hash = "sha256:961fe0e2e716a2a1d967aab7caee97512f71767f852f67432d572e36cb3a11f3", upload-time = "2025-01-29T22:51:39.232Z" },
            { url = "https://download.pytorch.org/whl/cu124/nvidia_cuda_runtime_cu12-12.4.127-py3-none-manylinux2014_x86_64.whl", hash = "sha256:64403288fa2136ee8e467cdc9c9427e0434110899d07c779f25b5c068934faa5", upload-time = "2025-01-29T22:51:39.232Z" },
            { url = "https://download.pytorch.org/whl/cu124/nvidia_cuda_runtime_cu12-12.4.127-py3-none-win_amd64.whl", hash = "sha256:09c2e35f48359752dfa822c09918211844a3d93c100a715d79b59591130c5e1e", upload-time = "2025-01-29T22:51:39.232Z" },
        ]

        [[package]]
        name = "nvidia-cudnn-cu12"
        version = "9.1.0.70"
        source = { registry = "https://astral-sh.github.io/pytorch-mirror/whl/cu124" }
        dependencies = [
            { name = "nvidia-cublas-cu12" },
        ]
        wheels = [
            { url = "https://download.pytorch.org/whl/cu124/nvidia_cudnn_cu12-9.1.0.70-py3-none-manylinux2014_x86_64.whl", hash = "sha256:165764f44ef8c61fcdfdfdbe769d687e06374059fbb388b6c89ecb0e28793a6f", upload-time = "2025-01-29T22:51:39.303Z" },
        ]

        [[package]]
        name = "nvidia-cufft-cu12"
        version = "11.2.1.3"
        source = { registry = "https://astral-sh.github.io/pytorch-mirror/whl/cu124" }
        dependencies = [
            { name = "nvidia-nvjitlink-cu12" },
        ]
        wheels = [
            { url = "https://download.pytorch.org/whl/cu124/nvidia_cufft_cu12-11.2.1.3-py3-none-manylinux2014_aarch64.whl", hash = "sha256:5dad8008fc7f92f5ddfa2101430917ce2ffacd86824914c82e28990ad7f00399", upload-time = "2025-01-29T22:51:39.382Z" },
            { url = "https://download.pytorch.org/whl/cu124/nvidia_cufft_cu12-11.2.1.3-py3-none-manylinux2014_x86_64.whl", hash = "sha256:f083fc24912aa410be21fa16d157fed2055dab1cc4b6934a0e03cba69eb242b9", upload-time = "2025-01-29T22:51:39.382Z" },
            { url = "https://download.pytorch.org/whl/cu124/nvidia_cufft_cu12-11.2.1.3-py3-none-win_amd64.whl", hash = "sha256:d802f4954291101186078ccbe22fc285a902136f974d369540fd4a5333d1440b", upload-time = "2025-01-29T22:51:39.382Z" },
        ]

        [[package]]
        name = "nvidia-curand-cu12"
        version = "10.3.5.147"
        source = { registry = "https://astral-sh.github.io/pytorch-mirror/whl/cu124" }
        wheels = [
            { url = "https://download.pytorch.org/whl/cu124/nvidia_curand_cu12-10.3.5.147-py3-none-manylinux2014_aarch64.whl", hash = "sha256:1f173f09e3e3c76ab084aba0de819c49e56614feae5c12f69883f4ae9bb5fad9", upload-time = "2025-01-29T22:51:39.466Z" },
            { url = "https://download.pytorch.org/whl/cu124/nvidia_curand_cu12-10.3.5.147-py3-none-manylinux2014_x86_64.whl", hash = "sha256:a88f583d4e0bb643c49743469964103aa59f7f708d862c3ddb0fc07f851e3b8b", upload-time = "2025-01-29T22:51:39.467Z" },
            { url = "https://download.pytorch.org/whl/cu124/nvidia_curand_cu12-10.3.5.147-py3-none-win_amd64.whl", hash = "sha256:f307cc191f96efe9e8f05a87096abc20d08845a841889ef78cb06924437f6771", upload-time = "2025-01-29T22:51:39.467Z" },
        ]

        [[package]]
        name = "nvidia-cusolver-cu12"
        version = "11.6.1.9"
        source = { registry = "https://astral-sh.github.io/pytorch-mirror/whl/cu124" }
        dependencies = [
            { name = "nvidia-cublas-cu12" },
            { name = "nvidia-cusparse-cu12" },
            { name = "nvidia-nvjitlink-cu12" },
        ]
        wheels = [
            { url = "https://download.pytorch.org/whl/cu124/nvidia_cusolver_cu12-11.6.1.9-py3-none-manylinux2014_aarch64.whl", hash = "sha256:d338f155f174f90724bbde3758b7ac375a70ce8e706d70b018dd3375545fc84e", upload-time = "2025-01-29T22:51:39.551Z" },
            { url = "https://download.pytorch.org/whl/cu124/nvidia_cusolver_cu12-11.6.1.9-py3-none-manylinux2014_x86_64.whl", hash = "sha256:19e33fa442bcfd085b3086c4ebf7e8debc07cfe01e11513cc6d332fd918ac260", upload-time = "2025-01-29T22:51:39.551Z" },
            { url = "https://download.pytorch.org/whl/cu124/nvidia_cusolver_cu12-11.6.1.9-py3-none-win_amd64.whl", hash = "sha256:e77314c9d7b694fcebc84f58989f3aa4fb4cb442f12ca1a9bde50f5e8f6d1b9c", upload-time = "2025-01-29T22:51:39.551Z" },
        ]

        [[package]]
        name = "nvidia-cusparse-cu12"
        version = "12.3.1.170"
        source = { registry = "https://astral-sh.github.io/pytorch-mirror/whl/cu124" }
        dependencies = [
            { name = "nvidia-nvjitlink-cu12" },
        ]
        wheels = [
            { url = "https://download.pytorch.org/whl/cu124/nvidia_cusparse_cu12-12.3.1.170-py3-none-manylinux2014_aarch64.whl", hash = "sha256:9d32f62896231ebe0480efd8a7f702e143c98cfaa0e8a76df3386c1ba2b54df3", upload-time = "2025-01-29T22:51:39.648Z" },
            { url = "https://download.pytorch.org/whl/cu124/nvidia_cusparse_cu12-12.3.1.170-py3-none-manylinux2014_x86_64.whl", hash = "sha256:ea4f11a2904e2a8dc4b1833cc1b5181cde564edd0d5cd33e3c168eff2d1863f1", upload-time = "2025-01-29T22:51:39.648Z" },
            { url = "https://download.pytorch.org/whl/cu124/nvidia_cusparse_cu12-12.3.1.170-py3-none-win_amd64.whl", hash = "sha256:9bc90fb087bc7b4c15641521f31c0371e9a612fc2ba12c338d3ae032e6b6797f", upload-time = "2025-01-29T22:51:39.648Z" },
        ]

        [[package]]
        name = "nvidia-cusparselt-cu12"
        version = "0.6.2"
        source = { registry = "https://astral-sh.github.io/pytorch-mirror/whl/cu124" }
        wheels = [
            { url = "https://download.pytorch.org/whl/cu124/nvidia_cusparselt_cu12-0.6.2-py3-none-manylinux2014_aarch64.whl", upload-time = "2025-01-29T22:51:39.74Z" },
            { url = "https://download.pytorch.org/whl/cu124/nvidia_cusparselt_cu12-0.6.2-py3-none-manylinux2014_x86_64.whl", upload-time = "2025-01-29T22:51:39.74Z" },
            { url = "https://download.pytorch.org/whl/cu124/nvidia_cusparselt_cu12-0.6.2-py3-none-win_amd64.whl", upload-time = "2025-01-29T22:51:39.74Z" },
        ]

        [[package]]
        name = "nvidia-nccl-cu12"
        version = "2.21.5"
        source = { registry = "https://astral-sh.github.io/pytorch-mirror/whl/cu124" }
        wheels = [
            { url = "https://download.pytorch.org/whl/cu124/nvidia_nccl_cu12-2.21.5-py3-none-manylinux2014_x86_64.whl", hash = "sha256:8579076d30a8c24988834445f8d633c697d42397e92ffc3f63fa26766d25e0a0", upload-time = "2025-01-29T22:51:39.917Z" },
            { url = "https://download.pytorch.org/whl/nvidia_nccl_cu12-2.21.5-py3-none-manylinux2014_x86_64.whl", hash = "sha256:8579076d30a8c24988834445f8d633c697d42397e92ffc3f63fa26766d25e0a0", upload-time = "2025-01-29T22:50:58.013Z" },
        ]

        [[package]]
        name = "nvidia-nvjitlink-cu12"
        version = "12.4.127"
        source = { registry = "https://astral-sh.github.io/pytorch-mirror/whl/cu124" }
        wheels = [
            { url = "https://download.pytorch.org/whl/cu124/nvidia_nvjitlink_cu12-12.4.127-py3-none-manylinux2014_aarch64.whl", hash = "sha256:4abe7fef64914ccfa909bc2ba39739670ecc9e820c83ccc7a6ed414122599b83", upload-time = "2025-01-29T22:51:40.008Z" },
            { url = "https://download.pytorch.org/whl/cu124/nvidia_nvjitlink_cu12-12.4.127-py3-none-manylinux2014_x86_64.whl", hash = "sha256:06b3b9b25bf3f8af351d664978ca26a16d2c5127dbd53c0497e28d1fb9611d57", upload-time = "2025-01-29T22:51:40.008Z" },
            { url = "https://download.pytorch.org/whl/cu124/nvidia_nvjitlink_cu12-12.4.127-py3-none-win_amd64.whl", hash = "sha256:fd9020c501d27d135f983c6d3e244b197a7ccad769e34df53a42e276b0e25fa1", upload-time = "2025-01-29T22:51:40.008Z" },
        ]

        [[package]]
        name = "nvidia-nvtx-cu12"
        version = "12.4.127"
        source = { registry = "https://astral-sh.github.io/pytorch-mirror/whl/cu124" }
        wheels = [
            { url = "https://download.pytorch.org/whl/cu124/nvidia_nvtx_cu12-12.4.127-py3-none-manylinux2014_aarch64.whl", hash = "sha256:7959ad635db13edf4fc65c06a6e9f9e55fc2f92596db928d169c0bb031e88ef3", upload-time = "2025-01-29T22:51:40.099Z" },
            { url = "https://download.pytorch.org/whl/cu124/nvidia_nvtx_cu12-12.4.127-py3-none-manylinux2014_x86_64.whl", hash = "sha256:781e950d9b9f60d8241ccea575b32f5105a5baf4c2351cab5256a24869f12a1a", upload-time = "2025-01-29T22:51:40.099Z" },
            { url = "https://download.pytorch.org/whl/cu124/nvidia_nvtx_cu12-12.4.127-py3-none-win_amd64.whl", hash = "sha256:641dccaaa1139f3ffb0d3164b4b84f9d253397e38246a4f2f36728b48566d485", upload-time = "2025-01-29T22:51:40.099Z" },
        ]

        [[package]]
        name = "setuptools"
        version = "70.2.0"
        source = { registry = "https://astral-sh.github.io/pytorch-mirror/whl/cu124" }
        wheels = [
            { url = "https://download.pytorch.org/whl/setuptools-70.2.0-py3-none-any.whl", upload-time = "2025-01-29T22:50:58.769Z" },
        ]

        [[package]]
        name = "sympy"
        version = "1.13.1"
        source = { registry = "https://astral-sh.github.io/pytorch-mirror/whl/cu124" }
        dependencies = [
            { name = "mpmath" },
        ]
        wheels = [
            { url = "https://download.pytorch.org/whl/sympy-1.13.1-py3-none-any.whl", hash = "sha256:db36cdc64bf61b9b24578b6f7bab1ecdd2452cf008f34faa33776680c26d66f8", upload-time = "2025-01-29T22:50:58.85Z" },
        ]

        [[package]]
        name = "test-torch"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "torch", version = "2.6.0", source = { registry = "https://astral-sh.github.io/pytorch-mirror/whl/cpu" }, marker = "(sys_platform == 'darwin' and extra == 'extra-10-test-torch-cpu') or (extra == 'extra-10-test-torch-cpu' and extra == 'extra-10-test-torch-cu124')" },
            { name = "torch", version = "2.6.0+cpu", source = { registry = "https://astral-sh.github.io/pytorch-mirror/whl/cpu" }, marker = "(sys_platform != 'darwin' and extra == 'extra-10-test-torch-cpu') or (extra == 'extra-10-test-torch-cpu' and extra == 'extra-10-test-torch-cu124')" },
            { name = "torch", version = "2.6.0+cu124", source = { registry = "https://astral-sh.github.io/pytorch-mirror/whl/cu124" }, marker = "extra == 'extra-10-test-torch-cu124' or extra != 'extra-10-test-torch-cpu'" },
        ]

        [package.optional-dependencies]
        cpu = [
            { name = "torch", version = "2.6.0", source = { registry = "https://astral-sh.github.io/pytorch-mirror/whl/cpu" }, marker = "(sys_platform == 'darwin' and extra == 'extra-10-test-torch-cpu') or (extra == 'extra-10-test-torch-cpu' and extra == 'extra-10-test-torch-cu124')" },
            { name = "torch", version = "2.6.0+cpu", source = { registry = "https://astral-sh.github.io/pytorch-mirror/whl/cpu" }, marker = "(sys_platform != 'darwin' and extra == 'extra-10-test-torch-cpu') or (extra == 'extra-10-test-torch-cpu' and extra == 'extra-10-test-torch-cu124')" },
        ]
        cu124 = [
            { name = "torch", version = "2.6.0+cu124", source = { registry = "https://astral-sh.github.io/pytorch-mirror/whl/cu124" } },
        ]

        [package.metadata]
        requires-dist = [
            { name = "torch", specifier = ">=2.6.0,<2.7.0" },
            { name = "torch", marker = "extra == 'cpu'", specifier = ">=2.6.0,<2.7.0", index = "https://astral-sh.github.io/pytorch-mirror/whl/cpu", conflict = { package = "test-torch", extra = "cpu" } },
            { name = "torch", marker = "extra == 'cu124'", specifier = ">=2.6.0,<2.7.0" },
        ]
        provides-extras = ["cpu", "cu124"]

        [[package]]
        name = "torch"
        version = "2.6.0"
        source = { registry = "https://astral-sh.github.io/pytorch-mirror/whl/cpu" }
        resolution-markers = [
            "sys_platform == 'darwin'",
        ]
        dependencies = [
            { name = "filelock", marker = "(sys_platform == 'darwin' and extra == 'extra-10-test-torch-cpu') or (extra == 'extra-10-test-torch-cpu' and extra == 'extra-10-test-torch-cu124')" },
            { name = "fsspec", marker = "(sys_platform == 'darwin' and extra == 'extra-10-test-torch-cpu') or (extra == 'extra-10-test-torch-cpu' and extra == 'extra-10-test-torch-cu124')" },
            { name = "jinja2", marker = "(sys_platform == 'darwin' and extra == 'extra-10-test-torch-cpu') or (extra == 'extra-10-test-torch-cpu' and extra == 'extra-10-test-torch-cu124')" },
            { name = "networkx", marker = "(sys_platform == 'darwin' and extra == 'extra-10-test-torch-cpu') or (extra == 'extra-10-test-torch-cpu' and extra == 'extra-10-test-torch-cu124')" },
            { name = "setuptools", marker = "(sys_platform == 'darwin' and extra == 'extra-10-test-torch-cpu') or (extra == 'extra-10-test-torch-cpu' and extra == 'extra-10-test-torch-cu124')" },
            { name = "sympy", marker = "(sys_platform == 'darwin' and extra == 'extra-10-test-torch-cpu') or (extra == 'extra-10-test-torch-cpu' and extra == 'extra-10-test-torch-cu124')" },
            { name = "typing-extensions", marker = "(sys_platform == 'darwin' and extra == 'extra-10-test-torch-cpu') or (extra == 'extra-10-test-torch-cpu' and extra == 'extra-10-test-torch-cu124')" },
        ]
        wheels = [
            { url = "https://download.pytorch.org/whl/cpu/torch-2.6.0-cp312-none-macosx_11_0_arm64.whl", upload-time = "2025-01-29T22:50:59.085Z" },
        ]

        [[package]]
        name = "torch"
        version = "2.6.0+cpu"
        source = { registry = "https://astral-sh.github.io/pytorch-mirror/whl/cpu" }
        resolution-markers = [
            "sys_platform != 'darwin'",
        ]
        dependencies = [
            { name = "filelock", marker = "(sys_platform != 'darwin' and extra == 'extra-10-test-torch-cpu') or (extra == 'extra-10-test-torch-cpu' and extra == 'extra-10-test-torch-cu124')" },
            { name = "fsspec", marker = "(sys_platform != 'darwin' and extra == 'extra-10-test-torch-cpu') or (extra == 'extra-10-test-torch-cpu' and extra == 'extra-10-test-torch-cu124')" },
            { name = "jinja2", marker = "(sys_platform != 'darwin' and extra == 'extra-10-test-torch-cpu') or (extra == 'extra-10-test-torch-cpu' and extra == 'extra-10-test-torch-cu124')" },
            { name = "networkx", marker = "(sys_platform != 'darwin' and extra == 'extra-10-test-torch-cpu') or (extra == 'extra-10-test-torch-cpu' and extra == 'extra-10-test-torch-cu124')" },
            { name = "setuptools", marker = "(sys_platform != 'darwin' and extra == 'extra-10-test-torch-cpu') or (extra == 'extra-10-test-torch-cpu' and extra == 'extra-10-test-torch-cu124')" },
            { name = "sympy", marker = "(sys_platform != 'darwin' and extra == 'extra-10-test-torch-cpu') or (extra == 'extra-10-test-torch-cpu' and extra == 'extra-10-test-torch-cu124')" },
            { name = "typing-extensions", marker = "(sys_platform != 'darwin' and extra == 'extra-10-test-torch-cpu') or (extra == 'extra-10-test-torch-cpu' and extra == 'extra-10-test-torch-cu124')" },
        ]
        wheels = [
            { url = "https://download.pytorch.org/whl/cpu/torch-2.6.0%2Bcpu-cp312-cp312-linux_x86_64.whl", upload-time = "2025-01-29T22:50:59.085Z" },
            { url = "https://download.pytorch.org/whl/cpu/torch-2.6.0%2Bcpu-cp312-cp312-manylinux_2_28_aarch64.whl", upload-time = "2025-01-29T22:50:59.085Z" },
            { url = "https://download.pytorch.org/whl/cpu/torch-2.6.0%2Bcpu-cp312-cp312-win_amd64.whl", upload-time = "2025-01-29T22:50:59.085Z" },
        ]

        [[package]]
        name = "torch"
        version = "2.6.0+cu124"
        source = { registry = "https://astral-sh.github.io/pytorch-mirror/whl/cu124" }
        dependencies = [
            { name = "filelock" },
            { name = "fsspec" },
            { name = "jinja2" },
            { name = "networkx" },
            { name = "nvidia-cublas-cu12", marker = "(platform_machine == 'x86_64' and sys_platform == 'linux' and extra != 'extra-10-test-torch-cpu') or (extra == 'extra-10-test-torch-cpu' and extra == 'extra-10-test-torch-cu124')" },
            { name = "nvidia-cuda-cupti-cu12", marker = "(platform_machine == 'x86_64' and sys_platform == 'linux' and extra != 'extra-10-test-torch-cpu') or (extra == 'extra-10-test-torch-cpu' and extra == 'extra-10-test-torch-cu124')" },
            { name = "nvidia-cuda-nvrtc-cu12", marker = "(platform_machine == 'x86_64' and sys_platform == 'linux' and extra != 'extra-10-test-torch-cpu') or (extra == 'extra-10-test-torch-cpu' and extra == 'extra-10-test-torch-cu124')" },
            { name = "nvidia-cuda-runtime-cu12", marker = "(platform_machine == 'x86_64' and sys_platform == 'linux' and extra != 'extra-10-test-torch-cpu') or (extra == 'extra-10-test-torch-cpu' and extra == 'extra-10-test-torch-cu124')" },
            { name = "nvidia-cudnn-cu12", marker = "(platform_machine == 'x86_64' and sys_platform == 'linux' and extra != 'extra-10-test-torch-cpu') or (extra == 'extra-10-test-torch-cpu' and extra == 'extra-10-test-torch-cu124')" },
            { name = "nvidia-cufft-cu12", marker = "(platform_machine == 'x86_64' and sys_platform == 'linux' and extra != 'extra-10-test-torch-cpu') or (extra == 'extra-10-test-torch-cpu' and extra == 'extra-10-test-torch-cu124')" },
            { name = "nvidia-curand-cu12", marker = "(platform_machine == 'x86_64' and sys_platform == 'linux' and extra != 'extra-10-test-torch-cpu') or (extra == 'extra-10-test-torch-cpu' and extra == 'extra-10-test-torch-cu124')" },
            { name = "nvidia-cusolver-cu12", marker = "(platform_machine == 'x86_64' and sys_platform == 'linux' and extra != 'extra-10-test-torch-cpu') or (extra == 'extra-10-test-torch-cpu' and extra == 'extra-10-test-torch-cu124')" },
            { name = "nvidia-cusparse-cu12", marker = "(platform_machine == 'x86_64' and sys_platform == 'linux' and extra != 'extra-10-test-torch-cpu') or (extra == 'extra-10-test-torch-cpu' and extra == 'extra-10-test-torch-cu124')" },
            { name = "nvidia-cusparselt-cu12", marker = "(platform_machine == 'x86_64' and sys_platform == 'linux' and extra != 'extra-10-test-torch-cpu') or (extra == 'extra-10-test-torch-cpu' and extra == 'extra-10-test-torch-cu124')" },
            { name = "nvidia-nccl-cu12", marker = "(platform_machine == 'x86_64' and sys_platform == 'linux' and extra != 'extra-10-test-torch-cpu') or (extra == 'extra-10-test-torch-cpu' and extra == 'extra-10-test-torch-cu124')" },
            { name = "nvidia-nvjitlink-cu12", marker = "(platform_machine == 'x86_64' and sys_platform == 'linux' and extra != 'extra-10-test-torch-cpu') or (extra == 'extra-10-test-torch-cpu' and extra == 'extra-10-test-torch-cu124')" },
            { name = "nvidia-nvtx-cu12", marker = "(platform_machine == 'x86_64' and sys_platform == 'linux' and extra != 'extra-10-test-torch-cpu') or (extra == 'extra-10-test-torch-cpu' and extra == 'extra-10-test-torch-cu124')" },
            { name = "setuptools", marker = "extra == 'extra-10-test-torch-cu124' or extra != 'extra-10-test-torch-cpu'" },
            { name = "sympy", marker = "extra == 'extra-10-test-torch-cu124' or extra != 'extra-10-test-torch-cpu'" },
            { name = "triton", marker = "(platform_machine == 'x86_64' and sys_platform == 'linux' and extra != 'extra-10-test-torch-cpu') or (extra == 'extra-10-test-torch-cpu' and extra == 'extra-10-test-torch-cu124')" },
            { name = "typing-extensions" },
        ]
        wheels = [
            { url = "https://download.pytorch.org/whl/cu124/torch-2.6.0%2Bcu124-cp312-cp312-linux_x86_64.whl", upload-time = "2025-01-29T22:51:41.169Z" },
            { url = "https://download.pytorch.org/whl/cu124/torch-2.6.0%2Bcu124-cp312-cp312-win_amd64.whl", upload-time = "2025-01-29T22:51:41.169Z" },
        ]

        [[package]]
        name = "triton"
        version = "3.2.0"
        source = { registry = "https://astral-sh.github.io/pytorch-mirror/whl/cu124" }
        wheels = [
            { url = "https://download.pytorch.org/whl/triton-3.2.0-cp312-cp312-manylinux_2_17_x86_64.manylinux2014_x86_64.whl", upload-time = "2025-01-29T22:51:00.867Z" },
            { url = "https://download.pytorch.org/whl/triton-3.2.0-cp312-cp312-manylinux_2_27_x86_64.manylinux_2_28_x86_64.whl", upload-time = "2025-01-29T22:51:00.867Z" },
        ]

        [[package]]
        name = "typing-extensions"
        version = "4.12.2"
        source = { registry = "https://astral-sh.github.io/pytorch-mirror/whl/cu124" }
        wheels = [
            { url = "https://download.pytorch.org/whl/typing_extensions-4.12.2-py3-none-any.whl", upload-time = "2025-01-29T22:51:00.933Z" },
        ]
        "#
        );
    });

    uv_snapshot!(
        context.filters(),
        context
            .sync()
            .arg("--frozen")
            .arg("--dry-run")
            .arg("--python-platform")
            .arg("x86_64-manylinux2014"),
        @"
    exit_code: 0 (success)
    ----- stderr -----
    Would use project environment at: .venv
    Would download 24 packages
    Would install 24 packages
     + filelock==3.13.1
     + fsspec==2024.6.1
     + jinja2==3.1.4
     + markupsafe==2.1.5
     + mpmath==1.3.0
     + networkx==3.3
     + nvidia-cublas-cu12==12.4.5.8
     + nvidia-cuda-cupti-cu12==12.4.127
     + nvidia-cuda-nvrtc-cu12==12.4.127
     + nvidia-cuda-runtime-cu12==12.4.127
     + nvidia-cudnn-cu12==9.1.0.70
     + nvidia-cufft-cu12==11.2.1.3
     + nvidia-curand-cu12==10.3.5.147
     + nvidia-cusolver-cu12==11.6.1.9
     + nvidia-cusparse-cu12==12.3.1.170
     + nvidia-cusparselt-cu12==0.6.2
     + nvidia-nccl-cu12==2.21.5
     + nvidia-nvjitlink-cu12==12.4.127
     + nvidia-nvtx-cu12==12.4.127
     + setuptools==70.2.0
     + sympy==1.13.1
     + torch==2.6.0+cu124
     + triton==3.2.0
     + typing-extensions==4.12.2
    ");

    Ok(())
}

/// Ref: <https://github.com/astral-sh/uv/issues/9735>
#[test]
fn avoids_exponential_lock_file_growth() -> Result<()> {
    let cpu = uv_test::packse::PackseServer::new("packages/marker-growth-cpu.toml");
    let cu124 = uv_test::packse::PackseServer::new("packages/marker-growth-cu124.toml");
    let context = uv_test::test_context!("3.12")
        .with_packse_index("packages/marker-growth-registry.toml")
        .with_exclude_newer("2025-02-06T00:00Z");

    let pyproject = r#"
        [project]
        name = "resolution-markers-for-days"
        version = "0.1.0"
        description = "Add your description here"
        readme = "README.md"
        requires-python = ">=3.12"
        dependencies = []

        [project.optional-dependencies]
        cpu = [
            "growth-engine>=2.6.0"
        ]
        cu124 = [
            "growth-engine>=2.6.0"
        ]

        [tool.uv]
        conflicts = [
            [
                { extra = "cpu" },
                { extra = "cu124" },
            ],
        ]

        [tool.uv.sources]
        growth-engine = [
            { extra = "cpu", index = "pytorch-cpu" },
            { extra = "cu124", index = "pytorch-cu124" },
        ]

        [[tool.uv.index]]
        name = "pytorch-cpu"
        url = "[CPU_INDEX]"
        explicit = true

        [[tool.uv.index]]
        name = "pytorch-cu124"
        url = "[CU124_INDEX]"
        explicit = true
    "#
    .replace("[CPU_INDEX]", &cpu.index_url())
    .replace("[CU124_INDEX]", &cu124.index_url());

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&pyproject)?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 27 packages in [TIME]
    ");

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock,
            @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        resolution-markers = [
            "extra != 'extra-27-resolution-markers-for-days-cpu' and extra == 'extra-27-resolution-markers-for-days-cu124'",
            "sys_platform != 'darwin' and extra == 'extra-27-resolution-markers-for-days-cpu' and extra != 'extra-27-resolution-markers-for-days-cu124'",
            "sys_platform == 'darwin' and extra == 'extra-27-resolution-markers-for-days-cpu' and extra != 'extra-27-resolution-markers-for-days-cu124'",
            "extra != 'extra-27-resolution-markers-for-days-cpu' and extra != 'extra-27-resolution-markers-for-days-cu124'",
        ]
        conflicts = [[
            { package = "resolution-markers-for-days", extra = "cpu" },
            { package = "resolution-markers-for-days", extra = "cu124" },
        ]]

        [options]
        exclude-newer = "2025-02-06T00:00:00Z"

        [[package]]
        name = "growth-engine"
        version = "2.6.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform == 'darwin'",
        ]
        dependencies = [
            { name = "growth-filelock", marker = "sys_platform == 'darwin'" },
            { name = "growth-fsspec", marker = "sys_platform == 'darwin'" },
            { name = "growth-jinja2", marker = "sys_platform == 'darwin'" },
            { name = "growth-networkx", marker = "sys_platform == 'darwin'" },
            { name = "growth-setuptools", marker = "sys_platform == 'darwin'" },
            { name = "growth-sympy", marker = "sys_platform == 'darwin'" },
            { name = "growth-typing-extensions", marker = "sys_platform == 'darwin'" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_engine-2.6.0-cp312-none-macosx_11_0_arm64.whl", hash = "sha256:96a8c25718fe29975090f16fff626ecc195d2af8b64cbea091f8066198ea296a", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/growth_engine-2.6.0-cp313-none-macosx_11_0_arm64.whl", hash = "sha256:78a423cb02542f852f47a85fc658d514955c0d8b2d968cb8c1697072ecb4fdb3", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-engine"
        version = "2.6.0+cpu"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform != 'darwin'",
        ]
        dependencies = [
            { name = "growth-filelock", marker = "sys_platform != 'darwin'" },
            { name = "growth-fsspec", marker = "sys_platform != 'darwin'" },
            { name = "growth-jinja2", marker = "sys_platform != 'darwin'" },
            { name = "growth-networkx", marker = "sys_platform != 'darwin'" },
            { name = "growth-setuptools", marker = "sys_platform != 'darwin'" },
            { name = "growth-sympy", marker = "sys_platform != 'darwin'" },
            { name = "growth-typing-extensions", marker = "sys_platform != 'darwin'" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_engine-2.6.0+cpu-cp312-cp312-linux_x86_64.whl", hash = "sha256:f0bd0382024e2ac0dbbbdbfde7746c7e0faf74a9ac90e35bdb5152336553c111", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/growth_engine-2.6.0+cpu-cp312-cp312-manylinux_2_28_aarch64.whl", hash = "sha256:ebd644b1761c9dd4a8755b07d66b705be316acc49720a2052db86c91a9729c9c", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/growth_engine-2.6.0+cpu-cp312-cp312-win_amd64.whl", hash = "sha256:32024c6b42944e75aa3fde7692b7cb3b6dad2d10d186a952d7ae584af9747d57", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/growth_engine-2.6.0+cpu-cp313-cp313-linux_x86_64.whl", hash = "sha256:90a1733afd66d12dab21e69a8ef7ca9a16c01a605be1c2b9e4c16a9f06fab624", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/growth_engine-2.6.0+cpu-cp313-cp313-manylinux_2_28_aarch64.whl", hash = "sha256:553e24661806bc43daafaceb9464975e7e8048a2b3b6d9275e82d3a1eee87c92", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/growth_engine-2.6.0+cpu-cp313-cp313-win_amd64.whl", hash = "sha256:419b7bee3fca480ed6d0e394216fbaeef8c86743b9b1f35d170e4abcb1aae370", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/growth_engine-2.6.0+cpu-cp313-cp313t-linux_x86_64.whl", hash = "sha256:0ab1fcdfd38f63f88e4e51a9e813a0ce0e9676f05e2cf077d4676fb703d1bf36", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/growth_engine-2.6.0+cpu-cp313-cp313t-manylinux_2_28_aarch64.whl", hash = "sha256:fe47c4209446cf4b7783c817fd55daab9b23f4c3321695bc5d15a741882ec75a", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-engine"
        version = "2.6.0+cu124"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "growth-filelock" },
            { name = "growth-fsspec" },
            { name = "growth-jinja2" },
            { name = "growth-networkx" },
            { name = "growth-nvidia-cublas-cu12", marker = "platform_machine == 'x86_64' and sys_platform == 'linux'" },
            { name = "growth-nvidia-cuda-cupti-cu12", marker = "platform_machine == 'x86_64' and sys_platform == 'linux'" },
            { name = "growth-nvidia-cuda-nvrtc-cu12", marker = "platform_machine == 'x86_64' and sys_platform == 'linux'" },
            { name = "growth-nvidia-cuda-runtime-cu12", marker = "platform_machine == 'x86_64' and sys_platform == 'linux'" },
            { name = "growth-nvidia-cudnn-cu12", marker = "platform_machine == 'x86_64' and sys_platform == 'linux'" },
            { name = "growth-nvidia-cufft-cu12", marker = "platform_machine == 'x86_64' and sys_platform == 'linux'" },
            { name = "growth-nvidia-curand-cu12", marker = "platform_machine == 'x86_64' and sys_platform == 'linux'" },
            { name = "growth-nvidia-cusolver-cu12", marker = "platform_machine == 'x86_64' and sys_platform == 'linux'" },
            { name = "growth-nvidia-cusparse-cu12", marker = "platform_machine == 'x86_64' and sys_platform == 'linux'" },
            { name = "growth-nvidia-cusparselt-cu12", marker = "platform_machine == 'x86_64' and sys_platform == 'linux'" },
            { name = "growth-nvidia-nccl-cu12", marker = "platform_machine == 'x86_64' and sys_platform == 'linux'" },
            { name = "growth-nvidia-nvjitlink-cu12", marker = "platform_machine == 'x86_64' and sys_platform == 'linux'" },
            { name = "growth-nvidia-nvtx-cu12", marker = "platform_machine == 'x86_64' and sys_platform == 'linux'" },
            { name = "growth-setuptools" },
            { name = "growth-sympy" },
            { name = "growth-triton", marker = "platform_machine == 'x86_64' and sys_platform == 'linux'" },
            { name = "growth-typing-extensions" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_engine-2.6.0+cu124-cp312-cp312-linux_x86_64.whl", hash = "sha256:36f666d5ddcef79acdf7b9ce43edd7b9ccb7f1ac89a36a2172c8e2a0264ade6c", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/growth_engine-2.6.0+cu124-cp312-cp312-win_amd64.whl", hash = "sha256:36c94e83374a2098cef68b8e2d632f9ccb26f4c8123465c41b264d9a51428c2b", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/growth_engine-2.6.0+cu124-cp313-cp313-linux_x86_64.whl", hash = "sha256:0105d2e8c009afe607a3203c705e967a02ab14d00277b9e5084daadf7fb552d4", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/growth_engine-2.6.0+cu124-cp313-cp313-win_amd64.whl", hash = "sha256:25c699aef7e4d160f74a4c54d1de9e8a85844c5c0f5510c93fd7fc4ee985ed2e", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/growth_engine-2.6.0+cu124-cp313-cp313t-linux_x86_64.whl", hash = "sha256:894d4ab091ec070d96435917bdf7fb1da1d98ca49cb02439fb99933d92461b2f", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-filelock"
        version = "3.17.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_filelock-3.17.0.tar.gz", hash = "sha256:1e1a922a0cda081005e61197f3354c980f9ca650269a650d78d337546cf749ee", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_filelock-3.17.0-py3-none-any.whl", hash = "sha256:2a3998021c278eeb6b5b8699625c31967d6ea0c55894956bd8ec0c2852951211", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-fsspec"
        version = "2025.2.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_fsspec-2025.2.0.tar.gz", hash = "sha256:8c5c2243eb43c5a8bf59b478f5cc58e3a5ef4bb51a95f8ad035d8995c0cb5ad8", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_fsspec-2025.2.0-py3-none-any.whl", hash = "sha256:696954c3a486883c72f1505b4d32b1b85cab12dfa34b78b2159d026c02178f6e", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-jinja2"
        version = "3.1.5"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "growth-markupsafe" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/growth_jinja2-3.1.5.tar.gz", hash = "sha256:d9fc1c65e805939a53c0afdfdd9683d25dd933fbe081ce473f197bc89268d4bc", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_jinja2-3.1.5-py3-none-any.whl", hash = "sha256:d497cfbb73c8578b409001c9b3c34f3c62e2c13cb76319bb8ab503f956381e01", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-markupsafe"
        version = "3.0.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_markupsafe-3.0.2.tar.gz", hash = "sha256:b9646bebb2e466f8c9597cedb257b3f0e4bdf313f613f759a8700594d3261074", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_markupsafe-3.0.2-py3-none-any.whl", hash = "sha256:19852dcf57a19fbd42dc4825426f8660e3999907712bc2eac89818757640db6e", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-mpmath"
        version = "1.3.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_mpmath-1.3.0.tar.gz", hash = "sha256:f2846406b35724885dbb886b70d87dc5b31d1f20defe9157a9cc40add1e1888b", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_mpmath-1.3.0-py3-none-any.whl", hash = "sha256:96486c46d6669f45f54b9850915534fa627b138f9f8d31ad3416e011b11501ec", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-networkx"
        version = "3.4.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_networkx-3.4.2.tar.gz", hash = "sha256:2b39945e3bc15b3c9adf59d943d547cff96e70774a3a55ea7c3adccfbbb840cd", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_networkx-3.4.2-py3-none-any.whl", hash = "sha256:5a279b5af7574f358e17ffa4827e3ee3457864876fc4d0ae810d3952c5067733", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-nvidia-cublas-cu12"
        version = "12.4.5.8"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_nvidia_cublas_cu12-12.4.5.8.tar.gz", hash = "sha256:7691c11b1fda56266e82b9c62454c479766bd826fd5827823c0b2f7b879d4771", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_nvidia_cublas_cu12-12.4.5.8-py3-none-any.whl", hash = "sha256:345f9fc861eb0ed7dd307ff4ba627e4927be6306bcf7d8b6fea9c86fc589ab44", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-nvidia-cuda-cupti-cu12"
        version = "12.4.127"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_nvidia_cuda_cupti_cu12-12.4.127.tar.gz", hash = "sha256:3e94836f0a24be6f638e865a998968e553d126f177a68e2de90df8177782aab0", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_nvidia_cuda_cupti_cu12-12.4.127-py3-none-any.whl", hash = "sha256:7449c2204a5f09698ac3b08dc276c4e2805ea8d91c203a5dd8886172638cb5e3", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-nvidia-cuda-nvrtc-cu12"
        version = "12.4.127"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_nvidia_cuda_nvrtc_cu12-12.4.127.tar.gz", hash = "sha256:acbf5d628a3f94c09768ed03f3ebb29bda7a3c97771b83156273d34e2f6d777e", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_nvidia_cuda_nvrtc_cu12-12.4.127-py3-none-any.whl", hash = "sha256:acb38fdd41b98d7089947b8e442d0080b262cfd68baf26d98f6b06f753311968", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-nvidia-cuda-runtime-cu12"
        version = "12.4.127"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_nvidia_cuda_runtime_cu12-12.4.127.tar.gz", hash = "sha256:671e70fbff2c859e75294e9a89efede6c6b34e907427ea121d276bff825ed7b1", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_nvidia_cuda_runtime_cu12-12.4.127-py3-none-any.whl", hash = "sha256:e3262c1ffb651540ae7e976bafcbf1a9e4e01c5942561b0b1057539f7e3c179c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-nvidia-cudnn-cu12"
        version = "9.1.0.70"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "growth-nvidia-cublas-cu12" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/growth_nvidia_cudnn_cu12-9.1.0.70.tar.gz", hash = "sha256:406ec73218dd97317967c2bac493d76f4f390a659231c6db255fdfdd85813248", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_nvidia_cudnn_cu12-9.1.0.70-py3-none-any.whl", hash = "sha256:1fd769ce860e0b20edb5c4e5114edc9693e961faadd02929bde7552e03e30f1d", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-nvidia-cufft-cu12"
        version = "11.2.1.3"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "growth-nvidia-nvjitlink-cu12" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/growth_nvidia_cufft_cu12-11.2.1.3.tar.gz", hash = "sha256:9df96c33d9578535eea019f046c5caefd5e0cc9f2173b7533506c6e82bfef4a8", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_nvidia_cufft_cu12-11.2.1.3-py3-none-any.whl", hash = "sha256:2ec7fab3dbd205335525d064e0f8f218efd6c2680cc1ca3249ec8ba0a23fd0da", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-nvidia-curand-cu12"
        version = "10.3.5.147"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_nvidia_curand_cu12-10.3.5.147.tar.gz", hash = "sha256:86a72e3a65f1c9a4ddb2bade1c284e76a3dba3aaf24eba4e0f854c94ad2e8435", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_nvidia_curand_cu12-10.3.5.147-py3-none-any.whl", hash = "sha256:c819b8e0be5dde260f4a1907c2305030830d385492173bf57a8e7a250b6d2ac9", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-nvidia-cusolver-cu12"
        version = "11.6.1.9"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "growth-nvidia-cublas-cu12" },
            { name = "growth-nvidia-cusparse-cu12" },
            { name = "growth-nvidia-nvjitlink-cu12" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/growth_nvidia_cusolver_cu12-11.6.1.9.tar.gz", hash = "sha256:fcaeea12714f69862a41aa770154caf7b2cd28f79509e150e1966739d3d83494", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_nvidia_cusolver_cu12-11.6.1.9-py3-none-any.whl", hash = "sha256:d6a304880e6cecb9635f2f5eeef05fa6c0f94359d40901495b1d5d6707dabb64", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-nvidia-cusparse-cu12"
        version = "12.3.1.170"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "growth-nvidia-nvjitlink-cu12" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/growth_nvidia_cusparse_cu12-12.3.1.170.tar.gz", hash = "sha256:0c8539c34253e205bb4e46e9458b16e362dd51603ecaf9b061438142fd62e244", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_nvidia_cusparse_cu12-12.3.1.170-py3-none-any.whl", hash = "sha256:4a6a742bdd483d84f978157f55bc74677df79fc680546694d6ac7aab64bdaff7", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-nvidia-cusparselt-cu12"
        version = "0.6.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_nvidia_cusparselt_cu12-0.6.2.tar.gz", hash = "sha256:04ba07505e669b525285c5a6727e3061b6b59cd4c0175572d701db70f0a37616", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_nvidia_cusparselt_cu12-0.6.2-py3-none-any.whl", hash = "sha256:cb017cf2ac2fbcb914676ce334114a69e4ec50016a449b0fdeedf868d19c8333", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-nvidia-nccl-cu12"
        version = "2.21.5"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_nvidia_nccl_cu12-2.21.5.tar.gz", hash = "sha256:e2f51e84ab990a55a2750e8befeec63dfdc83c0a721dc0730b3c095d2cb107eb", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_nvidia_nccl_cu12-2.21.5-py3-none-any.whl", hash = "sha256:e45d6431babc7e832a26f71fcb1c8a08818529abecd71cb9a76471a3c14a07c8", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-nvidia-nvjitlink-cu12"
        version = "12.4.127"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_nvidia_nvjitlink_cu12-12.4.127.tar.gz", hash = "sha256:50daf134d6502093bc9894ecf56e8e115d398f978edb2f1422a0664c74976574", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_nvidia_nvjitlink_cu12-12.4.127-py3-none-any.whl", hash = "sha256:e9bbf60768315188a199317a6267a89fefd1c21fbae1ed4de13ffa38233542ae", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-nvidia-nvtx-cu12"
        version = "12.4.127"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_nvidia_nvtx_cu12-12.4.127.tar.gz", hash = "sha256:20efe9080b7ddf567ae7b4f163aa2551528230043a276990d083bd10ad5e49be", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_nvidia_nvtx_cu12-12.4.127-py3-none-any.whl", hash = "sha256:233a0fc4d3c4056a4418759130154ccd90eaadb3ac030d0cb63281746a418e74", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-setuptools"
        version = "75.8.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_setuptools-75.8.0.tar.gz", hash = "sha256:c6081c327dc7b9277f59d7c8841ad091f17ea46a612d4476bdeb79b1badb3391", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_setuptools-75.8.0-py3-none-any.whl", hash = "sha256:edb1b4f637a3d48196c9c7379d2eb239bccee1549f7a78b31decf280c7e36718", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-sympy"
        version = "1.13.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "growth-mpmath" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/growth_sympy-1.13.1.tar.gz", hash = "sha256:bc2a29c7250402bbb830194bf94bb1a0c272f915ae31c9a7bb9e2a9980b72206", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_sympy-1.13.1-py3-none-any.whl", hash = "sha256:fe1e4dbf15926bd40a2f16f7ffd7fa8ea726d5a0682f8130be3e23114dca48a6", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-triton"
        version = "3.2.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_triton-3.2.0.tar.gz", hash = "sha256:30e807f960d3b673c8cc6de9acb4849ba33ba396847092e33fd4f313acd19c71", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_triton-3.2.0-py3-none-any.whl", hash = "sha256:97d9ef8084039c1734f2a9e01be9c6f3c3c620f6bf8325c1abb92dcebf15417a", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-typing-extensions"
        version = "4.12.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_typing_extensions-4.12.2.tar.gz", hash = "sha256:25bbfdc1e62d1d2f2d4b22006fd753c7934d0703c7e1c4316deeeb91eb7afdea", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_typing_extensions-4.12.2-py3-none-any.whl", hash = "sha256:6256ca08ae2bec12771d1c3aba988f38165c7b88c252368ac9647f1678bc6f29", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "resolution-markers-for-days"
        version = "0.1.0"
        source = { virtual = "." }

        [package.optional-dependencies]
        cpu = [
            { name = "growth-engine", version = "2.6.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "(sys_platform == 'darwin' and extra == 'extra-27-resolution-markers-for-days-cpu') or (extra == 'extra-27-resolution-markers-for-days-cpu' and extra == 'extra-27-resolution-markers-for-days-cu124')" },
            { name = "growth-engine", version = "2.6.0+cpu", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "(sys_platform != 'darwin' and extra == 'extra-27-resolution-markers-for-days-cpu') or (extra == 'extra-27-resolution-markers-for-days-cpu' and extra == 'extra-27-resolution-markers-for-days-cu124')" },
        ]
        cu124 = [
            { name = "growth-engine", version = "2.6.0+cu124", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]

        [package.metadata]
        requires-dist = [
            { name = "growth-engine", marker = "extra == 'cpu'", specifier = ">=2.6.0", index = "http://[LOCALHOST]/simple/", conflict = { package = "resolution-markers-for-days", extra = "cpu" } },
            { name = "growth-engine", marker = "extra == 'cu124'", specifier = ">=2.6.0", index = "http://[LOCALHOST]/simple/", conflict = { package = "resolution-markers-for-days", extra = "cu124" } },
        ]
        provides-extras = ["cpu", "cu124"]
        "#
        );
    });

    // At this point, we twiddle something in the pyproject.toml
    // to re-run a resolution using the existing lock file.
    // Previously, this is where we would run into problems with
    // the resolution markers seemingly growing at an exponential
    // rate.
    let pyproject = pyproject.replace(r#"version = "0.1.0""#, r#"version = "0.1.1""#);
    pyproject_toml.write_str(&pyproject)?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 27 packages in [TIME]
    Updated resolution-markers-for-days v0.1.0 -> v0.1.1
    ");

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock,
            @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        resolution-markers = [
            "extra != 'extra-27-resolution-markers-for-days-cpu' and extra == 'extra-27-resolution-markers-for-days-cu124'",
            "sys_platform != 'darwin' and extra == 'extra-27-resolution-markers-for-days-cpu' and extra != 'extra-27-resolution-markers-for-days-cu124'",
            "sys_platform == 'darwin' and extra == 'extra-27-resolution-markers-for-days-cpu' and extra != 'extra-27-resolution-markers-for-days-cu124'",
            "extra != 'extra-27-resolution-markers-for-days-cpu' and extra != 'extra-27-resolution-markers-for-days-cu124'",
        ]
        conflicts = [[
            { package = "resolution-markers-for-days", extra = "cpu" },
            { package = "resolution-markers-for-days", extra = "cu124" },
        ]]

        [options]
        exclude-newer = "2025-02-06T00:00:00Z"

        [[package]]
        name = "growth-engine"
        version = "2.6.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform == 'darwin'",
        ]
        dependencies = [
            { name = "growth-filelock", marker = "sys_platform == 'darwin'" },
            { name = "growth-fsspec", marker = "sys_platform == 'darwin'" },
            { name = "growth-jinja2", marker = "sys_platform == 'darwin'" },
            { name = "growth-networkx", marker = "sys_platform == 'darwin'" },
            { name = "growth-setuptools", marker = "sys_platform == 'darwin'" },
            { name = "growth-sympy", marker = "sys_platform == 'darwin'" },
            { name = "growth-typing-extensions", marker = "sys_platform == 'darwin'" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_engine-2.6.0-cp312-none-macosx_11_0_arm64.whl", hash = "sha256:96a8c25718fe29975090f16fff626ecc195d2af8b64cbea091f8066198ea296a", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/growth_engine-2.6.0-cp313-none-macosx_11_0_arm64.whl", hash = "sha256:78a423cb02542f852f47a85fc658d514955c0d8b2d968cb8c1697072ecb4fdb3", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-engine"
        version = "2.6.0+cpu"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform != 'darwin'",
        ]
        dependencies = [
            { name = "growth-filelock", marker = "sys_platform != 'darwin'" },
            { name = "growth-fsspec", marker = "sys_platform != 'darwin'" },
            { name = "growth-jinja2", marker = "sys_platform != 'darwin'" },
            { name = "growth-networkx", marker = "sys_platform != 'darwin'" },
            { name = "growth-setuptools", marker = "sys_platform != 'darwin'" },
            { name = "growth-sympy", marker = "sys_platform != 'darwin'" },
            { name = "growth-typing-extensions", marker = "sys_platform != 'darwin'" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_engine-2.6.0+cpu-cp312-cp312-linux_x86_64.whl", hash = "sha256:f0bd0382024e2ac0dbbbdbfde7746c7e0faf74a9ac90e35bdb5152336553c111", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/growth_engine-2.6.0+cpu-cp312-cp312-manylinux_2_28_aarch64.whl", hash = "sha256:ebd644b1761c9dd4a8755b07d66b705be316acc49720a2052db86c91a9729c9c", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/growth_engine-2.6.0+cpu-cp312-cp312-win_amd64.whl", hash = "sha256:32024c6b42944e75aa3fde7692b7cb3b6dad2d10d186a952d7ae584af9747d57", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/growth_engine-2.6.0+cpu-cp313-cp313-linux_x86_64.whl", hash = "sha256:90a1733afd66d12dab21e69a8ef7ca9a16c01a605be1c2b9e4c16a9f06fab624", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/growth_engine-2.6.0+cpu-cp313-cp313-manylinux_2_28_aarch64.whl", hash = "sha256:553e24661806bc43daafaceb9464975e7e8048a2b3b6d9275e82d3a1eee87c92", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/growth_engine-2.6.0+cpu-cp313-cp313-win_amd64.whl", hash = "sha256:419b7bee3fca480ed6d0e394216fbaeef8c86743b9b1f35d170e4abcb1aae370", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/growth_engine-2.6.0+cpu-cp313-cp313t-linux_x86_64.whl", hash = "sha256:0ab1fcdfd38f63f88e4e51a9e813a0ce0e9676f05e2cf077d4676fb703d1bf36", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/growth_engine-2.6.0+cpu-cp313-cp313t-manylinux_2_28_aarch64.whl", hash = "sha256:fe47c4209446cf4b7783c817fd55daab9b23f4c3321695bc5d15a741882ec75a", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-engine"
        version = "2.6.0+cu124"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "growth-filelock" },
            { name = "growth-fsspec" },
            { name = "growth-jinja2" },
            { name = "growth-networkx" },
            { name = "growth-nvidia-cublas-cu12", marker = "platform_machine == 'x86_64' and sys_platform == 'linux'" },
            { name = "growth-nvidia-cuda-cupti-cu12", marker = "platform_machine == 'x86_64' and sys_platform == 'linux'" },
            { name = "growth-nvidia-cuda-nvrtc-cu12", marker = "platform_machine == 'x86_64' and sys_platform == 'linux'" },
            { name = "growth-nvidia-cuda-runtime-cu12", marker = "platform_machine == 'x86_64' and sys_platform == 'linux'" },
            { name = "growth-nvidia-cudnn-cu12", marker = "platform_machine == 'x86_64' and sys_platform == 'linux'" },
            { name = "growth-nvidia-cufft-cu12", marker = "platform_machine == 'x86_64' and sys_platform == 'linux'" },
            { name = "growth-nvidia-curand-cu12", marker = "platform_machine == 'x86_64' and sys_platform == 'linux'" },
            { name = "growth-nvidia-cusolver-cu12", marker = "platform_machine == 'x86_64' and sys_platform == 'linux'" },
            { name = "growth-nvidia-cusparse-cu12", marker = "platform_machine == 'x86_64' and sys_platform == 'linux'" },
            { name = "growth-nvidia-cusparselt-cu12", marker = "platform_machine == 'x86_64' and sys_platform == 'linux'" },
            { name = "growth-nvidia-nccl-cu12", marker = "platform_machine == 'x86_64' and sys_platform == 'linux'" },
            { name = "growth-nvidia-nvjitlink-cu12", marker = "platform_machine == 'x86_64' and sys_platform == 'linux'" },
            { name = "growth-nvidia-nvtx-cu12", marker = "platform_machine == 'x86_64' and sys_platform == 'linux'" },
            { name = "growth-setuptools" },
            { name = "growth-sympy" },
            { name = "growth-triton", marker = "platform_machine == 'x86_64' and sys_platform == 'linux'" },
            { name = "growth-typing-extensions" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_engine-2.6.0+cu124-cp312-cp312-linux_x86_64.whl", hash = "sha256:36f666d5ddcef79acdf7b9ce43edd7b9ccb7f1ac89a36a2172c8e2a0264ade6c", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/growth_engine-2.6.0+cu124-cp312-cp312-win_amd64.whl", hash = "sha256:36c94e83374a2098cef68b8e2d632f9ccb26f4c8123465c41b264d9a51428c2b", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/growth_engine-2.6.0+cu124-cp313-cp313-linux_x86_64.whl", hash = "sha256:0105d2e8c009afe607a3203c705e967a02ab14d00277b9e5084daadf7fb552d4", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/growth_engine-2.6.0+cu124-cp313-cp313-win_amd64.whl", hash = "sha256:25c699aef7e4d160f74a4c54d1de9e8a85844c5c0f5510c93fd7fc4ee985ed2e", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/growth_engine-2.6.0+cu124-cp313-cp313t-linux_x86_64.whl", hash = "sha256:894d4ab091ec070d96435917bdf7fb1da1d98ca49cb02439fb99933d92461b2f", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-filelock"
        version = "3.17.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_filelock-3.17.0.tar.gz", hash = "sha256:1e1a922a0cda081005e61197f3354c980f9ca650269a650d78d337546cf749ee", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_filelock-3.17.0-py3-none-any.whl", hash = "sha256:2a3998021c278eeb6b5b8699625c31967d6ea0c55894956bd8ec0c2852951211", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-fsspec"
        version = "2025.2.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_fsspec-2025.2.0.tar.gz", hash = "sha256:8c5c2243eb43c5a8bf59b478f5cc58e3a5ef4bb51a95f8ad035d8995c0cb5ad8", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_fsspec-2025.2.0-py3-none-any.whl", hash = "sha256:696954c3a486883c72f1505b4d32b1b85cab12dfa34b78b2159d026c02178f6e", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-jinja2"
        version = "3.1.5"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "growth-markupsafe" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/growth_jinja2-3.1.5.tar.gz", hash = "sha256:d9fc1c65e805939a53c0afdfdd9683d25dd933fbe081ce473f197bc89268d4bc", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_jinja2-3.1.5-py3-none-any.whl", hash = "sha256:d497cfbb73c8578b409001c9b3c34f3c62e2c13cb76319bb8ab503f956381e01", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-markupsafe"
        version = "3.0.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_markupsafe-3.0.2.tar.gz", hash = "sha256:b9646bebb2e466f8c9597cedb257b3f0e4bdf313f613f759a8700594d3261074", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_markupsafe-3.0.2-py3-none-any.whl", hash = "sha256:19852dcf57a19fbd42dc4825426f8660e3999907712bc2eac89818757640db6e", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-mpmath"
        version = "1.3.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_mpmath-1.3.0.tar.gz", hash = "sha256:f2846406b35724885dbb886b70d87dc5b31d1f20defe9157a9cc40add1e1888b", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_mpmath-1.3.0-py3-none-any.whl", hash = "sha256:96486c46d6669f45f54b9850915534fa627b138f9f8d31ad3416e011b11501ec", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-networkx"
        version = "3.4.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_networkx-3.4.2.tar.gz", hash = "sha256:2b39945e3bc15b3c9adf59d943d547cff96e70774a3a55ea7c3adccfbbb840cd", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_networkx-3.4.2-py3-none-any.whl", hash = "sha256:5a279b5af7574f358e17ffa4827e3ee3457864876fc4d0ae810d3952c5067733", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-nvidia-cublas-cu12"
        version = "12.4.5.8"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_nvidia_cublas_cu12-12.4.5.8.tar.gz", hash = "sha256:7691c11b1fda56266e82b9c62454c479766bd826fd5827823c0b2f7b879d4771", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_nvidia_cublas_cu12-12.4.5.8-py3-none-any.whl", hash = "sha256:345f9fc861eb0ed7dd307ff4ba627e4927be6306bcf7d8b6fea9c86fc589ab44", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-nvidia-cuda-cupti-cu12"
        version = "12.4.127"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_nvidia_cuda_cupti_cu12-12.4.127.tar.gz", hash = "sha256:3e94836f0a24be6f638e865a998968e553d126f177a68e2de90df8177782aab0", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_nvidia_cuda_cupti_cu12-12.4.127-py3-none-any.whl", hash = "sha256:7449c2204a5f09698ac3b08dc276c4e2805ea8d91c203a5dd8886172638cb5e3", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-nvidia-cuda-nvrtc-cu12"
        version = "12.4.127"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_nvidia_cuda_nvrtc_cu12-12.4.127.tar.gz", hash = "sha256:acbf5d628a3f94c09768ed03f3ebb29bda7a3c97771b83156273d34e2f6d777e", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_nvidia_cuda_nvrtc_cu12-12.4.127-py3-none-any.whl", hash = "sha256:acb38fdd41b98d7089947b8e442d0080b262cfd68baf26d98f6b06f753311968", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-nvidia-cuda-runtime-cu12"
        version = "12.4.127"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_nvidia_cuda_runtime_cu12-12.4.127.tar.gz", hash = "sha256:671e70fbff2c859e75294e9a89efede6c6b34e907427ea121d276bff825ed7b1", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_nvidia_cuda_runtime_cu12-12.4.127-py3-none-any.whl", hash = "sha256:e3262c1ffb651540ae7e976bafcbf1a9e4e01c5942561b0b1057539f7e3c179c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-nvidia-cudnn-cu12"
        version = "9.1.0.70"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "growth-nvidia-cublas-cu12" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/growth_nvidia_cudnn_cu12-9.1.0.70.tar.gz", hash = "sha256:406ec73218dd97317967c2bac493d76f4f390a659231c6db255fdfdd85813248", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_nvidia_cudnn_cu12-9.1.0.70-py3-none-any.whl", hash = "sha256:1fd769ce860e0b20edb5c4e5114edc9693e961faadd02929bde7552e03e30f1d", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-nvidia-cufft-cu12"
        version = "11.2.1.3"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "growth-nvidia-nvjitlink-cu12" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/growth_nvidia_cufft_cu12-11.2.1.3.tar.gz", hash = "sha256:9df96c33d9578535eea019f046c5caefd5e0cc9f2173b7533506c6e82bfef4a8", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_nvidia_cufft_cu12-11.2.1.3-py3-none-any.whl", hash = "sha256:2ec7fab3dbd205335525d064e0f8f218efd6c2680cc1ca3249ec8ba0a23fd0da", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-nvidia-curand-cu12"
        version = "10.3.5.147"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_nvidia_curand_cu12-10.3.5.147.tar.gz", hash = "sha256:86a72e3a65f1c9a4ddb2bade1c284e76a3dba3aaf24eba4e0f854c94ad2e8435", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_nvidia_curand_cu12-10.3.5.147-py3-none-any.whl", hash = "sha256:c819b8e0be5dde260f4a1907c2305030830d385492173bf57a8e7a250b6d2ac9", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-nvidia-cusolver-cu12"
        version = "11.6.1.9"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "growth-nvidia-cublas-cu12" },
            { name = "growth-nvidia-cusparse-cu12" },
            { name = "growth-nvidia-nvjitlink-cu12" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/growth_nvidia_cusolver_cu12-11.6.1.9.tar.gz", hash = "sha256:fcaeea12714f69862a41aa770154caf7b2cd28f79509e150e1966739d3d83494", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_nvidia_cusolver_cu12-11.6.1.9-py3-none-any.whl", hash = "sha256:d6a304880e6cecb9635f2f5eeef05fa6c0f94359d40901495b1d5d6707dabb64", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-nvidia-cusparse-cu12"
        version = "12.3.1.170"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "growth-nvidia-nvjitlink-cu12" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/growth_nvidia_cusparse_cu12-12.3.1.170.tar.gz", hash = "sha256:0c8539c34253e205bb4e46e9458b16e362dd51603ecaf9b061438142fd62e244", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_nvidia_cusparse_cu12-12.3.1.170-py3-none-any.whl", hash = "sha256:4a6a742bdd483d84f978157f55bc74677df79fc680546694d6ac7aab64bdaff7", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-nvidia-cusparselt-cu12"
        version = "0.6.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_nvidia_cusparselt_cu12-0.6.2.tar.gz", hash = "sha256:04ba07505e669b525285c5a6727e3061b6b59cd4c0175572d701db70f0a37616", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_nvidia_cusparselt_cu12-0.6.2-py3-none-any.whl", hash = "sha256:cb017cf2ac2fbcb914676ce334114a69e4ec50016a449b0fdeedf868d19c8333", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-nvidia-nccl-cu12"
        version = "2.21.5"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_nvidia_nccl_cu12-2.21.5.tar.gz", hash = "sha256:e2f51e84ab990a55a2750e8befeec63dfdc83c0a721dc0730b3c095d2cb107eb", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_nvidia_nccl_cu12-2.21.5-py3-none-any.whl", hash = "sha256:e45d6431babc7e832a26f71fcb1c8a08818529abecd71cb9a76471a3c14a07c8", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-nvidia-nvjitlink-cu12"
        version = "12.4.127"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_nvidia_nvjitlink_cu12-12.4.127.tar.gz", hash = "sha256:50daf134d6502093bc9894ecf56e8e115d398f978edb2f1422a0664c74976574", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_nvidia_nvjitlink_cu12-12.4.127-py3-none-any.whl", hash = "sha256:e9bbf60768315188a199317a6267a89fefd1c21fbae1ed4de13ffa38233542ae", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-nvidia-nvtx-cu12"
        version = "12.4.127"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_nvidia_nvtx_cu12-12.4.127.tar.gz", hash = "sha256:20efe9080b7ddf567ae7b4f163aa2551528230043a276990d083bd10ad5e49be", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_nvidia_nvtx_cu12-12.4.127-py3-none-any.whl", hash = "sha256:233a0fc4d3c4056a4418759130154ccd90eaadb3ac030d0cb63281746a418e74", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-setuptools"
        version = "75.8.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_setuptools-75.8.0.tar.gz", hash = "sha256:c6081c327dc7b9277f59d7c8841ad091f17ea46a612d4476bdeb79b1badb3391", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_setuptools-75.8.0-py3-none-any.whl", hash = "sha256:edb1b4f637a3d48196c9c7379d2eb239bccee1549f7a78b31decf280c7e36718", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-sympy"
        version = "1.13.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "growth-mpmath" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/growth_sympy-1.13.1.tar.gz", hash = "sha256:bc2a29c7250402bbb830194bf94bb1a0c272f915ae31c9a7bb9e2a9980b72206", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_sympy-1.13.1-py3-none-any.whl", hash = "sha256:fe1e4dbf15926bd40a2f16f7ffd7fa8ea726d5a0682f8130be3e23114dca48a6", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-triton"
        version = "3.2.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_triton-3.2.0.tar.gz", hash = "sha256:30e807f960d3b673c8cc6de9acb4849ba33ba396847092e33fd4f313acd19c71", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_triton-3.2.0-py3-none-any.whl", hash = "sha256:97d9ef8084039c1734f2a9e01be9c6f3c3c620f6bf8325c1abb92dcebf15417a", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "growth-typing-extensions"
        version = "4.12.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/growth_typing_extensions-4.12.2.tar.gz", hash = "sha256:25bbfdc1e62d1d2f2d4b22006fd753c7934d0703c7e1c4316deeeb91eb7afdea", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/growth_typing_extensions-4.12.2-py3-none-any.whl", hash = "sha256:6256ca08ae2bec12771d1c3aba988f38165c7b88c252368ac9647f1678bc6f29", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "resolution-markers-for-days"
        version = "0.1.1"
        source = { virtual = "." }

        [package.optional-dependencies]
        cpu = [
            { name = "growth-engine", version = "2.6.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "(sys_platform == 'darwin' and extra == 'extra-27-resolution-markers-for-days-cpu') or (extra == 'extra-27-resolution-markers-for-days-cpu' and extra == 'extra-27-resolution-markers-for-days-cu124')" },
            { name = "growth-engine", version = "2.6.0+cpu", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "(sys_platform != 'darwin' and extra == 'extra-27-resolution-markers-for-days-cpu') or (extra == 'extra-27-resolution-markers-for-days-cpu' and extra == 'extra-27-resolution-markers-for-days-cu124')" },
        ]
        cu124 = [
            { name = "growth-engine", version = "2.6.0+cu124", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]

        [package.metadata]
        requires-dist = [
            { name = "growth-engine", marker = "extra == 'cpu'", specifier = ">=2.6.0", index = "http://[LOCALHOST]/simple/", conflict = { package = "resolution-markers-for-days", extra = "cpu" } },
            { name = "growth-engine", marker = "extra == 'cu124'", specifier = ">=2.6.0", index = "http://[LOCALHOST]/simple/", conflict = { package = "resolution-markers-for-days", extra = "cu124" } },
        ]
        provides-extras = ["cpu", "cu124"]
        "#
        );
    });

    Ok(())
}

/// Check that simplification of conflict markers does not apply with not all paths activate the
/// marker unconditionally.
///
/// For example, when `a` and `b` conflict, this marker does not simplify:
/// ```text
/// (platform_machine == 'x86_64' and extra == 'extra-5-foo-b') or extra == 'extra-5-foo-a'
/// ````
///
/// Ref: <https://github.com/astral-sh/uv/issues/14805>
#[test]
fn do_not_simplify_if_not_all_conflict_extras_satisfy_the_marker_by_themselves() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_exclude_newer("2025-02-06T00:00Z");

    let pyproject = r#"
        [project]
        name = "debug"
        version = "0.0.1"
        requires-python = "==3.12.*"

        [project.optional-dependencies]
        a = [
          "python-dateutil==2.8.0",
        ]
        b = [
          "python-dateutil==2.8.1; platform_machine != 'inapplicable'",
          "python-dateutil==2.8.0; platform_machine == 'inapplicable'",
        ]

        [tool.uv]
        conflicts = [
          [
            { extra = "a" },
            { extra = "b" },
          ],
        ]
    "#;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(pyproject)?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock,
            @r#"
        version = 1
        revision = 3
        requires-python = "==3.12.*"
        resolution-markers = [
            "platform_machine != 'inapplicable' and extra != 'extra-5-debug-a' and extra == 'extra-5-debug-b'",
            "platform_machine == 'inapplicable' and extra != 'extra-5-debug-a' and extra == 'extra-5-debug-b'",
            "extra == 'extra-5-debug-a' and extra != 'extra-5-debug-b'",
            "extra != 'extra-5-debug-a' and extra != 'extra-5-debug-b'",
        ]
        conflicts = [[
            { package = "debug", extra = "a" },
            { package = "debug", extra = "b" },
        ]]

        [options]
        exclude-newer = "2025-02-06T00:00:00Z"

        [[package]]
        name = "debug"
        version = "0.0.1"
        source = { virtual = "." }

        [package.optional-dependencies]
        a = [
            { name = "python-dateutil", version = "2.8.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        b = [
            { name = "python-dateutil", version = "2.8.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "(platform_machine == 'inapplicable' and extra == 'extra-5-debug-b') or (extra == 'extra-5-debug-a' and extra == 'extra-5-debug-b')" },
            { name = "python-dateutil", version = "2.8.1", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "(platform_machine != 'inapplicable' and extra == 'extra-5-debug-b') or (extra == 'extra-5-debug-a' and extra == 'extra-5-debug-b')" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "python-dateutil", marker = "platform_machine == 'inapplicable' and extra == 'b'", specifier = "==2.8.0" },
            { name = "python-dateutil", marker = "platform_machine != 'inapplicable' and extra == 'b'", specifier = "==2.8.1" },
            { name = "python-dateutil", marker = "extra == 'a'", specifier = "==2.8.0" },
        ]
        provides-extras = ["a", "b"]

        [[package]]
        name = "python-dateutil"
        version = "2.8.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "platform_machine == 'inapplicable' and extra != 'extra-5-debug-a' and extra == 'extra-5-debug-b'",
            "extra == 'extra-5-debug-a' and extra != 'extra-5-debug-b'",
        ]
        dependencies = [
            { name = "six", marker = "(platform_machine == 'inapplicable' and extra == 'extra-5-debug-b') or extra == 'extra-5-debug-a'" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/python_dateutil-2.8.0.tar.gz", hash = "sha256:2e367ab830c8d1536057683c53121800c4a1a48aaf03c39e1ad2d489013b76fb", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/python_dateutil-2.8.0-py3-none-any.whl", hash = "sha256:9f4657d8f4b47ee59763503139461164f37af699867ee69b4008f168387799e1", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "python-dateutil"
        version = "2.8.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "platform_machine != 'inapplicable'",
        ]
        dependencies = [
            { name = "six", marker = "platform_machine != 'inapplicable'" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/python_dateutil-2.8.1.tar.gz", hash = "sha256:f316769fe81d4262b903ef1ed0191e00f81c31181e1317767a67a634b1ed229b", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/python_dateutil-2.8.1-py3-none-any.whl", hash = "sha256:6bb1418a7f1419a871e05fde777b47ec8b79c310581ba20168e93643bf898b31", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "six"
        version = "1.17.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/six-1.17.0.tar.gz", hash = "sha256:f862da43225047e86e2726667de4651067f7a3cddd0f9250f4bb33b07e4d7b0f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/six-1.17.0-py3-none-any.whl", hash = "sha256:2d4a0e87315356b6faa7b5ac289a75c6f77675311e4200f4709661e113b830fe", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // The incorrect behavior was that only python-dateutil was installed and six was missing.
    uv_snapshot!(context.filters(), context.sync().arg("--extra").arg("a").arg("--dry-run"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Would use project environment at: .venv
    Resolved 4 packages in [TIME]
    Found up-to-date lockfile at: uv.lock
    Would download 2 packages
    Would install 2 packages
     + python-dateutil==2.8.0
     + six==1.17.0
    ");

    Ok(())
}

/// This tests that typos in conflict item keys are rejected.
///
/// Using `name` instead of `package` should produce an error rather than being
/// silently ignored.
#[test]
fn conflict_item_unknown_field() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [tool.uv]
        conflicts = [
            [
              { name = "foo", extra = "extra1" },
              { extra = "extra2" },
            ],
        ]

        [project.optional-dependencies]
        extra1 = ["sortedcontainers==2.3.0"]
        extra2 = ["sortedcontainers==2.4.0"]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock(), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to parse: `pyproject.toml`
      cause: TOML parse error at line 10, column 17
                |
             10 |               { name = "foo", extra = "extra1" },
                |                 ^^^^
             unknown field `name`, expected one of `package`, `extra`, `group`
    "#);

    Ok(())
}

/// Test that many pairwise conflicts sharing a common extra don't cause
/// exponential fork growth. This mimics the pattern seen in projects like
/// sktime, where CI pin extras (e.g., `dependencies_lowest`) conflict with
/// many independent task extras (e.g., `forecasting`, `classification`).
///
/// Without optimization, N pairwise conflicts involving the same extra would
/// create O(2^N) forks. With the optimization, this stays small.
#[test]
fn many_pairwise_conflicts_shared_extra() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // A project with:
    // - `pinned`: pins sortedcontainers==2.3.0 (old version for CI testing)
    // - `a` through `e`: each uses sortedcontainers>=2.4
    // - Each task extra conflicts with `pinned` pairwise
    // - The task extras do NOT conflict with each other
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.optional-dependencies]
        pinned = ["sortedcontainers==2.3.0"]
        a = ["sortedcontainers>=2.4"]
        b = ["sortedcontainers>=2.4"]
        c = ["sortedcontainers>=2.4"]
        d = ["sortedcontainers>=2.4"]
        e = ["sortedcontainers>=2.4"]

        [tool.uv]
        conflicts = [
            [
              { extra = "pinned" },
              { extra = "a" },
            ],
            [
              { extra = "pinned" },
              { extra = "b" },
            ],
            [
              { extra = "pinned" },
              { extra = "c" },
            ],
            [
              { extra = "pinned" },
              { extra = "d" },
            ],
            [
              { extra = "pinned" },
              { extra = "e" },
            ],
        ]
        "#,
    )?;

    // This should resolve quickly — not hang due to exponential forking.
    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");

    let lock = context.read("uv.lock");

    // The lock file should have exactly 2 versions of sortedcontainers:
    // 2.3.0 for pinned and 2.4.0 for the task extras.
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        conflicts = [[
            { package = "project", extra = "a" },
            { package = "project", extra = "pinned" },
        ], [
            { package = "project", extra = "b" },
            { package = "project", extra = "pinned" },
        ], [
            { package = "project", extra = "c" },
            { package = "project", extra = "pinned" },
        ], [
            { package = "project", extra = "d" },
            { package = "project", extra = "pinned" },
        ], [
            { package = "project", extra = "e" },
            { package = "project", extra = "pinned" },
        ]]

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }

        [package.optional-dependencies]
        a = [
            { name = "sortedcontainers", version = "2.4.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        b = [
            { name = "sortedcontainers", version = "2.4.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        c = [
            { name = "sortedcontainers", version = "2.4.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        d = [
            { name = "sortedcontainers", version = "2.4.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        e = [
            { name = "sortedcontainers", version = "2.4.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        pinned = [
            { name = "sortedcontainers", version = "2.3.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]

        [package.metadata]
        requires-dist = [
            { name = "sortedcontainers", marker = "extra == 'a'", specifier = ">=2.4" },
            { name = "sortedcontainers", marker = "extra == 'b'", specifier = ">=2.4" },
            { name = "sortedcontainers", marker = "extra == 'c'", specifier = ">=2.4" },
            { name = "sortedcontainers", marker = "extra == 'd'", specifier = ">=2.4" },
            { name = "sortedcontainers", marker = "extra == 'e'", specifier = ">=2.4" },
            { name = "sortedcontainers", marker = "extra == 'pinned'", specifier = "==2.3.0" },
        ]
        provides-extras = ["pinned", "a", "b", "c", "d", "e"]

        [[package]]
        name = "sortedcontainers"
        version = "2.3.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sortedcontainers-2.3.0.tar.gz", hash = "sha256:c7bfd220eb6dcc29774e2e05298edc2c5d62fa1086cc6409c800c7f413afe2a6", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sortedcontainers-2.3.0-py3-none-any.whl", hash = "sha256:3228c480e84b2b7a504f28bd68f73b8cfde1d1fddf8a47783b19a0dbcc93ffcf", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "sortedcontainers"
        version = "2.4.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sortedcontainers-2.4.0.tar.gz", hash = "sha256:fb6015d312cfaf15c65e0aeadac28191546cc25e48d41b126ecc322d5b116be2", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sortedcontainers-2.4.0-py3-none-any.whl", hash = "sha256:e90c0f20bb5c630bbf840e4ccc94441cd55ae8048cf55fc42df715d28b67cb80", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    Ok(())
}

/// Test that a project-level conflict (i.e., `{ package = "pkg-a" }` without
/// extra or group) properly excludes the package's extras from the conflicting
/// fork.
///
/// Regression test for: <https://github.com/astral-sh/uv/issues/18015>
#[test]
fn project_level_conflict_with_extra() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let root_pyproject_toml = context.temp_dir.child("pyproject.toml");
    root_pyproject_toml.write_str(
        r#"
        [project]
        name = "workspace-root"
        version = "0.1.0"
        requires-python = ">=3.12"

        [tool.uv.workspace]
        members = ["pkg-a", "pkg-b"]

        [tool.uv]
        conflicts = [
            [
                { package = "pkg-a" },
                { package = "pkg-b", extra = "extra1" },
            ],
        ]
        "#,
    )?;

    let pkg_a_pyproject_toml = context.temp_dir.child("pkg-a").child("pyproject.toml");
    pkg_a_pyproject_toml.write_str(
        r#"
        [project]
        name = "pkg-a"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["sortedcontainers==2.3.0"]

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
        "#,
    )?;

    let pkg_b_pyproject_toml = context.temp_dir.child("pkg-b").child("pyproject.toml");
    pkg_b_pyproject_toml.write_str(
        r#"
        [project]
        name = "pkg-b"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.optional-dependencies]
        extra1 = ["sortedcontainers==2.4.0"]

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
        "#,
    )?;

    // Resolution should succeed: pkg-a (with sortedcontainers==2.3.0) and
    // pkg-b[extra1] (with sortedcontainers==2.4.0) are in separate forks
    // because of the declared project-level conflict.
    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Declaring conflicts for packages (`package = ...`) is experimental and may change without warning. Pass `--preview-features package-conflicts` to disable this warning.
    Resolved 5 packages in [TIME]
    ");

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock,
            @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        conflicts = [[
            { package = "pkg-a" },
            { package = "pkg-b", extra = "extra1" },
        ]]

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [manifest]
        members = [
            "pkg-a",
            "pkg-b",
            "workspace-root",
        ]

        [[package]]
        name = "pkg-a"
        version = "0.1.0"
        source = { editable = "pkg-a" }
        dependencies = [
            { name = "sortedcontainers", version = "2.3.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'project-5-pkg-a'" },
        ]

        [package.metadata]
        requires-dist = [{ name = "sortedcontainers", specifier = "==2.3.0" }]

        [[package]]
        name = "pkg-b"
        version = "0.1.0"
        source = { editable = "pkg-b" }

        [package.optional-dependencies]
        extra1 = [
            { name = "sortedcontainers", version = "2.4.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]

        [package.metadata]
        requires-dist = [{ name = "sortedcontainers", marker = "extra == 'extra1'", specifier = "==2.4.0" }]
        provides-extras = ["extra1"]

        [[package]]
        name = "sortedcontainers"
        version = "2.3.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sortedcontainers-2.3.0.tar.gz", hash = "sha256:c7bfd220eb6dcc29774e2e05298edc2c5d62fa1086cc6409c800c7f413afe2a6", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sortedcontainers-2.3.0-py3-none-any.whl", hash = "sha256:3228c480e84b2b7a504f28bd68f73b8cfde1d1fddf8a47783b19a0dbcc93ffcf", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "sortedcontainers"
        version = "2.4.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sortedcontainers-2.4.0.tar.gz", hash = "sha256:fb6015d312cfaf15c65e0aeadac28191546cc25e48d41b126ecc322d5b116be2", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sortedcontainers-2.4.0-py3-none-any.whl", hash = "sha256:e90c0f20bb5c630bbf840e4ccc94441cd55ae8048cf55fc42df715d28b67cb80", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "workspace-root"
        version = "0.1.0"
        source = { virtual = "." }
        "#
        );
    });

    // Re-run with `--locked`.
    uv_snapshot!(context.filters(), context.lock().arg("--locked"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Declaring conflicts for packages (`package = ...`) is experimental and may change without warning. Pass `--preview-features package-conflicts` to disable this warning.
    Resolved 5 packages in [TIME]
    ");

    Ok(())
}

/// Test that a project-level conflict works when the conflicting package has
/// extras that are also declared in additional conflict sets. This mirrors the
/// exact scenario from the issue: pkg-a depends on datasets<4, pkg-b[unsafe]
/// depends on datasets>4, and pkg-a depends on pkg-b[safe].
///
/// Regression test for: <https://github.com/astral-sh/uv/issues/18015>
#[test]
fn project_level_conflict_with_extras_and_cross_dependency() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let root_pyproject_toml = context.temp_dir.child("pyproject.toml");
    root_pyproject_toml.write_str(
        r#"
        [project]
        name = "workspace-root"
        version = "0.1.0"
        requires-python = ">=3.12"

        [tool.uv.workspace]
        members = ["pkg-a", "pkg-b"]

        [tool.uv]
        conflicts = [
            [
                { package = "pkg-a" },
                { package = "pkg-b", extra = "extra1" },
            ],
        ]
        "#,
    )?;

    let pkg_a_pyproject_toml = context.temp_dir.child("pkg-a").child("pyproject.toml");
    pkg_a_pyproject_toml.write_str(
        r#"
        [project]
        name = "pkg-a"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "sortedcontainers==2.3.0",
            "pkg-b[safe]",
        ]

        [project.optional-dependencies]
        all = [
            "pkg-a[foo]",
            "pkg-a[bar]",
        ]
        foo = ["idna==3.4"]
        bar = ["sniffio==1.3.0"]

        [tool.uv.sources]
        pkg-b = { workspace = true }

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
        "#,
    )?;

    let pkg_b_pyproject_toml = context.temp_dir.child("pkg-b").child("pyproject.toml");
    pkg_b_pyproject_toml.write_str(
        r#"
        [project]
        name = "pkg-b"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.optional-dependencies]
        safe = ["sortedcontainers"]
        extra1 = ["sortedcontainers==2.4.0"]

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
        "#,
    )?;

    // Resolution should succeed. pkg-a depends on sortedcontainers==2.3.0 and
    // pkg-b[extra1] depends on sortedcontainers==2.4.0. The project-level
    // conflict between pkg-a and pkg-b[extra1] should properly separate these
    // into different forks, including pkg-a's extras (all, foo, bar).
    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Declaring conflicts for packages (`package = ...`) is experimental and may change without warning. Pass `--preview-features package-conflicts` to disable this warning.
    Resolved 7 packages in [TIME]
    ");

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock,
            @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        conflicts = [[
            { package = "pkg-a" },
            { package = "pkg-b", extra = "extra1" },
        ]]

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [manifest]
        members = [
            "pkg-a",
            "pkg-b",
            "workspace-root",
        ]

        [[package]]
        name = "idna"
        version = "3.4"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.4.tar.gz", hash = "sha256:fa04479ae8ef727bc1261e8db7e7347dbef7a50e56e7fa5d9de7b18b43ab6d42", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.4-py3-none-any.whl", hash = "sha256:f1f209736fbedfb02f5abdcd1d04682bee585684dc5a944a896d763a12322b77", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "pkg-a"
        version = "0.1.0"
        source = { editable = "pkg-a" }
        dependencies = [
            { name = "pkg-b", extra = ["safe"], marker = "extra == 'project-5-pkg-a'" },
            { name = "sortedcontainers", version = "2.3.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'project-5-pkg-a'" },
        ]

        [package.optional-dependencies]
        all = [
            { name = "idna", marker = "extra == 'project-5-pkg-a'" },
            { name = "sniffio", marker = "extra == 'project-5-pkg-a'" },
        ]
        bar = [
            { name = "sniffio", marker = "extra == 'project-5-pkg-a'" },
        ]
        foo = [
            { name = "idna", marker = "extra == 'project-5-pkg-a'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "idna", marker = "extra == 'foo'", specifier = "==3.4" },
            { name = "pkg-a", extras = ["bar"], marker = "extra == 'all'" },
            { name = "pkg-a", extras = ["foo"], marker = "extra == 'all'" },
            { name = "pkg-b", extras = ["safe"], editable = "pkg-b" },
            { name = "sniffio", marker = "extra == 'bar'", specifier = "==1.3.0" },
            { name = "sortedcontainers", specifier = "==2.3.0" },
        ]
        provides-extras = ["all", "foo", "bar"]

        [[package]]
        name = "pkg-b"
        version = "0.1.0"
        source = { editable = "pkg-b" }

        [package.optional-dependencies]
        extra1 = [
            { name = "sortedcontainers", version = "2.4.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        safe = [
            { name = "sortedcontainers", version = "2.3.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'project-5-pkg-a'" },
            { name = "sortedcontainers", version = "2.4.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'extra-5-pkg-b-extra1' or extra != 'project-5-pkg-a'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "sortedcontainers", marker = "extra == 'extra1'", specifier = "==2.4.0" },
            { name = "sortedcontainers", marker = "extra == 'safe'" },
        ]
        provides-extras = ["safe", "extra1"]

        [[package]]
        name = "sniffio"
        version = "1.3.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sniffio-1.3.0.tar.gz", hash = "sha256:a08217890a74948b5d32f04af82b044fdda52ca3702ca589325b821188b5961b", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sniffio-1.3.0-py3-none-any.whl", hash = "sha256:4afbbc6d88347f024497f7d969f5af2085d24ced6377e63a30bfe881b6a99f77", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "sortedcontainers"
        version = "2.3.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sortedcontainers-2.3.0.tar.gz", hash = "sha256:c7bfd220eb6dcc29774e2e05298edc2c5d62fa1086cc6409c800c7f413afe2a6", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sortedcontainers-2.3.0-py3-none-any.whl", hash = "sha256:3228c480e84b2b7a504f28bd68f73b8cfde1d1fddf8a47783b19a0dbcc93ffcf", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "sortedcontainers"
        version = "2.4.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sortedcontainers-2.4.0.tar.gz", hash = "sha256:fb6015d312cfaf15c65e0aeadac28191546cc25e48d41b126ecc322d5b116be2", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sortedcontainers-2.4.0-py3-none-any.whl", hash = "sha256:e90c0f20bb5c630bbf840e4ccc94441cd55ae8048cf55fc42df715d28b67cb80", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "workspace-root"
        version = "0.1.0"
        source = { virtual = "." }
        "#
        );
    });

    // Re-run with `--locked`.
    uv_snapshot!(context.filters(), context.lock().arg("--locked"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Declaring conflicts for packages (`package = ...`) is experimental and may change without warning. Pass `--preview-features package-conflicts` to disable this warning.
    Resolved 7 packages in [TIME]
    ");

    Ok(())
}

/// Test that a project-level conflict correctly handles groups on the
/// excluded package. Groups do NOT depend on the base package (unlike
/// extras), so a group on pkg-a should remain active even when pkg-a
/// itself is excluded by a project-level conflict.
///
/// Regression test for: <https://github.com/astral-sh/uv/issues/18015>
#[test]
fn project_level_conflict_with_group() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let root_pyproject_toml = context.temp_dir.child("pyproject.toml");
    root_pyproject_toml.write_str(
        r#"
        [project]
        name = "workspace-root"
        version = "0.1.0"
        requires-python = ">=3.12"

        [tool.uv.workspace]
        members = ["pkg-a", "pkg-b"]

        [tool.uv]
        conflicts = [
            [
                { package = "pkg-a" },
                { package = "pkg-b", extra = "extra1" },
            ],
        ]
        "#,
    )?;

    let pkg_a_pyproject_toml = context.temp_dir.child("pkg-a").child("pyproject.toml");
    pkg_a_pyproject_toml.write_str(
        r#"
        [project]
        name = "pkg-a"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["sortedcontainers==2.3.0"]

        [dependency-groups]
        dev = ["sniffio==1.3.0"]

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
        "#,
    )?;

    let pkg_b_pyproject_toml = context.temp_dir.child("pkg-b").child("pyproject.toml");
    pkg_b_pyproject_toml.write_str(
        r#"
        [project]
        name = "pkg-b"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.optional-dependencies]
        extra1 = ["sortedcontainers==2.4.0"]

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
        "#,
    )?;

    // Resolution should succeed. The project-level conflict between pkg-a
    // and pkg-b[extra1] properly forks. The dev group on pkg-a should NOT
    // be excluded by the project-level conflict since groups don't depend
    // on the base package.
    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Declaring conflicts for packages (`package = ...`) is experimental and may change without warning. Pass `--preview-features package-conflicts` to disable this warning.
    Resolved 6 packages in [TIME]
    ");

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock,
            @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        conflicts = [[
            { package = "pkg-a" },
            { package = "pkg-b", extra = "extra1" },
        ]]

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [manifest]
        members = [
            "pkg-a",
            "pkg-b",
            "workspace-root",
        ]

        [[package]]
        name = "pkg-a"
        version = "0.1.0"
        source = { editable = "pkg-a" }
        dependencies = [
            { name = "sortedcontainers", version = "2.3.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "extra == 'project-5-pkg-a'" },
        ]

        [package.dev-dependencies]
        dev = [
            { name = "sniffio" },
        ]

        [package.metadata]
        requires-dist = [{ name = "sortedcontainers", specifier = "==2.3.0" }]

        [package.metadata.requires-dev]
        dev = [{ name = "sniffio", specifier = "==1.3.0" }]

        [[package]]
        name = "pkg-b"
        version = "0.1.0"
        source = { editable = "pkg-b" }

        [package.optional-dependencies]
        extra1 = [
            { name = "sortedcontainers", version = "2.4.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]

        [package.metadata]
        requires-dist = [{ name = "sortedcontainers", marker = "extra == 'extra1'", specifier = "==2.4.0" }]
        provides-extras = ["extra1"]

        [[package]]
        name = "sniffio"
        version = "1.3.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sniffio-1.3.0.tar.gz", hash = "sha256:a08217890a74948b5d32f04af82b044fdda52ca3702ca589325b821188b5961b", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sniffio-1.3.0-py3-none-any.whl", hash = "sha256:4afbbc6d88347f024497f7d969f5af2085d24ced6377e63a30bfe881b6a99f77", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "sortedcontainers"
        version = "2.3.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sortedcontainers-2.3.0.tar.gz", hash = "sha256:c7bfd220eb6dcc29774e2e05298edc2c5d62fa1086cc6409c800c7f413afe2a6", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sortedcontainers-2.3.0-py3-none-any.whl", hash = "sha256:3228c480e84b2b7a504f28bd68f73b8cfde1d1fddf8a47783b19a0dbcc93ffcf", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "sortedcontainers"
        version = "2.4.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sortedcontainers-2.4.0.tar.gz", hash = "sha256:fb6015d312cfaf15c65e0aeadac28191546cc25e48d41b126ecc322d5b116be2", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sortedcontainers-2.4.0-py3-none-any.whl", hash = "sha256:e90c0f20bb5c630bbf840e4ccc94441cd55ae8048cf55fc42df715d28b67cb80", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "workspace-root"
        version = "0.1.0"
        source = { virtual = "." }
        "#
        );
    });

    // Re-run with `--locked`.
    uv_snapshot!(context.filters(), context.lock().arg("--locked"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Declaring conflicts for packages (`package = ...`) is experimental and may change without warning. Pass `--preview-features package-conflicts` to disable this warning.
    Resolved 6 packages in [TIME]
    ");

    Ok(())
}

/// See: <https://github.com/astral-sh/uv/issues/16779>
#[test]
fn many_conflicts_with_requested_dependency_extra() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let root_pyproject_toml = context.temp_dir.child("pyproject.toml");
    // 29 conflicts results in a reasonably short run-time after fixing, and a very long runtime
    // without the fix. This is to attempt to ensure that this test won't fail sporadically on a
    // slow machine, or succeed (while broken) on a super fast machine.
    root_pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = "==3.12.*"
        dependencies = []

        [project.optional-dependencies]
        x00 = ["branch-package"]

        [tool.uv.sources]
        branch-package = { path = "branch-package" }

        [tool.uv]
        conflicts = [[
            { extra = "x00" },
            { extra = "x01" },
            { extra = "x02" },
            { extra = "x03" },
            { extra = "x04" },
            { extra = "x05" },
            { extra = "x06" },
            { extra = "x07" },
            { extra = "x08" },
            { extra = "x09" },
            { extra = "x10" },
            { extra = "x11" },
            { extra = "x12" },
            { extra = "x13" },
            { extra = "x14" },
            { extra = "x15" },
            { extra = "x16" },
            { extra = "x17" },
            { extra = "x18" },
            { extra = "x19" },
            { extra = "x20" },
            { extra = "x21" },
            { extra = "x22" },
            { extra = "x23" },
            { extra = "x24" },
            { extra = "x25" },
            { extra = "x26" },
            { extra = "x27" },
            { extra = "x28" },
        ]]
        "#,
    )?;

    let branch_pyproject_toml = context
        .temp_dir
        .child("branch-package")
        .child("pyproject.toml");
    branch_pyproject_toml.write_str(
        r#"
        [project]
        name = "branch-package"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["leaf-package[easy]"]

        [tool.uv.sources]
        leaf-package = { path = "../leaf-package" }
        "#,
    )?;

    let leaf_pyproject_toml = context
        .temp_dir
        .child("leaf-package")
        .child("pyproject.toml");
    leaf_pyproject_toml.write_str(
        r#"
        [project]
        name = "leaf-package"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.optional-dependencies]
        easy = []
        "#,
    )?;

    assert_cmd::Command::from_std(context.lock())
        .timeout(Duration::from_mins(1))
        .assert()
        .success();

    Ok(())
}
