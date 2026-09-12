use std::process::Command;

use anyhow::{Result, anyhow};
use assert_cmd::prelude::*;
use assert_fs::prelude::*;
use indoc::{formatdoc, indoc};
use insta::assert_snapshot;
use url::Url;

use uv_test::{TestContext, apply_filters, make_project, uv_snapshot};

fn git_package_url(context: &TestContext) -> Result<Url> {
    let repository = context.temp_dir.child("git-branch");
    repository.child("src/git_branch").create_dir_all()?;
    repository
        .child("src/git_branch/__init__.py")
        .write_str("__version__ = \"2.0.0\"\n")?;
    repository.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "git-branch"
        version = "2.0.0"
        requires-python = ">=3.11"

        [build-system]
        requires = ["uv_build>=0.9,<10000"]
        build-backend = "uv_build"
    "#})?;

    Command::new("git")
        .arg("init")
        .arg(repository.path())
        .assert()
        .success();
    Command::new("git")
        .arg("-C")
        .arg(repository.path())
        .arg("add")
        .arg(".")
        .assert()
        .success();
    Command::new("git")
        .arg("-C")
        .arg(repository.path())
        .arg("-c")
        .arg("user.name=Example")
        .arg("-c")
        .arg("user.email=example@example.com")
        .arg("commit")
        .arg("-m")
        .arg("Initial commit")
        .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z")
        .assert()
        .success();

    Url::from_directory_path(repository.path())
        .map_err(|()| anyhow!("failed to convert git package path to a file URL"))
}

/// The root package has diverging URLs for disjoint markers:
/// ```toml
/// dependencies = [
///   "iniconfig @ https://files.pythonhosted.org/packages/ef/a6/62565a6e1cf69e10f5727360368e451d4b7f58beeac6173dc9db836a5b46/iniconfig-2.0.0-py3-none-any.whl ; python_version >= '3.12'",
///   "iniconfig @ https://files.pythonhosted.org/packages/9b/dd/b3c12c6d707058fa947864b67f0c4e0c39ef8610988d7baea9578f3c48f3/iniconfig-1.1.1-py2.py3-none-any.whl ; python_version < '3.12'",
/// ]
/// ```
#[test]
fn branching_urls_disjoint() -> Result<()> {
    let server = uv_test::packse::PackseServer::new("packages/branching-urls.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&server.index_url());

    let deps = formatdoc! {r#"
        dependencies = [
            # Valid, disjoint split
            "branch-url @ {branch_url_1} ; python_version < '3.12'",
            "branch-url @ {branch_url_2} ; python_version >= '3.12'",
        ]
    "#,
        branch_url_1 = server.file_url("branch_url-1.1.1-py3-none-any.whl"),
        branch_url_2 = server.file_url("branch_url-2.0.0-py3-none-any.whl"),
    };
    make_project(context.temp_dir.path(), "a", &deps)?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    "
    );

    Ok(())
}

/// The root package has diverging URLs, but their markers are not disjoint:
/// ```toml
/// dependencies = [
///   "iniconfig @ https://files.pythonhosted.org/packages/ef/a6/62565a6e1cf69e10f5727360368e451d4b7f58beeac6173dc9db836a5b46/iniconfig-2.0.0-py3-none-any.whl ; python_version >= '3.11'",
///   "iniconfig @ https://files.pythonhosted.org/packages/9b/dd/b3c12c6d707058fa947864b67f0c4e0c39ef8610988d7baea9578f3c48f3/iniconfig-1.1.1-py2.py3-none-any.whl ; python_version < '3.12'",
/// ]
/// ```
#[test]
fn branching_urls_overlapping() -> Result<()> {
    let server = uv_test::packse::PackseServer::new("packages/branching-urls.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&server.index_url());

    let deps = formatdoc! {r#"
        dependencies = [
            # Conflicting split
            "branch-url @ {branch_url_1} ; python_version < '3.12'",
            "branch-url @ {branch_url_2} ; python_version >= '3.11'",
        ]
    "#,
        branch_url_1 = server.file_url("branch_url-1.1.1-py3-none-any.whl"),
        branch_url_2 = server.file_url("branch_url-2.0.0-py3-none-any.whl"),
    };
    make_project(context.temp_dir.path(), "a", &deps)?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to resolve dependencies for package `a==0.1.0`
      cause: Requirements contain conflicting URLs for package `branch-url` in split `python_full_version == '3.11.*'`:
             - http://[LOCALHOST]/files/branch_url-1.1.1-py3-none-any.whl
             - http://[LOCALHOST]/files/branch_url-2.0.0-py3-none-any.whl
    "
    );

    Ok(())
}

/// The root package has diverging URLs, but transitive dependencies have conflicting URLs.
///
/// Requirements:
/// ```text
/// a -> anyio (allowed forking urls to force a split)
/// a -> b -> b1 -> https://../iniconfig-1.1.1-py3-none-any.whl
/// a -> b -> b2 -> https://../iniconfig-2.0.0-py3-none-any.whl
/// ```
#[test]
fn root_package_splits_but_transitive_conflict() -> Result<()> {
    let server = uv_test::packse::PackseServer::new("packages/branching-urls.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&server.index_url());

    let deps = indoc! {r#"
        dependencies = [
            # Force a split
            "branch-splitter==4.3.0 ; python_version >= '3.12'",
            "branch-splitter==4.2.0 ; python_version < '3.12'",
            "b"
        ]

        [tool.uv.sources]
        b = { path = "b" }
    "# };
    make_project(context.temp_dir.path(), "a", deps)?;

    let deps = indoc! {r#"
        dependencies = [
            "b1",
            "b2",
        ]

        [tool.uv.sources]
        b1 = { path = "../b1" }
        b2 = { path = "../b2" }
    "# };
    make_project(&context.temp_dir.path().join("b"), "b", deps)?;

    let deps = formatdoc! {r#"
        dependencies = [
            "branch-url @ {branch_url}",
        ]
    "#,
        branch_url = server.file_url("branch_url-1.1.1-py3-none-any.whl"),
    };
    make_project(&context.temp_dir.path().join("b1"), "b1", &deps)?;

    let deps = formatdoc! {r#"
        dependencies = [
            "branch-url @ {branch_url}",
        ]
    "#,
        branch_url = server.file_url("branch_url-2.0.0-py3-none-any.whl"),
    };
    make_project(&context.temp_dir.path().join("b2"), "b2", &deps)?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to resolve dependencies for package `b2==0.1.0`
      cause: Requirements contain conflicting URLs for package `branch-url` in split `python_full_version >= '3.12'`:
             - http://[LOCALHOST]/files/branch_url-1.1.1-py3-none-any.whl
             - http://[LOCALHOST]/files/branch_url-2.0.0-py3-none-any.whl

    hint: `b2` (v0.1.0) was included because `a` (v0.1.0) depends on `b` (v0.1.0) which depends on `b2`
    "
    );

    Ok(())
}

/// The root package has diverging URLs, and transitive dependencies through an intermediate
/// package have one URL for each side.
///
/// Requirements:
/// ```text
/// a -> anyio==4.3.0 ; python_version >= '3.12'
///  a -> anyio==4.2.0 ; python_version < '3.12'
/// a -> b -> b1 ; python_version < '3.12' -> https://../iniconfig-1.1.1-py3-none-any.whl
/// a -> b -> b2 ; python_version >= '3.12' -> https://../iniconfig-2.0.0-py3-none-any.whl
/// ```
#[test]
fn root_package_splits_transitive_too() -> Result<()> {
    let server = uv_test::packse::PackseServer::new("packages/branching-urls.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&server.index_url());

    let deps = indoc! {r#"
        dependencies = [
            # Force a split
            "branch-splitter==4.3.0 ; python_version >= '3.12'",
            "branch-splitter==4.2.0 ; python_version < '3.12'",
            "b"
        ]

        [tool.uv.sources]
        b = { path = "b" }
    "# };
    make_project(context.temp_dir.path(), "a", deps)?;

    let deps = indoc! {r#"
        dependencies = [
            "b1 ; python_version < '3.12'",
            "b2 ; python_version >= '3.12'",
        ]

        [tool.uv.sources]
        b1 = { path = "../b1" }
        b2 = { path = "../b2" }
    "# };
    make_project(&context.temp_dir.path().join("b"), "b", deps)?;

    let deps = formatdoc! {r#"
        dependencies = [
            "branch-url @ {branch_url}",
        ]
    "#,
        branch_url = server.file_url("branch_url-1.1.1-py3-none-any.whl"),
    };
    make_project(&context.temp_dir.path().join("b1"), "b1", &deps)?;

    let deps = formatdoc! {r#"
        dependencies = [
            "branch-url @ {branch_url}",
        ]
    "#,
        branch_url = server.file_url("branch_url-2.0.0-py3-none-any.whl"),
    };
    make_project(&context.temp_dir.path().join("b2"), "b2", &deps)?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    "
    );

    assert_snapshot!(apply_filters(context.read("uv.lock"), context.filters()), @r#"
    version = 1
    revision = 3
    requires-python = ">=3.11, <3.13"
    resolution-markers = [
        "python_full_version >= '3.12'",
        "python_full_version < '3.12'",
    ]

    [options]
    exclude-newer = "2024-03-25T00:00:00Z"

    [[package]]
    name = "a"
    version = "0.1.0"
    source = { editable = "." }
    dependencies = [
        { name = "b" },
        { name = "branch-splitter", version = "4.2.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "python_full_version < '3.12'" },
        { name = "branch-splitter", version = "4.3.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "python_full_version >= '3.12'" },
    ]

    [package.metadata]
    requires-dist = [
        { name = "b", directory = "b" },
        { name = "branch-splitter", marker = "python_full_version < '3.12'", specifier = "==4.2.0" },
        { name = "branch-splitter", marker = "python_full_version >= '3.12'", specifier = "==4.3.0" },
    ]

    [[package]]
    name = "b"
    version = "0.1.0"
    source = { directory = "b" }
    dependencies = [
        { name = "b1", marker = "python_full_version < '3.12'" },
        { name = "b2", marker = "python_full_version >= '3.12'" },
    ]

    [package.metadata]
    requires-dist = [
        { name = "b1", marker = "python_full_version < '3.12'", directory = "b1" },
        { name = "b2", marker = "python_full_version >= '3.12'", directory = "b2" },
    ]

    [[package]]
    name = "b1"
    version = "0.1.0"
    source = { directory = "b1" }
    dependencies = [
        { name = "branch-url", version = "1.1.1", source = { url = "http://[LOCALHOST]/files/branch_url-1.1.1-py3-none-any.whl" } },
    ]

    [package.metadata]
    requires-dist = [{ name = "branch-url", url = "http://[LOCALHOST]/files/branch_url-1.1.1-py3-none-any.whl" }]

    [[package]]
    name = "b2"
    version = "0.1.0"
    source = { directory = "b2" }
    dependencies = [
        { name = "branch-url", version = "2.0.0", source = { url = "http://[LOCALHOST]/files/branch_url-2.0.0-py3-none-any.whl" } },
    ]

    [package.metadata]
    requires-dist = [{ name = "branch-url", url = "http://[LOCALHOST]/files/branch_url-2.0.0-py3-none-any.whl" }]

    [[package]]
    name = "branch-splitter"
    version = "4.2.0"
    source = { registry = "http://[LOCALHOST]/simple/" }
    resolution-markers = [
        "python_full_version < '3.12'",
    ]
    sdist = { url = "http://[LOCALHOST]/files/branch_splitter-4.2.0.tar.gz", hash = "sha256:1fd4ba13cae1b8de1f851e4081da4cb1659bd7a5fdaf233b526a0d4538b5f0bd", upload-time = "2024-03-24T00:00:00Z" }
    wheels = [
        { url = "http://[LOCALHOST]/files/branch_splitter-4.2.0-py3-none-any.whl", hash = "sha256:2a0858629ee7c581366d70d4abba1a003cb879eb5a1f20bee029a80b40d6ec4e", upload-time = "2024-03-24T00:00:00Z" },
    ]

    [[package]]
    name = "branch-splitter"
    version = "4.3.0"
    source = { registry = "http://[LOCALHOST]/simple/" }
    resolution-markers = [
        "python_full_version >= '3.12'",
    ]
    sdist = { url = "http://[LOCALHOST]/files/branch_splitter-4.3.0.tar.gz", hash = "sha256:ce3fcb5cdb2a0a60523cf91c7ed3029129ca2d17b8ec45f51f247bdb2d0f1c49", upload-time = "2024-03-24T00:00:00Z" }
    wheels = [
        { url = "http://[LOCALHOST]/files/branch_splitter-4.3.0-py3-none-any.whl", hash = "sha256:14da5de6e342e8547a7fd8962fd9ac657c4c321b693d42668b157557920d1016", upload-time = "2024-03-24T00:00:00Z" },
    ]

    [[package]]
    name = "branch-url"
    version = "1.1.1"
    source = { url = "http://[LOCALHOST]/files/branch_url-1.1.1-py3-none-any.whl" }
    resolution-markers = [
        "python_full_version < '3.12'",
    ]
    wheels = [
        { url = "http://[LOCALHOST]/files/branch_url-1.1.1-py3-none-any.whl", hash = "sha256:859128ee0d6d0248a1170ee4030d342d23d20f5d8840b9568fb5802f9f0912dd" },
    ]

    [[package]]
    name = "branch-url"
    version = "2.0.0"
    source = { url = "http://[LOCALHOST]/files/branch_url-2.0.0-py3-none-any.whl" }
    resolution-markers = [
        "python_full_version >= '3.12'",
    ]
    wheels = [
        { url = "http://[LOCALHOST]/files/branch_url-2.0.0-py3-none-any.whl", hash = "sha256:a6ec1b64c087a48cbd93ff6424a689588a48c35239c918ed85f67d299e7ea79d" },
    ]
    "#);

    Ok(())
}

/// The root package has diverging URLs on one package, and other dependencies have one URL
/// for each side.
///
/// Requirements:
/// ```
/// a -> anyio==4.3.0 ; python_version >= '3.12'
/// a -> anyio==4.2.0 ; python_version < '3.12'
/// a -> b1 ; python_version < '3.12' -> iniconfig==1.1.1
/// a -> b2 ; python_version >= '3.12' -> iniconfig==2.0.0
/// ```
#[test]
fn root_package_splits_other_dependencies_too() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/branching-urls.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let deps = indoc! {r#"
        dependencies = [
            # Force a split
            "branch-splitter==4.3.0 ; python_version >= '3.12'",
            "branch-splitter==4.2.0 ; python_version < '3.12'",
            # These two are currently included in both parts of the split.
            "b1 ; python_version < '3.12'",
            "b2 ; python_version >= '3.12'",
        ]

        [tool.uv.sources]
        b1 = { path = "b1" }
        b2 = { path = "b2" }
    "# };
    make_project(context.temp_dir.path(), "a", deps)?;

    let deps = indoc! {r#"
        dependencies = [
            "branch-url==1.1.1",
        ]
    "# };
    make_project(&context.temp_dir.path().join("b1"), "b1", deps)?;

    let deps = indoc! {r#"
        dependencies = [
            "branch-url==2.0.0"
        ]
    "# };
    make_project(&context.temp_dir.path().join("b2"), "b2", deps)?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 7 packages in [TIME]
    "
    );

    assert_snapshot!(apply_filters(context.read("uv.lock"), context.filters()), @r#"
    version = 1
    revision = 3
    requires-python = ">=3.11, <3.13"
    resolution-markers = [
        "python_full_version >= '3.12'",
        "python_full_version < '3.12'",
    ]

    [options]
    exclude-newer = "2024-03-25T00:00:00Z"

    [[package]]
    name = "a"
    version = "0.1.0"
    source = { editable = "." }
    dependencies = [
        { name = "b1", marker = "python_full_version < '3.12'" },
        { name = "b2", marker = "python_full_version >= '3.12'" },
        { name = "branch-splitter", version = "4.2.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "python_full_version < '3.12'" },
        { name = "branch-splitter", version = "4.3.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "python_full_version >= '3.12'" },
    ]

    [package.metadata]
    requires-dist = [
        { name = "b1", marker = "python_full_version < '3.12'", directory = "b1" },
        { name = "b2", marker = "python_full_version >= '3.12'", directory = "b2" },
        { name = "branch-splitter", marker = "python_full_version < '3.12'", specifier = "==4.2.0" },
        { name = "branch-splitter", marker = "python_full_version >= '3.12'", specifier = "==4.3.0" },
    ]

    [[package]]
    name = "b1"
    version = "0.1.0"
    source = { directory = "b1" }
    dependencies = [
        { name = "branch-url", version = "1.1.1", source = { registry = "http://[LOCALHOST]/simple/" } },
    ]

    [package.metadata]
    requires-dist = [{ name = "branch-url", specifier = "==1.1.1" }]

    [[package]]
    name = "b2"
    version = "0.1.0"
    source = { directory = "b2" }
    dependencies = [
        { name = "branch-url", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" } },
    ]

    [package.metadata]
    requires-dist = [{ name = "branch-url", specifier = "==2.0.0" }]

    [[package]]
    name = "branch-splitter"
    version = "4.2.0"
    source = { registry = "http://[LOCALHOST]/simple/" }
    resolution-markers = [
        "python_full_version < '3.12'",
    ]
    sdist = { url = "http://[LOCALHOST]/files/branch_splitter-4.2.0.tar.gz", hash = "sha256:1fd4ba13cae1b8de1f851e4081da4cb1659bd7a5fdaf233b526a0d4538b5f0bd", upload-time = "2024-03-24T00:00:00Z" }
    wheels = [
        { url = "http://[LOCALHOST]/files/branch_splitter-4.2.0-py3-none-any.whl", hash = "sha256:2a0858629ee7c581366d70d4abba1a003cb879eb5a1f20bee029a80b40d6ec4e", upload-time = "2024-03-24T00:00:00Z" },
    ]

    [[package]]
    name = "branch-splitter"
    version = "4.3.0"
    source = { registry = "http://[LOCALHOST]/simple/" }
    resolution-markers = [
        "python_full_version >= '3.12'",
    ]
    sdist = { url = "http://[LOCALHOST]/files/branch_splitter-4.3.0.tar.gz", hash = "sha256:ce3fcb5cdb2a0a60523cf91c7ed3029129ca2d17b8ec45f51f247bdb2d0f1c49", upload-time = "2024-03-24T00:00:00Z" }
    wheels = [
        { url = "http://[LOCALHOST]/files/branch_splitter-4.3.0-py3-none-any.whl", hash = "sha256:14da5de6e342e8547a7fd8962fd9ac657c4c321b693d42668b157557920d1016", upload-time = "2024-03-24T00:00:00Z" },
    ]

    [[package]]
    name = "branch-url"
    version = "1.1.1"
    source = { registry = "http://[LOCALHOST]/simple/" }
    resolution-markers = [
        "python_full_version < '3.12'",
    ]
    sdist = { url = "http://[LOCALHOST]/files/branch_url-1.1.1.tar.gz", hash = "sha256:0beac3f2ad3ee94ec48aab67e9b43f48047961e1481c29940aaf6165c5b9f80b", upload-time = "2024-03-24T00:00:00Z" }
    wheels = [
        { url = "http://[LOCALHOST]/files/branch_url-1.1.1-py3-none-any.whl", hash = "sha256:859128ee0d6d0248a1170ee4030d342d23d20f5d8840b9568fb5802f9f0912dd", upload-time = "2024-03-24T00:00:00Z" },
    ]

    [[package]]
    name = "branch-url"
    version = "2.0.0"
    source = { registry = "http://[LOCALHOST]/simple/" }
    resolution-markers = [
        "python_full_version >= '3.12'",
    ]
    sdist = { url = "http://[LOCALHOST]/files/branch_url-2.0.0.tar.gz", hash = "sha256:3f62663c8f332d34d580019d403a62f7b7962cdc39d5a7885b0c1b62ee22bf66", upload-time = "2024-03-24T00:00:00Z" }
    wheels = [
        { url = "http://[LOCALHOST]/files/branch_url-2.0.0-py3-none-any.whl", hash = "sha256:a6ec1b64c087a48cbd93ff6424a689588a48c35239c918ed85f67d299e7ea79d", upload-time = "2024-03-24T00:00:00Z" },
    ]
    "#);

    Ok(())
}

/// Whether the dependency comes from the registry or a direct URL depends on the branch.
///
/// ```toml
/// dependencies = [
///   "iniconfig == 1.1.1 ; python_version < '3.12'",
///   "iniconfig @ https://files.pythonhosted.org/packages/ef/a6/62565a6e1cf69e10f5727360368e451d4b7f58beeac6173dc9db836a5b46/iniconfig-2.0.0-py3-none-any.whl ; python_version >= '3.12'",
/// ]
/// ```
#[test]
fn branching_between_registry_and_direct_url() -> Result<()> {
    let server = uv_test::packse::PackseServer::new("packages/branching-urls.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&server.index_url());

    let deps = formatdoc! {r#"
        dependencies = [
            "branch-url == 1.1.1 ; python_version < '3.12'",
            "branch-url @ {branch_url} ; python_version >= '3.12'",
        ]
    "#,
        branch_url = server.file_url("branch_url-2.0.0-py3-none-any.whl"),
    };
    make_project(context.temp_dir.path(), "a", &deps)?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    "
    );

    // We have source dist and wheel for the registry, but only the wheel for the direct URL.
    assert_snapshot!(apply_filters(context.read("uv.lock"), context.filters()), @r#"
    version = 1
    revision = 3
    requires-python = ">=3.11, <3.13"
    resolution-markers = [
        "python_full_version >= '3.12'",
        "python_full_version < '3.12'",
    ]

    [options]
    exclude-newer = "2024-03-25T00:00:00Z"

    [[package]]
    name = "a"
    version = "0.1.0"
    source = { editable = "." }
    dependencies = [
        { name = "branch-url", version = "1.1.1", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "python_full_version < '3.12'" },
        { name = "branch-url", version = "2.0.0", source = { url = "http://[LOCALHOST]/files/branch_url-2.0.0-py3-none-any.whl" }, marker = "python_full_version >= '3.12'" },
    ]

    [package.metadata]
    requires-dist = [
        { name = "branch-url", marker = "python_full_version < '3.12'", specifier = "==1.1.1" },
        { name = "branch-url", marker = "python_full_version >= '3.12'", url = "http://[LOCALHOST]/files/branch_url-2.0.0-py3-none-any.whl" },
    ]

    [[package]]
    name = "branch-url"
    version = "1.1.1"
    source = { registry = "http://[LOCALHOST]/simple/" }
    resolution-markers = [
        "python_full_version < '3.12'",
    ]
    sdist = { url = "http://[LOCALHOST]/files/branch_url-1.1.1.tar.gz", hash = "sha256:0beac3f2ad3ee94ec48aab67e9b43f48047961e1481c29940aaf6165c5b9f80b", upload-time = "2024-03-24T00:00:00Z" }
    wheels = [
        { url = "http://[LOCALHOST]/files/branch_url-1.1.1-py3-none-any.whl", hash = "sha256:859128ee0d6d0248a1170ee4030d342d23d20f5d8840b9568fb5802f9f0912dd", upload-time = "2024-03-24T00:00:00Z" },
    ]

    [[package]]
    name = "branch-url"
    version = "2.0.0"
    source = { url = "http://[LOCALHOST]/files/branch_url-2.0.0-py3-none-any.whl" }
    resolution-markers = [
        "python_full_version >= '3.12'",
    ]
    wheels = [
        { url = "http://[LOCALHOST]/files/branch_url-2.0.0-py3-none-any.whl", hash = "sha256:a6ec1b64c087a48cbd93ff6424a689588a48c35239c918ed85f67d299e7ea79d" },
    ]
    "#);

    Ok(())
}

/// The root package has two different direct URLs for disjoint forks, but they are from different sources.
///
/// ```toml
/// dependencies = [
///   "iniconfig @ https://files.pythonhosted.org/packages/9b/dd/b3c12c6d707058fa947864b67f0c4e0c39ef8610988d7baea9578f3c48f3/iniconfig-1.1.1-py2.py3-none-any.whl ; python_version < '3.12'",
///   "iniconfig @ git+https://github.com/pytest-dev/iniconfig@93f5930e668c0d1ddf4597e38dd0dea4e2665e7a ; python_version >= '3.12'",
/// ]
/// ```
#[test]
#[cfg(feature = "test-git")]
fn branching_urls_of_different_sources_disjoint() -> Result<()> {
    let server = uv_test::packse::PackseServer::new("packages/branching-urls.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&server.index_url());
    let git_url = git_package_url(&context)?;

    let deps = formatdoc! {r#"
        dependencies = [
            # Valid, disjoint split
            "git-branch @ {git_branch_url} ; python_version < '3.12'",
            "git-branch @ git+{git_url} ; python_version >= '3.12'",
        ]
    "#,
        git_branch_url = server.file_url("git_branch-1.1.1-py3-none-any.whl"),
    };
    make_project(context.temp_dir.path(), "a", &deps)?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    "
    );

    // We have source dist and wheel for the registry, but only the wheel for the direct URL.
    let mut filters = context.filters();
    filters.push((r"#[0-9a-f]{40}", "#[COMMIT]"));
    assert_snapshot!(apply_filters(context.read("uv.lock"), filters), @r#"
    version = 1
    revision = 3
    requires-python = ">=3.11, <3.13"
    resolution-markers = [
        "python_full_version >= '3.12'",
        "python_full_version < '3.12'",
    ]

    [options]
    exclude-newer = "2024-03-25T00:00:00Z"

    [[package]]
    name = "a"
    version = "0.1.0"
    source = { editable = "." }
    dependencies = [
        { name = "git-branch", version = "1.1.1", source = { url = "http://[LOCALHOST]/files/git_branch-1.1.1-py3-none-any.whl" }, marker = "python_full_version < '3.12'" },
        { name = "git-branch", version = "2.0.0", source = { git = "file://[TEMP_DIR]/git-branch/#[COMMIT]" }, marker = "python_full_version >= '3.12'" },
    ]

    [package.metadata]
    requires-dist = [
        { name = "git-branch", marker = "python_full_version < '3.12'", url = "http://[LOCALHOST]/files/git_branch-1.1.1-py3-none-any.whl" },
        { name = "git-branch", marker = "python_full_version >= '3.12'", git = "file://[TEMP_DIR]/git-branch/" },
    ]

    [[package]]
    name = "git-branch"
    version = "1.1.1"
    source = { url = "http://[LOCALHOST]/files/git_branch-1.1.1-py3-none-any.whl" }
    resolution-markers = [
        "python_full_version < '3.12'",
    ]
    wheels = [
        { url = "http://[LOCALHOST]/files/git_branch-1.1.1-py3-none-any.whl", hash = "sha256:41c1289c9f0e044c4bdc4868671dd08e79f4aa0ccff9941e42ab09c6fb6a3044" },
    ]

    [[package]]
    name = "git-branch"
    version = "2.0.0"
    source = { git = "file://[TEMP_DIR]/git-branch/#[COMMIT]" }
    resolution-markers = [
        "python_full_version >= '3.12'",
    ]
    "#);

    Ok(())
}

/// The root package has two different direct URLs from different sources, but they are not
/// disjoint.
///
/// ```toml
/// dependencies = [
///   "iniconfig @ https://files.pythonhosted.org/packages/9b/dd/b3c12c6d707058fa947864b67f0c4e0c39ef8610988d7baea9578f3c48f3/iniconfig-1.1.1-py2.py3-none-any.whl ; python_version < '3.12'",
///   "iniconfig @ git+https://github.com/pytest-dev/iniconfig@93f5930e668c0d1ddf4597e38dd0dea4e2665e7a ; python_version >= '3.12'",
/// ]
/// ```
#[test]
#[cfg(feature = "test-git")]
fn branching_urls_of_different_sources_conflict() -> Result<()> {
    let server = uv_test::packse::PackseServer::new("packages/branching-urls.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&server.index_url());
    let git_url = git_package_url(&context)?;

    let deps = formatdoc! {r#"
        dependencies = [
            # Conflicting split
            "git-branch @ {git_branch_url} ; python_version < '3.12'",
            "git-branch @ git+{git_url} ; python_version >= '3.11'",
        ]
    "#,
        git_branch_url = server.file_url("git_branch-1.1.1-py3-none-any.whl"),
    };
    make_project(context.temp_dir.path(), "a", &deps)?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to resolve dependencies for package `a==0.1.0`
      cause: Requirements contain conflicting URLs for package `git-branch` in split `python_full_version == '3.11.*'`:
             - git+file://[TEMP_DIR]/git-branch/
             - http://[LOCALHOST]/files/git_branch-1.1.1-py3-none-any.whl
    "
    );

    Ok(())
}

/// Ensure that we don't pre-visit package with URLs.
#[test]
fn dont_pre_visit_url_packages() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let deps = indoc! {r#"
        dependencies = [
            # This c is not a registry distribution, we must not pre-visit it as such.
            "c==0.1.0",
            "b",
        ]

        [tool.uv.sources]
        b = { path = "b" }
    "# };
    make_project(context.temp_dir.path(), "a", deps)?;

    let deps = indoc! {r#"
        dependencies = [
          "c",
        ]

        [tool.uv.sources]
        c = { path = "../c" }
    "# };
    make_project(&context.temp_dir.join("b"), "b", deps)?;
    let deps = indoc! {r"
        dependencies = []
    " };
    make_project(&context.temp_dir.join("c"), "c", deps)?;

    uv_snapshot!(context.filters(), context.lock().arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    "
    );

    assert_snapshot!(context.read("uv.lock"), @r#"
    version = 1
    revision = 3
    requires-python = ">=3.11, <3.13"

    [options]
    exclude-newer = "2024-03-25T00:00:00Z"

    [[package]]
    name = "a"
    version = "0.1.0"
    source = { editable = "." }
    dependencies = [
        { name = "b" },
        { name = "c" },
    ]

    [package.metadata]
    requires-dist = [
        { name = "b", directory = "b" },
        { name = "c", specifier = "==0.1.0" },
    ]

    [[package]]
    name = "b"
    version = "0.1.0"
    source = { directory = "b" }
    dependencies = [
        { name = "c" },
    ]

    [package.metadata]
    requires-dist = [{ name = "c", directory = "c" }]

    [[package]]
    name = "c"
    version = "0.1.0"
    source = { directory = "c" }
    "#);

    Ok(())
}
