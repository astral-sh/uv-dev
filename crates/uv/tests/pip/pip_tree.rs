#![cfg(not(windows))]

use assert_cmd::assert::OutputAssertExt;
use assert_fs::fixture::FileTouch;
use assert_fs::fixture::FileWriteStr;
use assert_fs::fixture::PathChild;
use assert_fs::fixture::PathCreateDir;
use indoc::indoc;

use uv_test::packse::PackseServer;
use uv_test::uv_snapshot;

#[test]
fn no_package() {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    uv_snapshot!(context.filters(), context.pip_tree(), @"
    exit_code: 0 (success)
    "
    );
}

#[test]
fn prune_last_in_the_subgroup() {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("tree-parent==2.31.0").unwrap();

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 5 packages in [TIME]
    Installed 5 packages in [TIME]
     + tree-leaf-a==2024.2.2
     + tree-leaf-b==3.3.2
     + tree-leaf-c==3.6
     + tree-leaf-d==2.2.1
     + tree-parent==2.31.0
    "
    );

    context.assert_command("import tree_parent").success();
    uv_snapshot!(context.filters(), context.pip_tree().arg("--prune").arg("tree-leaf-a"), @"
    exit_code: 0 (success)
    ----- stdout -----
    tree-parent v2.31.0
    ├── tree-leaf-b v3.3.2
    ├── tree-leaf-c v3.6
    └── tree-leaf-d v2.2.1
    "
    );
}

#[test]
fn single_package() {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("tree-parent==2.31.0").unwrap();

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 5 packages in [TIME]
    Installed 5 packages in [TIME]
     + tree-leaf-a==2024.2.2
     + tree-leaf-b==3.3.2
     + tree-leaf-c==3.6
     + tree-leaf-d==2.2.1
     + tree-parent==2.31.0
    "
    );

    context.assert_command("import tree_parent").success();

    uv_snapshot!(context.filters(), context.pip_tree(), @"
    exit_code: 0 (success)
    ----- stdout -----
    tree-parent v2.31.0
    ├── tree-leaf-a v2024.2.2
    ├── tree-leaf-b v3.3.2
    ├── tree-leaf-c v3.6
    └── tree-leaf-d v2.2.1
    "
    );
}

#[test]
fn nested_dependencies() {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("tree-root").unwrap();

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 7 packages in [TIME]
    Prepared 7 packages in [TIME]
    Installed 7 packages in [TIME]
     + tree-branch-a==1.7.0
     + tree-branch-b==8.1.7
     + tree-branch-c==2.1.2
     + tree-branch-d==3.1.3
     + tree-branch-e==3.0.1
     + tree-root==3.0.2
     + tree-shared-leaf==2.1.5
    "
    );

    uv_snapshot!(context.filters(), context.pip_tree(), @"
    exit_code: 0 (success)
    ----- stdout -----
    tree-root v3.0.2
    ├── tree-branch-a v1.7.0
    ├── tree-branch-b v8.1.7
    ├── tree-branch-c v2.1.2
    ├── tree-branch-d v3.1.3
    │   └── tree-shared-leaf v2.1.5
    └── tree-branch-e v3.0.1
        └── tree-shared-leaf v2.1.5
    "
    );
}

/// Identical test as `invert` since `--reverse` is simply an alias for `--invert`.
#[test]
fn reverse() {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("tree-root").unwrap();

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 7 packages in [TIME]
    Prepared 7 packages in [TIME]
    Installed 7 packages in [TIME]
     + tree-branch-a==1.7.0
     + tree-branch-b==8.1.7
     + tree-branch-c==2.1.2
     + tree-branch-d==3.1.3
     + tree-branch-e==3.0.1
     + tree-root==3.0.2
     + tree-shared-leaf==2.1.5
    "
    );

    uv_snapshot!(context.filters(), context.pip_tree().arg("--reverse"), @"
    exit_code: 0 (success)
    ----- stdout -----
    tree-branch-a v1.7.0
    └── tree-root v3.0.2
    tree-branch-b v8.1.7
    └── tree-root v3.0.2
    tree-branch-c v2.1.2
    └── tree-root v3.0.2
    tree-shared-leaf v2.1.5
    ├── tree-branch-d v3.1.3
    │   └── tree-root v3.0.2
    └── tree-branch-e v3.0.1
        └── tree-root v3.0.2
    "
    );
}

#[test]
fn invert() {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("tree-root").unwrap();

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 7 packages in [TIME]
    Prepared 7 packages in [TIME]
    Installed 7 packages in [TIME]
     + tree-branch-a==1.7.0
     + tree-branch-b==8.1.7
     + tree-branch-c==2.1.2
     + tree-branch-d==3.1.3
     + tree-branch-e==3.0.1
     + tree-root==3.0.2
     + tree-shared-leaf==2.1.5
    "
    );

    uv_snapshot!(context.filters(), context.pip_tree().arg("--invert"), @"
    exit_code: 0 (success)
    ----- stdout -----
    tree-branch-a v1.7.0
    └── tree-root v3.0.2
    tree-branch-b v8.1.7
    └── tree-root v3.0.2
    tree-branch-c v2.1.2
    └── tree-root v3.0.2
    tree-shared-leaf v2.1.5
    ├── tree-branch-d v3.1.3
    │   └── tree-root v3.0.2
    └── tree-branch-e v3.0.1
        └── tree-root v3.0.2
    "
    );
}

#[test]
fn depth() {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("tree-root").unwrap();

    uv_snapshot!(context.pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 7 packages in [TIME]
    Prepared 7 packages in [TIME]
    Installed 7 packages in [TIME]
     + tree-branch-a==1.7.0
     + tree-branch-b==8.1.7
     + tree-branch-c==2.1.2
     + tree-branch-d==3.1.3
     + tree-branch-e==3.0.1
     + tree-root==3.0.2
     + tree-shared-leaf==2.1.5
    "
    );

    uv_snapshot!(context.filters(), context.pip_tree()
        .arg("--depth")
        .arg("0"), @"
    exit_code: 0 (success)
    ----- stdout -----
    tree-root v3.0.2
    "
    );

    uv_snapshot!(context.filters(), context.pip_tree()
        .arg("--depth")
        .arg("1"), @"
    exit_code: 0 (success)
    ----- stdout -----
    tree-root v3.0.2
    ├── tree-branch-a v1.7.0
    ├── tree-branch-b v8.1.7
    ├── tree-branch-c v2.1.2
    ├── tree-branch-d v3.1.3
    └── tree-branch-e v3.0.1
    "
    );

    uv_snapshot!(context.filters(), context.pip_tree()
        .arg("--depth")
        .arg("2"), @"
    exit_code: 0 (success)
    ----- stdout -----
    tree-root v3.0.2
    ├── tree-branch-a v1.7.0
    ├── tree-branch-b v8.1.7
    ├── tree-branch-c v2.1.2
    ├── tree-branch-d v3.1.3
    │   └── tree-shared-leaf v2.1.5
    └── tree-branch-e v3.0.1
        └── tree-shared-leaf v2.1.5
    "
    );
}

#[test]
fn prune() {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("tree-root").unwrap();

    uv_snapshot!(context.pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 7 packages in [TIME]
    Prepared 7 packages in [TIME]
    Installed 7 packages in [TIME]
     + tree-branch-a==1.7.0
     + tree-branch-b==8.1.7
     + tree-branch-c==2.1.2
     + tree-branch-d==3.1.3
     + tree-branch-e==3.0.1
     + tree-root==3.0.2
     + tree-shared-leaf==2.1.5
    "
    );

    uv_snapshot!(context.filters(), context.pip_tree()
        .arg("--prune")
        .arg("tree-branch-e"), @"
    exit_code: 0 (success)
    ----- stdout -----
    tree-root v3.0.2
    ├── tree-branch-a v1.7.0
    ├── tree-branch-b v8.1.7
    ├── tree-branch-c v2.1.2
    └── tree-branch-d v3.1.3
        └── tree-shared-leaf v2.1.5
    "
    );

    uv_snapshot!(context.filters(), context.pip_tree()
        .arg("--prune")
        .arg("tree-branch-e")
        .arg("--prune")
        .arg("tree-branch-d"), @"
    exit_code: 0 (success)
    ----- stdout -----
    tree-root v3.0.2
    ├── tree-branch-a v1.7.0
    ├── tree-branch-b v8.1.7
    └── tree-branch-c v2.1.2
    tree-shared-leaf v2.1.5
    "
    );

    uv_snapshot!(context.filters(), context.pip_tree()
        .arg("--prune")
        .arg("tree-branch-e"), @"
    exit_code: 0 (success)
    ----- stdout -----
    tree-root v3.0.2
    ├── tree-branch-a v1.7.0
    ├── tree-branch-b v8.1.7
    ├── tree-branch-c v2.1.2
    └── tree-branch-d v3.1.3
        └── tree-shared-leaf v2.1.5
    "
    );
}

/// Ensure `pip tree` behaves correctly after a package has been removed.
#[test]
fn removed_dependency() {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("tree-parent==2.31.0").unwrap();

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 5 packages in [TIME]
    Installed 5 packages in [TIME]
     + tree-leaf-a==2024.2.2
     + tree-leaf-b==3.3.2
     + tree-leaf-c==3.6
     + tree-leaf-d==2.2.1
     + tree-parent==2.31.0
    "
    );

    uv_snapshot!(context.filters(), context
        .pip_uninstall()
        .arg("tree-parent"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 1 package in [TIME]
     - tree-parent==2.31.0
    "
    );

    uv_snapshot!(context.filters(), context.pip_tree(), @"
    exit_code: 0 (success)
    ----- stdout -----
    tree-leaf-a v2024.2.2
    tree-leaf-b v3.3.2
    tree-leaf-c v3.6
    tree-leaf-d v2.2.1
    "
    );
}

#[test]
fn multiple_packages() {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str(
            r"
        tree-parent==2.31.0
        tree-branch-b==8.1.7
    ",
        )
        .unwrap();

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 6 packages in [TIME]
    Installed 6 packages in [TIME]
     + tree-branch-b==8.1.7
     + tree-leaf-a==2024.2.2
     + tree-leaf-b==3.3.2
     + tree-leaf-c==3.6
     + tree-leaf-d==2.2.1
     + tree-parent==2.31.0
    "
    );

    context.assert_command("import tree_parent").success();
    uv_snapshot!(context.filters(), context.pip_tree(), @"
    exit_code: 0 (success)
    ----- stdout -----
    tree-branch-b v8.1.7
    tree-parent v2.31.0
    ├── tree-leaf-a v2024.2.2
    ├── tree-leaf-b v3.3.2
    ├── tree-leaf-c v3.6
    └── tree-leaf-d v2.2.1
    "
    );
}

/// Show the installed tree in the presence of a cycle.
#[test]
fn cycle() {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str(
            r"
        cycle-root==2.3.0
        cycle-backref==3.0.0
    ",
        )
        .unwrap();

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 9 packages in [TIME]
    Prepared 9 packages in [TIME]
    Installed 9 packages in [TIME]
     + cycle-backref==3.0.0
     + cycle-leaf-a==1.0.0
     + cycle-leaf-b==6.0.0
     + cycle-leaf-c==1.16.0
     + cycle-leaf-d==1.4.0
     + cycle-leaf-e==1.0.0
     + cycle-nested==1.1.0
     + cycle-root==2.3.0
     + cycle-trace==1.4.0
    "
    );

    uv_snapshot!(context.filters(), context.pip_tree(), @"
    exit_code: 0 (success)
    ----- stdout -----
    cycle-root v2.3.0
    ├── cycle-backref v3.0.0
    │   ├── cycle-leaf-c v1.16.0
    │   └── cycle-root v2.3.0 (*)
    ├── cycle-leaf-a v1.0.0
    ├── cycle-leaf-b v6.0.0
    └── cycle-nested v1.1.0
        ├── cycle-leaf-c v1.16.0
        ├── cycle-leaf-d v1.4.0
        └── cycle-trace v1.4.0
            └── cycle-leaf-e v1.0.0
    (*) Package tree already displayed
    "
    );
}

/// Both `pendulum` and `boto3` depend on `python-dateutil`.
#[test]
fn multiple_packages_shared_descendant() {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str(
            r"
        shared-root
        shared-branch
    ",
        )
        .unwrap();

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 5 packages in [TIME]
    Installed 5 packages in [TIME]
     + shared-bottom==1.16.0
     + shared-branch==2.14.1
     + shared-extra==2024.1
     + shared-leaf==2.9.0
     + shared-root==3.0.0
    "
    );

    uv_snapshot!(context.filters(), context.pip_tree(), @"
    exit_code: 0 (success)
    ----- stdout -----
    shared-root v3.0.0
    ├── shared-branch v2.14.1
    │   └── shared-leaf v2.9.0
    │       └── shared-bottom v1.16.0
    ├── shared-extra v2024.1
    └── shared-leaf v2.9.0 (*)
    (*) Package tree already displayed
    "
    );
}

/// Test the interaction between `--no-dedupe` and `--invert`.
#[test]
fn no_dedupe_and_invert() {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str(
            r"
        shared-root
        shared-branch
    ",
        )
        .unwrap();

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 5 packages in [TIME]
    Installed 5 packages in [TIME]
     + shared-bottom==1.16.0
     + shared-branch==2.14.1
     + shared-extra==2024.1
     + shared-leaf==2.9.0
     + shared-root==3.0.0
    "
    );

    uv_snapshot!(context.filters(), context.pip_tree().arg("--no-dedupe").arg("--invert"), @"
    exit_code: 0 (success)
    ----- stdout -----
    shared-bottom v1.16.0
    └── shared-leaf v2.9.0
        ├── shared-branch v2.14.1
        │   └── shared-root v3.0.0
        └── shared-root v3.0.0
    shared-extra v2024.1
    └── shared-root v3.0.0
    "
    );
}

#[test]
fn no_dedupe() {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str(
            r"
        shared-root
        shared-branch
    ",
        )
        .unwrap();

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 5 packages in [TIME]
    Installed 5 packages in [TIME]
     + shared-bottom==1.16.0
     + shared-branch==2.14.1
     + shared-extra==2024.1
     + shared-leaf==2.9.0
     + shared-root==3.0.0
    "
    );

    uv_snapshot!(context.filters(), context.pip_tree()
        .arg("--no-dedupe"), @"
    exit_code: 0 (success)
    ----- stdout -----
    shared-root v3.0.0
    ├── shared-branch v2.14.1
    │   └── shared-leaf v2.9.0
    │       └── shared-bottom v1.16.0
    ├── shared-extra v2024.1
    └── shared-leaf v2.9.0
        └── shared-bottom v1.16.0
    "
    );
}

#[test]
#[cfg(feature = "test-git")]
fn with_editable() {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    // Install the editable package.
    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg("-e")
        .arg(context.workspace_root.join("test/packages/hatchling_editable")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + hatchling-editable==0.1.0 (from file://[WORKSPACE]/test/packages/hatchling_editable)
     + iniconfig==2.0.1.dev6+g9cae431 (from git+https://github.com/pytest-dev/iniconfig@9cae43103df70bac6fde7b9f35ad11a9f1be0cb4)
    "
    );

    uv_snapshot!(context.filters(), context.pip_tree(), @"
    exit_code: 0 (success)
    ----- stdout -----
    hatchling-editable v0.1.0
    └── iniconfig v2.0.1.dev6+g9cae431
    "
    );
}

#[test]
fn package_flag() {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("tree-root").unwrap();

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 7 packages in [TIME]
    Prepared 7 packages in [TIME]
    Installed 7 packages in [TIME]
     + tree-branch-a==1.7.0
     + tree-branch-b==8.1.7
     + tree-branch-c==2.1.2
     + tree-branch-d==3.1.3
     + tree-branch-e==3.0.1
     + tree-root==3.0.2
     + tree-shared-leaf==2.1.5
    "
    );

    uv_snapshot!(
        context.filters(),
        context.pip_tree()
        .arg("--package")
        .arg("tree-branch-e"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    tree-branch-e v3.0.1
    └── tree-shared-leaf v2.1.5
    "
    );

    uv_snapshot!(
        context.filters(),
        context.pip_tree()
        .arg("--package")
        .arg("tree-branch-e")
        .arg("--package")
        .arg("tree-branch-d"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    tree-branch-d v3.1.3
    └── tree-shared-leaf v2.1.5
    tree-branch-e v3.0.1
    └── tree-shared-leaf v2.1.5
    "
    );
}

#[test]
fn show_version_specifiers_simple() {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("tree-parent==2.31.0").unwrap();

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 5 packages in [TIME]
    Installed 5 packages in [TIME]
     + tree-leaf-a==2024.2.2
     + tree-leaf-b==3.3.2
     + tree-leaf-c==3.6
     + tree-leaf-d==2.2.1
     + tree-parent==2.31.0
    "
    );

    uv_snapshot!(context.filters(), context.pip_tree().arg("--show-version-specifiers"), @"
    exit_code: 0 (success)
    ----- stdout -----
    tree-parent v2.31.0
    ├── tree-leaf-a v2024.2.2 [required: >=2017.4.17]
    ├── tree-leaf-b v3.3.2 [required: >=2, <4]
    ├── tree-leaf-c v3.6 [required: >=2.5, <4]
    └── tree-leaf-d v2.2.1 [required: >=1.21.1, <3]
    "
    );
}

#[test]
fn show_version_specifiers_with_invert() {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("tree-root").unwrap();

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 7 packages in [TIME]
    Prepared 7 packages in [TIME]
    Installed 7 packages in [TIME]
     + tree-branch-a==1.7.0
     + tree-branch-b==8.1.7
     + tree-branch-c==2.1.2
     + tree-branch-d==3.1.3
     + tree-branch-e==3.0.1
     + tree-root==3.0.2
     + tree-shared-leaf==2.1.5
    "
    );

    uv_snapshot!(
        context.filters(),
        context.pip_tree()
        .arg("--show-version-specifiers")
        .arg("--invert"), @"
    exit_code: 0 (success)
    ----- stdout -----
    tree-branch-a v1.7.0
    └── tree-root v3.0.2 [requires: tree-branch-a >=1.6.2]
    tree-branch-b v8.1.7
    └── tree-root v3.0.2 [requires: tree-branch-b >=8.1.3]
    tree-branch-c v2.1.2
    └── tree-root v3.0.2 [requires: tree-branch-c >=2.1.2]
    tree-shared-leaf v2.1.5
    ├── tree-branch-d v3.1.3 [requires: tree-shared-leaf >=2.0]
    │   └── tree-root v3.0.2 [requires: tree-branch-d >=3.1.2]
    └── tree-branch-e v3.0.1 [requires: tree-shared-leaf >=2.1.1]
        └── tree-root v3.0.2 [requires: tree-branch-e >=3.0.0]
    "
    );
}

#[test]
fn show_version_specifiers_with_package() {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("tree-root").unwrap();

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 7 packages in [TIME]
    Prepared 7 packages in [TIME]
    Installed 7 packages in [TIME]
     + tree-branch-a==1.7.0
     + tree-branch-b==8.1.7
     + tree-branch-c==2.1.2
     + tree-branch-d==3.1.3
     + tree-branch-e==3.0.1
     + tree-root==3.0.2
     + tree-shared-leaf==2.1.5
    "
    );

    uv_snapshot!(
        context.filters(),
        context.pip_tree()
        .arg("--show-version-specifiers")
        .arg("--package")
        .arg("tree-branch-e"), @"
    exit_code: 0 (success)
    ----- stdout -----
    tree-branch-e v3.0.1
    └── tree-shared-leaf v2.1.5 [required: >=2.1.1]
    "
    );
}

#[test]
fn print_output_even_with_quite_flag() {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("tree-parent==2.31.0").unwrap();

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 5 packages in [TIME]
    Installed 5 packages in [TIME]
     + tree-leaf-a==2024.2.2
     + tree-leaf-b==3.3.2
     + tree-leaf-c==3.6
     + tree-leaf-d==2.2.1
     + tree-parent==2.31.0
    "
    );

    context.assert_command("import tree_parent").success();
    uv_snapshot!(context.filters(), context.pip_tree().arg("--quiet"), @"
    exit_code: 0 (success)
    "
    );
}

#[test]
fn outdated() {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("tree-root==2.0.0").unwrap();

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 6 packages in [TIME]
    Installed 6 packages in [TIME]
     + tree-branch-b==8.1.7
     + tree-branch-c==2.1.2
     + tree-branch-d==3.1.3
     + tree-branch-e==3.0.1
     + tree-root==2.0.0
     + tree-shared-leaf==2.1.5
    "
    );

    uv_snapshot!(
        context.filters(),
        context.pip_tree().arg("--outdated"), @"
    exit_code: 0 (success)
    ----- stdout -----
    tree-root v2.0.0 (latest: v3.0.2)
    ├── tree-branch-b v8.1.7
    ├── tree-branch-c v2.1.2
    ├── tree-branch-d v3.1.3
    │   └── tree-shared-leaf v2.1.5
    └── tree-branch-e v3.0.1
        └── tree-shared-leaf v2.1.5
    "
    );
}

/// Test that dependencies with multiple marker-specific requirements
/// are only displayed once in the tree.
#[test]
fn no_duplicate_dependencies_with_markers() {
    const PY_PROJECT: &str = indoc! {r#"
        [project]
        name = "debug"
        version = "0.1.0"
        requires-python = ">=3.12.0"
        dependencies = [
          "marker-child>=1.0.0; python_version >= '3.11'",
          "marker-child>=1.0.1; python_version >= '3.12'",
          "marker-child>=1.0.2; python_version >= '3.13'",
        ]

        [build-system]
        requires = ["uv_build>=0.8.22,<10000"]
        build-backend = "uv_build"
    "#};

    let server = PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"])
        .with_default_index(&server.index_url())
        .with_filtered_counts();

    let project = context.temp_dir.child("debug");

    project.create_dir_all().unwrap();

    project.child("src/debug").create_dir_all().unwrap();

    project.child("src/debug/__init__.py").touch().unwrap();

    project
        .child("pyproject.toml")
        .write_str(PY_PROJECT)
        .unwrap();

    context.reset_venv();

    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg(project.path())
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + debug==0.1.0 (from file://[TEMP_DIR]/debug)
     + marker-child==1.3.1
    "
    );

    // Ensure that the dependency is only listed once, even though `debug` declares multiple
    // marker-specific requirements for the same dependency.
    uv_snapshot!(context.filters(), context.pip_tree(), @"
    exit_code: 0 (success)
    ----- stdout -----
    debug v0.1.0
    └── marker-child v1.3.1
    "
    );

    uv_snapshot!(
        context.filters(),
        context.pip_tree().arg("--show-version-specifiers"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    debug v0.1.0
    └── marker-child v1.3.1 [required: >=1.0.1]
    "
    );

    context
        .venv()
        .arg("--clear")
        .arg("--python")
        .arg("3.13")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg(project.path())
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + debug==0.1.0 (from file://[TEMP_DIR]/debug)
     + marker-child==1.3.1
    "
    );

    uv_snapshot!(
        context.filters(),
        context.pip_tree().arg("--show-version-specifiers"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    debug v0.1.0
    └── marker-child v1.3.1 [required: >=1.0.2]
    "
    );
}
