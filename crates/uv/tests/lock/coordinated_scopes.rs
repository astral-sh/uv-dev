#![cfg(feature = "test-universal")]

use anyhow::{Context, Result};
use assert_fs::prelude::*;
use indoc::{formatdoc, indoc};
use insta::assert_snapshot;
use toml_edit::{DocumentMut, Item};

use uv_test::packse::PackseServer;
use uv_test::uv_snapshot;

fn registry_choices(lock: &str, name: &str, indexes: &[(&str, &str)]) -> Result<String> {
    let lock = lock.parse::<DocumentMut>()?;
    let packages = lock
        .get("package")
        .and_then(Item::as_array_of_tables)
        .context("lockfile must contain packages")?;
    let mut choices = Vec::new();
    for package in packages {
        if package.get("name").and_then(Item::as_str) != Some(name) {
            continue;
        }
        let version = package
            .get("version")
            .and_then(Item::as_str)
            .context("registry package must contain a version")?;
        let index = package
            .get("source")
            .and_then(|source| source.get("registry"))
            .and_then(Item::as_str)
            .context("registry package must contain its index")?;
        let label = indexes
            .iter()
            .find_map(|(url, label)| (*url == index).then_some(*label))
            .unwrap_or(index);
        choices.push(format!("{name}=={version} @ {label}"));
    }
    choices.sort_unstable();
    Ok(choices.join("\n"))
}

/// An environment-covering explicit index is immutable source policy, so it does not prevent
/// coordinated backtracking of the package's parent.
#[test]
fn coordinated_scope_unconditional_explicit_index() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let primary = PackseServer::new("fork/coordinated-backtracking.toml");
    let shared = PackseServer::new("fork/coordinated-backtracking.toml");
    let shared_index = shared.index_url();
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12,<3.14"
        dependencies = [
            "switchable ; python_version < '3.13'",
            "delayed ; python_version >= '3.13'",
        ]

        [tool.uv]
        fork-strategy = "fewest"
        environments = [
            "python_version == '3.12'",
            "python_version == '3.13'",
        ]
        constraint-dependencies = ["shared>=1.0.0"]

        [tool.uv.sources]
        shared = {{ index = "shared" }}

        [[tool.uv.index]]
        name = "shared"
        url = "{shared_index}"
        explicit = true
    "#})?;

    uv_snapshot!(context.filters(), context.lock().arg("--index-url").arg(primary.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 7 packages in [TIME]
    ");

    let locked = context.read("uv.lock");
    assert_snapshot!(registry_choices(&locked, "shared", &[(&shared_index, "shared")])?, @"
    shared==1.0.0 @ shared
    ");
    uv_snapshot!(context.filters(), context.export()
        .args(["--frozen", "--no-header", "--no-hashes", "--no-annotate"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    constrained==1.0.0 ; python_full_version >= '3.13'
    delay-one==1.0.0 ; python_full_version >= '3.13'
    delay-two==1.0.0 ; python_full_version >= '3.13'
    delayed==1.0.0 ; python_full_version >= '3.13'
    shared==1.0.0
    switchable==1.0.0 ; python_full_version < '3.13'
    ");

    uv_snapshot!(context.filters(), context.lock()
        .args(["--locked", "--offline"])
        .arg("--index-url").arg(primary.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 7 packages in [TIME]
    ");
    assert_eq!(locked, context.read("uv.lock"));

    Ok(())
}

/// Identical package names and versions on disjoint explicit indexes can have different
/// dependencies. Neither soft preferences nor hard agreements may cross those registry identities.
#[test]
fn coordinated_scope_disjoint_explicit_indexes() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let primary = PackseServer::new("fork/coordinated-scope-indexes-primary.toml");
    let left = PackseServer::new("fork/coordinated-scope-indexes-left.toml");
    let right = PackseServer::new("fork/coordinated-scope-indexes-right.toml");
    let left_index = left.index_url();
    let right_index = right.index_url();
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12,<3.14"
        dependencies = [
            "flexible ; python_version < '3.13'",
            "delayed ; python_version >= '3.13'",
        ]

        [tool.uv]
        fork-strategy = "fewest"
        environments = [
            "python_version == '3.12'",
            "python_version == '3.13'",
        ]
        constraint-dependencies = ["shared>=1.0.0,<=2.0.0"]

        [tool.uv.sources]
        shared = [
            {{ index = "left", marker = "python_version < '3.13'" }},
            {{ index = "right", marker = "python_version >= '3.13'" }},
        ]

        [[tool.uv.index]]
        name = "left"
        url = "{left_index}"
        explicit = true

        [[tool.uv.index]]
        name = "right"
        url = "{right_index}"
        explicit = true
    "#})?;

    uv_snapshot!(context.filters(), context.lock().arg("--index-url").arg(primary.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 10 packages in [TIME]
    ");

    let locked = context.read("uv.lock");
    assert_snapshot!(registry_choices(&locked, "shared", &[(&left_index, "left"), (&right_index, "right")])?, @"
    shared==1.0.0 @ right
    shared==2.0.0 @ left
    ");
    uv_snapshot!(context.filters(), context.export()
        .args(["--frozen", "--no-header", "--no-hashes", "--no-annotate"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    constrained==1.0.0 ; python_full_version >= '3.13'
    delay-one==1.0.0 ; python_full_version >= '3.13'
    delay-two==1.0.0 ; python_full_version >= '3.13'
    delayed==1.0.0 ; python_full_version >= '3.13'
    flexible==1.0.0 ; python_full_version < '3.13'
    left-only==1.0.0 ; python_full_version < '3.13'
    right-only==1.0.0 ; python_full_version >= '3.13'
    shared==1.0.0 ; python_full_version >= '3.13'
    shared==2.0.0 ; python_full_version < '3.13'
    ");

    uv_snapshot!(context.filters(), context.lock()
        .args(["--locked", "--offline"])
        .arg("--index-url").arg(primary.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 10 packages in [TIME]
    ");
    assert_eq!(locked, context.read("uv.lock"));

    Ok(())
}

/// Conflict-scoped lockfile preferences remain valid when one extra loosens its requirements.
/// Upgrading the shared package can remove those preferences and permit a common version.
#[test]
fn coordinated_scope_conflicting_extra_preferences() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/coordinated-backtracking.toml");
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    let pyproject = indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [project.optional-dependencies]
        early = ["incompatible"]
        late = ["delayed"]

        [tool.uv]
        fork-strategy = "fewest"
        conflicts = [[{ extra = "early" }, { extra = "late" }]]
    "#};
    pyproject_toml.write_str(pyproject)?;

    uv_snapshot!(context.filters(), context.lock().arg("--index-url").arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.export()
        .args(["--frozen", "--no-header", "--no-hashes", "--no-annotate", "--extra", "early"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    incompatible==1.0.0
    shared==2.0.0
    ");
    uv_snapshot!(context.filters(), context.export()
        .args(["--frozen", "--no-header", "--no-hashes", "--no-annotate", "--extra", "late"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    constrained==1.0.0
    delay-one==1.0.0
    delay-two==1.0.0
    delayed==1.0.0
    shared==1.0.0
    ");

    pyproject_toml
        .write_str(&pyproject.replace("early = [\"incompatible\"]", "early = [\"flexible\"]"))?;

    uv_snapshot!(context.filters(), context.lock().arg("--index-url").arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    Added flexible v1.0.0
    Removed incompatible v1.0.0
    ");
    uv_snapshot!(context.filters(), context.export()
        .args(["--frozen", "--no-header", "--no-hashes", "--no-annotate", "--extra", "early"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    flexible==1.0.0
    shared==2.0.0
    ");
    uv_snapshot!(context.filters(), context.export()
        .args(["--frozen", "--no-header", "--no-hashes", "--no-annotate", "--extra", "late"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    constrained==1.0.0
    delay-one==1.0.0
    delay-two==1.0.0
    delayed==1.0.0
    shared==1.0.0
    ");

    let locked = context.read("uv.lock");
    uv_snapshot!(context.filters(), context.lock()
        .args(["--locked", "--offline"])
        .arg("--index-url").arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    ");
    assert_eq!(locked, context.read("uv.lock"));

    uv_snapshot!(context.filters(), context.lock()
        .args(["--upgrade-package", "shared"])
        .arg("--index-url").arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 7 packages in [TIME]
    Updated shared v1.0.0, v2.0.0 -> v1.0.0
    ");
    uv_snapshot!(context.filters(), context.export()
        .args(["--frozen", "--no-header", "--no-hashes", "--no-annotate", "--extra", "early"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    flexible==1.0.0
    shared==1.0.0
    ");
    uv_snapshot!(context.filters(), context.export()
        .args(["--frozen", "--no-header", "--no-hashes", "--no-annotate", "--extra", "late"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    constrained==1.0.0
    delay-one==1.0.0
    delay-two==1.0.0
    delayed==1.0.0
    shared==1.0.0
    ");

    let locked = context.read("uv.lock");
    uv_snapshot!(context.filters(), context.lock()
        .args(["--locked", "--offline"])
        .arg("--index-url").arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 7 packages in [TIME]
    ");
    assert_eq!(locked, context.read("uv.lock"));

    Ok(())
}
