#![cfg(feature = "test-universal")]

use anyhow::Result;
use assert_fs::prelude::*;
use indoc::{formatdoc, indoc};
use insta::assert_snapshot;

use uv_test::packse::PackseServer;
use uv_test::uv_snapshot;

/// An environment-covering explicit index is immutable source policy, so it does not prevent
/// coordinated backtracking of the package's parent.
#[test]
fn coordinated_scope_unconditional_explicit_index() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let primary = PackseServer::new("fork/coordinated-backtracking.toml");
    let shared = PackseServer::new("fork/coordinated-backtracking.toml");
    let shared_index = shared.index_url();
    let primary_pattern = regex::escape(&primary.index_url());
    let shared_pattern = regex::escape(&shared_index);
    let mut filters = vec![
        (primary_pattern.as_str(), "http://[PRIMARY]/simple"),
        (shared_pattern.as_str(), "http://[SHARED]/simple"),
    ];
    filters.extend(context.filters());
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
    insta::with_settings!({ filters => filters }, {
        assert_snapshot!(context.read("uv.lock"), @r#"
        version = 1
        revision = 5
        requires-python = ">=3.12, <3.14"
        resolution-markers = [
            "python_full_version < '3.13'",
            "python_full_version >= '3.13'",
        ]
        supported-markers = [
            "python_full_version < '3.13'",
            "python_full_version >= '3.13'",
        ]

        [options]
        fork-strategy = "fewest"
        exclude-newer = "2024-03-25T00:00:00Z"

        [manifest]
        constraints = [{ name = "shared", specifier = ">=1.0.0", index = "http://[SHARED]/simple" }]

        [[package]]
        name = "constrained"
        version = "1.0.0"
        source = { registry = "http://[PRIMARY]/simple" }
        dependencies = [
            { name = "shared" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/constrained-1.0.0.tar.gz", hash = "sha256:9b4062cacb890b169aab96e0228fb382b36dff75eac0b1f955207550931f7a0f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/constrained-1.0.0-py3-none-any.whl", hash = "sha256:2f66e11b867b8015a9e0ae19c76ea4aee4b8bb75b007de24c5f9f917e8a5cb7f", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-one"
        version = "1.0.0"
        source = { registry = "http://[PRIMARY]/simple" }
        dependencies = [
            { name = "delay-two" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/delay_one-1.0.0.tar.gz", hash = "sha256:5f12e248eff8774104018a912e6ba62ee73a05b1642f85a29c0f069e91eab9cb", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_one-1.0.0-py3-none-any.whl", hash = "sha256:9fb6fd4d056dec49f9579a7872ca9e2112e8464ab0cada656f151bf2a917ccfe", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-two"
        version = "1.0.0"
        source = { registry = "http://[PRIMARY]/simple" }
        dependencies = [
            { name = "constrained" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/delay_two-1.0.0.tar.gz", hash = "sha256:f8eeeb01fb981746b4a44422ab847ce7a0972c4fadd7c4aafa15b1d282d69fd4", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_two-1.0.0-py3-none-any.whl", hash = "sha256:f6f61fb03ff3ebdf7053cef619af7f667f2fa26dc7d19c83a22b42ea525323da", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delayed"
        version = "1.0.0"
        source = { registry = "http://[PRIMARY]/simple" }
        dependencies = [
            { name = "delay-one" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/delayed-1.0.0.tar.gz", hash = "sha256:a63fa5698e2a8ac4cc7c2b098ade0615fe786921b5276644b17e206177ed69a8", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/delayed-1.0.0-py3-none-any.whl", hash = "sha256:ebfe0ac448d02609803a2fc9f6d621aba9d4f307741de9588c02614fb25e65dd", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "delayed", marker = "python_full_version >= '3.13'" },
            { name = "switchable", marker = "python_full_version < '3.13'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "delayed", marker = "python_full_version >= '3.13'" },
            { name = "switchable", marker = "python_full_version < '3.13'" },
        ]

        [[package]]
        name = "shared"
        version = "1.0.0"
        source = { registry = "http://[SHARED]/simple" }
        sdist = { url = "http://[LOCALHOST]/files/shared-1.0.0.tar.gz", hash = "sha256:9a61c5124c678bf52b6fd5d37466b12ec9a023179745a86b0c2dedc58ff92ff5", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/shared-1.0.0-py3-none-any.whl", hash = "sha256:7d13d64b0b798eeeb66fc56711fab6238aa7a99e29011426f901e56ab1bb0ef2", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "switchable"
        version = "1.0.0"
        source = { registry = "http://[PRIMARY]/simple" }
        dependencies = [
            { name = "shared" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/switchable-1.0.0.tar.gz", hash = "sha256:044ec9f373c978f01045aa3fde7af5d2ef47ecaef4b8f74ed5c55fcc68271b37", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/switchable-1.0.0-py3-none-any.whl", hash = "sha256:7384d7fc95d1030432cc12fd19a1ff4701047490ea9c525664d79b9a9432951d", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#);
    });
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
    let primary_pattern = regex::escape(&primary.index_url());
    let left_pattern = regex::escape(&left_index);
    let right_pattern = regex::escape(&right_index);
    let mut filters = vec![
        (primary_pattern.as_str(), "http://[PRIMARY]/simple"),
        (left_pattern.as_str(), "http://[LEFT]/simple"),
        (right_pattern.as_str(), "http://[RIGHT]/simple"),
    ];
    filters.extend(context.filters());
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
    insta::with_settings!({ filters => filters }, {
        assert_snapshot!(context.read("uv.lock"), @r#"
        version = 1
        revision = 5
        requires-python = ">=3.12, <3.14"
        resolution-markers = [
            "python_full_version < '3.13'",
            "python_full_version >= '3.13'",
        ]
        supported-markers = [
            "python_full_version < '3.13'",
            "python_full_version >= '3.13'",
        ]

        [options]
        fork-strategy = "fewest"
        exclude-newer = "2024-03-25T00:00:00Z"

        [manifest]
        constraints = [
            { name = "shared", marker = "python_full_version < '3.13'", specifier = ">=1.0.0,<=2.0.0", index = "http://[LEFT]/simple" },
            { name = "shared", marker = "python_full_version >= '3.13'", specifier = ">=1.0.0,<=2.0.0", index = "http://[RIGHT]/simple" },
        ]

        [[package]]
        name = "constrained"
        version = "1.0.0"
        source = { registry = "http://[PRIMARY]/simple" }
        dependencies = [
            { name = "shared", version = "1.0.0", source = { registry = "http://[RIGHT]/simple" } },
        ]
        sdist = { url = "http://[LOCALHOST]/files/constrained-1.0.0.tar.gz", hash = "sha256:eb04a033793f2fe719150c5e0603311b5d8e59e8077b52ddd85eb3b385e70897", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/constrained-1.0.0-py3-none-any.whl", hash = "sha256:360f2e54fa8c7bf57ec2b1bad4616ad2765505815a69ab3e63aeffb2c5567c3f", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-one"
        version = "1.0.0"
        source = { registry = "http://[PRIMARY]/simple" }
        dependencies = [
            { name = "delay-two" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/delay_one-1.0.0.tar.gz", hash = "sha256:24641f1b68d386ea2a2ef23ebc54305cb347d9a2745618057697c3ff4dbc5648", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_one-1.0.0-py3-none-any.whl", hash = "sha256:c1e8aca4b1866ffedbf054bd89c0d7d4fbc8647de899909cff7dadf9ff9d59ef", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-two"
        version = "1.0.0"
        source = { registry = "http://[PRIMARY]/simple" }
        dependencies = [
            { name = "constrained" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/delay_two-1.0.0.tar.gz", hash = "sha256:a351f0383eb434fdc7baf52c081842695d685aeddac852a4394171b9079b86f9", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_two-1.0.0-py3-none-any.whl", hash = "sha256:1de8fc30a0e8e2c578b36cb66690f5c86aa5f86f5bae834b6c47c6089bce8c3f", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delayed"
        version = "1.0.0"
        source = { registry = "http://[PRIMARY]/simple" }
        dependencies = [
            { name = "delay-one" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/delayed-1.0.0.tar.gz", hash = "sha256:0211a4571e3bded055e0dcac1080c40f20455fcf07dcb0a4e2d1a112f200c205", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/delayed-1.0.0-py3-none-any.whl", hash = "sha256:cc3bd7eea5a6d2263f0ee72930379019167956dfc6b5941b33f133964aeb696c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "flexible"
        version = "1.0.0"
        source = { registry = "http://[PRIMARY]/simple" }
        dependencies = [
            { name = "shared", version = "2.0.0", source = { registry = "http://[LEFT]/simple" } },
        ]
        sdist = { url = "http://[LOCALHOST]/files/flexible-1.0.0.tar.gz", hash = "sha256:731d1890e528b7a605e056ad91267e8350a9fd21adc50d09982ac2bf10f37199", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/flexible-1.0.0-py3-none-any.whl", hash = "sha256:b4f5b803a4db6aed15a5a4da91864390188eb35a13c2f7a147150ead8e54d9b1", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "left-only"
        version = "1.0.0"
        source = { registry = "http://[PRIMARY]/simple" }
        sdist = { url = "http://[LOCALHOST]/files/left_only-1.0.0.tar.gz", hash = "sha256:2b2f40d062edc772bc0bf356d50ff00c760b527b81a4a13124e8a38509bd3d88", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/left_only-1.0.0-py3-none-any.whl", hash = "sha256:ca9043770b471570049ebb77e44b17443c4788f95d1a0df37077fa91aa0928d4", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "delayed", marker = "python_full_version >= '3.13'" },
            { name = "flexible", marker = "python_full_version < '3.13'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "delayed", marker = "python_full_version >= '3.13'" },
            { name = "flexible", marker = "python_full_version < '3.13'" },
        ]

        [[package]]
        name = "right-only"
        version = "1.0.0"
        source = { registry = "http://[PRIMARY]/simple" }
        sdist = { url = "http://[LOCALHOST]/files/right_only-1.0.0.tar.gz", hash = "sha256:ceaad53a9b00373a0c4e28923ce3cdea5457c32167f9897ffaf2937da9b99a3f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/right_only-1.0.0-py3-none-any.whl", hash = "sha256:9559b074cd64291c021817582017dfa3c2f2894bfa1cab892dd8dc1679867e27", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "shared"
        version = "1.0.0"
        source = { registry = "http://[RIGHT]/simple" }
        resolution-markers = [
            "python_full_version >= '3.13'",
        ]
        dependencies = [
            { name = "right-only" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/shared-1.0.0.tar.gz", hash = "sha256:81aef92abc25becc47531080abac4718eb78fd59c8253706e5412152f7f9bcfd", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/shared-1.0.0-py3-none-any.whl", hash = "sha256:56a1626f3df0a2654e99e8d81f6dfeb8cf767bdf07dc8710407ee966c1e6f22d", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "shared"
        version = "2.0.0"
        source = { registry = "http://[LEFT]/simple" }
        resolution-markers = [
            "python_full_version < '3.13'",
        ]
        dependencies = [
            { name = "left-only" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/shared-2.0.0.tar.gz", hash = "sha256:7c08a8d9fea593c870c0f988a93238289e2476cd490fdb29a0d1bf577934f810", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/shared-2.0.0-py3-none-any.whl", hash = "sha256:3d492212da79e347adcc1119ebcacbf2d1bedd60054f1404882884928d95f8b3", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#);
    });
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
