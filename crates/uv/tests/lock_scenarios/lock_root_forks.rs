use anyhow::Result;
use assert_fs::prelude::*;
use insta::assert_json_snapshot;

use uv_test::packse::PackseServer;
use uv_test::packse::scenario::Scenario;
use uv_test::uv_snapshot;

/// A transitive workspace member uses its parent's solve, not its standalone solve.
#[test]
fn workspace_roots_resolve_independently() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let scenario = toml::from_str::<Scenario>(
        r#"
        name = "independent-workspace-roots"
        [root]
        [expected]
        satisfiable = true
        [packages.shared-leaf.versions."1.0.0"]
        sdist = false
        [packages.shared-leaf.versions."2.0.0"]
        sdist = false
    "#,
    )?;
    let server = PackseServer::from_scenario(&scenario);
    context.temp_dir.child("pyproject.toml").write_str(
        r#"
        [tool.uv.workspace]
        members = ["members/*"]
        roots = ["root-a", "root-b", "root-c", "root-d"]
        [tool.uv.sources]
        root-a = { workspace = true }
        root-b = { workspace = true }
    "#,
    )?;
    for (name, dependencies) in [
        ("root-a", r#"["shared-leaf>=1,<3"]"#),
        ("root-b", r#"["root-a", "shared-leaf==1"]"#),
        ("root-c", r#"["shared-leaf==1"]"#),
        ("root-d", r#"["root-a", "root-b"]"#),
        ("unused", r#"["missing-package==1"]"#),
    ] {
        context
            .temp_dir
            .child("members")
            .child(name)
            .child("pyproject.toml")
            .write_str(&format!(
                r#"
                [project]
                name = "{name}"
                version = "0.1.0"
                requires-python = ">=3.12"
                dependencies = {dependencies}
                [tool.uv]
                package = false
            "#
            ))?;
    }
    uv_snapshot!(context.filters(), context.lock().arg("--index-url").arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.lock().arg("--locked").arg("--index-url").arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    ");
    let lock: toml::Value = toml::from_str(&context.read("uv.lock"))?;
    let packages = lock["package"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|package| {
            Some((
                package.get("name")?.as_str()?,
                package.get("version")?.as_str()?,
            ))
        })
        .collect::<Vec<_>>();
    assert_json_snapshot!(packages, @r#"
    [
      [
        "root-a",
        "0.1.0"
      ],
      [
        "root-b",
        "0.1.0"
      ],
      [
        "root-c",
        "0.1.0"
      ],
      [
        "root-d",
        "0.1.0"
      ],
      [
        "shared-leaf",
        "1.0.0"
      ],
      [
        "shared-leaf",
        "2.0.0"
      ]
    ]
    "#);
    uv_snapshot!(context.filters(), context.export()
        .arg("--frozen").arg("--package").arg("root-a")
        .arg("--no-header").arg("--no-hashes").arg("--no-annotate"), @"
    exit_code: 0 (success)
    ----- stdout -----
    shared-leaf==2.0.0
    ");
    uv_snapshot!(context.filters(), context.export()
        .arg("--frozen").arg("--package").arg("root-b")
        .arg("--no-header").arg("--no-hashes").arg("--no-annotate"), @"
    exit_code: 0 (success)
    ----- stdout -----
    shared-leaf==1.0.0
    ");
    uv_snapshot!(context.filters(), context.export()
        .arg("--frozen").arg("--package").arg("root-d")
        .arg("--no-header").arg("--no-hashes").arg("--no-annotate"), @"
    exit_code: 0 (success)
    ----- stdout -----
    shared-leaf==1.0.0
    ");
    uv_snapshot!(context.filters(), context.tree().arg("--frozen").arg("--package").arg("root-b"), @"
    exit_code: 0 (success)
    ----- stdout -----
    root-b v0.1.0
    ├── root-a v0.1.0
    │   └── shared-leaf v1.0.0
    └── shared-leaf v1.0.0
    ");
    uv_snapshot!(context.filters(), context.sync().arg("--frozen").arg("--package").arg("root-b"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + shared-leaf==1.0.0
    ");
    uv_snapshot!(context.filters(), context.sync().arg("--frozen").arg("--package").arg("root-a"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - shared-leaf==1.0.0
     + shared-leaf==2.0.0
    ");
    uv_snapshot!(context.filters(), context.sync().arg("--frozen")
        .arg("--package").arg("root-b").arg("--package").arg("root-c"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - shared-leaf==2.0.0
     + shared-leaf==1.0.0
    ");
    uv_snapshot!(context.filters(), context.sync().arg("--frozen")
        .arg("--package").arg("root-a").arg("--package").arg("root-b"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The selected workspace roots require incompatible locked packages `shared-leaf==1.0.0 @ registry+http://[LOCALHOST]/simple/` and `shared-leaf==2.0.0 @ registry+http://[LOCALHOST]/simple/`. Select one root, or add a workspace root that depends on both to resolve them together.
    ");
    uv_snapshot!(context.filters(), context.export().arg("--frozen")
        .arg("--package").arg("root-a").arg("--package").arg("root-b")
        .arg("--no-header").arg("--no-hashes").arg("--no-annotate"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The selected workspace roots require incompatible locked packages `shared-leaf==1.0.0 @ registry+http://[LOCALHOST]/simple/` and `shared-leaf==2.0.0 @ registry+http://[LOCALHOST]/simple/`. Select one root, or add a workspace root that depends on both to resolve them together.
    ");
    Ok(())
}

/// Earlier root choices are preferences, not constraints on later roots.
#[test]
fn workspace_roots_share_version_preferences() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let scenario = toml::from_str::<Scenario>(
        r#"
        name = "workspace-root-preferences"
        [root]
        [expected]
        satisfiable = true
        [packages.shared-leaf.versions."1.0.0"]
        sdist = false
        [packages.shared-leaf.versions."2.0.0"]
        sdist = false
    "#,
    )?;
    let server = PackseServer::from_scenario(&scenario);
    context.temp_dir.child("pyproject.toml").write_str(
        r#"
        [tool.uv.workspace]
        members = ["members/*"]
        roots = ["root-a", "root-b", "root-c"]
    "#,
    )?;
    for (name, dependency) in [
        ("root-a", "shared-leaf==1"),
        ("root-b", "shared-leaf>=1,<3"),
        ("root-c", "shared-leaf==2"),
    ] {
        context
            .temp_dir
            .child("members")
            .child(name)
            .child("pyproject.toml")
            .write_str(&format!(
                r#"
                [project]
                name = "{name}"
                version = "0.1.0"
                requires-python = ">=3.12"
                dependencies = ["{dependency}"]
                [tool.uv]
                package = false
            "#
            ))?;
    }
    uv_snapshot!(context.filters(), context.lock().arg("--index-url").arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.export().arg("--frozen").arg("--package").arg("root-b")
        .arg("--no-header").arg("--no-hashes").arg("--no-annotate"), @"
    exit_code: 0 (success)
    ----- stdout -----
    shared-leaf==1.0.0
    ");
    uv_snapshot!(context.filters(), context.export().arg("--frozen").arg("--package").arg("root-c")
        .arg("--no-header").arg("--no-hashes").arg("--no-annotate"), @"
    exit_code: 0 (success)
    ----- stdout -----
    shared-leaf==2.0.0
    ");
    uv_snapshot!(context.filters(), context.lock().arg("--locked").arg("--index-url").arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    ");
    Ok(())
}

#[test]
fn workspace_root_required_for_standalone_selection() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context.temp_dir.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "root-a"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["member"]
        [tool.uv]
        package = false
        [tool.uv.workspace]
        members = ["member"]
        roots = ["root-a"]
        [tool.uv.sources]
        member = { workspace = true }
    "#,
    )?;
    context.temp_dir.child("member/pyproject.toml").write_str(
        r#"
        [project]
        name = "member"
        version = "0.1.0"
        requires-python = ">=3.12"
        [tool.uv]
        package = false
    "#,
    )?;
    uv_snapshot!(context.filters(), context.lock().arg("--no-index"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.sync().arg("--frozen").arg("--package").arg("member"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Workspace member `member` has no independent locked resolution. Add it to `tool.uv.workspace.roots` and run `uv lock`.
    ");
    uv_snapshot!(context.filters(), context.export().arg("--frozen").arg("--package").arg("member")
        .arg("--no-header").arg("--no-hashes").arg("--no-annotate"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Workspace member `member` has no independent locked resolution. Add it to `tool.uv.workspace.roots` and run `uv lock`.
    ");
    Ok(())
}
