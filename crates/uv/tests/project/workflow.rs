use anyhow::Result;
use assert_fs::fixture::{FileWriteStr, PathChild};
use insta::assert_snapshot;
use uv_test::packse::PackseServer;
use uv_test::{TestContext, diff_snapshot, uv_snapshot};

fn local_workflow_context() -> Result<(TestContext, PackseServer)> {
    let server = PackseServer::new("packages/workflow.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&server.index_url());
    context.temp_dir.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "workflow-project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["workflow-direct"]
        "#,
    )?;
    Ok((context, server))
}

#[test]
fn add_remove_one_package() -> Result<()> {
    let (context, _server) = local_workflow_context()?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(lock);
    });

    let diff = context.diff_lock(|context| {
        let mut add_cmd = context.add();
        add_cmd.arg("--no-sync").arg("workflow-added");
        add_cmd
    });
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(diff, @r#"
        --- old
        +++ new
        @@ -1,38 +1,51 @@
         version = 1
         revision = 3
         requires-python = ">=3.12"

         [options]
         exclude-newer = "2024-03-25T00:00:00Z"

         [[package]]
        +name = "workflow-added"
        +version = "3.0.0"
        +source = { registry = "http://[LOCALHOST]/simple/" }
        +sdist = { url = "http://[LOCALHOST]/files/workflow_added-3.0.0.tar.gz", hash = "sha256:5d32f9ef844c1a14f5c09b732f14e7290777e91c41848204653b90d2c0be4a45", upload-time = "2024-03-24T00:00:00Z" }
        +wheels = [
        +    { url = "http://[LOCALHOST]/files/workflow_added-3.0.0-py3-none-any.whl", hash = "sha256:7e68eee77089f1476bcf7e1abd325011040fbdfd84ce1a15f13daa39aa453f1f", upload-time = "2024-03-24T00:00:00Z" },
        +]
        +
        +[[package]]
         name = "workflow-direct"
         version = "1.0.0"
         source = { registry = "http://[LOCALHOST]/simple/" }
         dependencies = [
             { name = "workflow-transitive" },
         ]
         sdist = { url = "http://[LOCALHOST]/files/workflow_direct-1.0.0.tar.gz", hash = "sha256:fa0578253d11f61c77812a6750cd10af24b439bd5c5175d14ff4279f57e774bb", upload-time = "2024-03-24T00:00:00Z" }
         wheels = [
             { url = "http://[LOCALHOST]/files/workflow_direct-1.0.0-py3-none-any.whl", hash = "sha256:a1fe63a562d9624824ba4879835665e98e06d0998addb88da0cc483195ed3b5a", upload-time = "2024-03-24T00:00:00Z" },
         ]

         [[package]]
         name = "workflow-project"
         version = "0.1.0"
         source = { virtual = "." }
         dependencies = [
        +    { name = "workflow-added" },
             { name = "workflow-direct" },
         ]

         [package.metadata]
        -requires-dist = [{ name = "workflow-direct" }]
        +requires-dist = [
        +    { name = "workflow-added", specifier = ">=3.0.0" },
        +    { name = "workflow-direct" },
        +]

         [[package]]
         name = "workflow-transitive"
         version = "2.0.0"
         source = { registry = "http://[LOCALHOST]/simple/" }
         sdist = { url = "http://[LOCALHOST]/files/workflow_transitive-2.0.0.tar.gz", hash = "sha256:dfe258f151e3c2c8e7975dcd06a49c3e36fb839d1df7eb8cb64e0405ae31a2a5", upload-time = "2024-03-24T00:00:00Z" }
         wheels = [
             { url = "http://[LOCALHOST]/files/workflow_transitive-2.0.0-py3-none-any.whl", hash = "sha256:1be1acb7f1842ce7bad0b4d5b3a6eda3d2c0fa66fd089dc80dbce947d05b4483", upload-time = "2024-03-24T00:00:00Z" },
         ]
        "#);
    });

    let diff = context.diff_lock(|context| {
        let mut remove_cmd = context.remove();
        remove_cmd.arg("--no-sync").arg("workflow-added");
        remove_cmd
    });
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(diff, @r#"
        --- old
        +++ new
        @@ -1,51 +1,38 @@
         version = 1
         revision = 3
         requires-python = ">=3.12"

         [options]
         exclude-newer = "2024-03-25T00:00:00Z"

         [[package]]
        -name = "workflow-added"
        -version = "3.0.0"
        -source = { registry = "http://[LOCALHOST]/simple/" }
        -sdist = { url = "http://[LOCALHOST]/files/workflow_added-3.0.0.tar.gz", hash = "sha256:5d32f9ef844c1a14f5c09b732f14e7290777e91c41848204653b90d2c0be4a45", upload-time = "2024-03-24T00:00:00Z" }
        -wheels = [
        -    { url = "http://[LOCALHOST]/files/workflow_added-3.0.0-py3-none-any.whl", hash = "sha256:7e68eee77089f1476bcf7e1abd325011040fbdfd84ce1a15f13daa39aa453f1f", upload-time = "2024-03-24T00:00:00Z" },
        -]
        -
        -[[package]]
         name = "workflow-direct"
         version = "1.0.0"
         source = { registry = "http://[LOCALHOST]/simple/" }
         dependencies = [
             { name = "workflow-transitive" },
         ]
         sdist = { url = "http://[LOCALHOST]/files/workflow_direct-1.0.0.tar.gz", hash = "sha256:fa0578253d11f61c77812a6750cd10af24b439bd5c5175d14ff4279f57e774bb", upload-time = "2024-03-24T00:00:00Z" }
         wheels = [
             { url = "http://[LOCALHOST]/files/workflow_direct-1.0.0-py3-none-any.whl", hash = "sha256:a1fe63a562d9624824ba4879835665e98e06d0998addb88da0cc483195ed3b5a", upload-time = "2024-03-24T00:00:00Z" },
         ]

         [[package]]
         name = "workflow-project"
         version = "0.1.0"
         source = { virtual = "." }
         dependencies = [
        -    { name = "workflow-added" },
             { name = "workflow-direct" },
         ]

         [package.metadata]
        -requires-dist = [
        -    { name = "workflow-added", specifier = ">=3.0.0" },
        -    { name = "workflow-direct" },
        -]
        +requires-dist = [{ name = "workflow-direct" }]

         [[package]]
         name = "workflow-transitive"
         version = "2.0.0"
         source = { registry = "http://[LOCALHOST]/simple/" }
         sdist = { url = "http://[LOCALHOST]/files/workflow_transitive-2.0.0.tar.gz", hash = "sha256:dfe258f151e3c2c8e7975dcd06a49c3e36fb839d1df7eb8cb64e0405ae31a2a5", upload-time = "2024-03-24T00:00:00Z" }
         wheels = [
             { url = "http://[LOCALHOST]/files/workflow_transitive-2.0.0-py3-none-any.whl", hash = "sha256:1be1acb7f1842ce7bad0b4d5b3a6eda3d2c0fa66fd089dc80dbce947d05b4483", upload-time = "2024-03-24T00:00:00Z" },
         ]
        "#);
    });

    // Back to where we started.
    let new_lock = context.read("uv.lock");
    let diff = diff_snapshot(&lock, &new_lock, 10);
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(diff, @"");
    });

    Ok(())
}

#[test]
fn add_remove_existing_package_noop() -> Result<()> {
    let (context, _server) = local_workflow_context()?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(lock);
    });

    let diff = context.diff_lock(|context| {
        let mut add_cmd = context.add();
        add_cmd.arg("--no-sync").arg("workflow-direct");
        add_cmd
    });
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(diff, @"");
    });

    Ok(())
}

/// This test adds a new direct dependency that was already a
/// transitive dependency.
#[test]
fn promote_transitive_to_direct_then_remove() -> Result<()> {
    let (context, _server) = local_workflow_context()?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(lock);
    });

    let diff = context.diff_lock(|context| {
        let mut add_cmd = context.add();
        add_cmd.arg("--no-sync").arg("workflow-transitive");
        add_cmd
    });
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(diff, @r#"
        --- old
        +++ new
        @@ -16,23 +16,27 @@
         wheels = [
             { url = "http://[LOCALHOST]/files/workflow_direct-1.0.0-py3-none-any.whl", hash = "sha256:a1fe63a562d9624824ba4879835665e98e06d0998addb88da0cc483195ed3b5a", upload-time = "2024-03-24T00:00:00Z" },
         ]

         [[package]]
         name = "workflow-project"
         version = "0.1.0"
         source = { virtual = "." }
         dependencies = [
             { name = "workflow-direct" },
        +    { name = "workflow-transitive" },
         ]

         [package.metadata]
        -requires-dist = [{ name = "workflow-direct" }]
        +requires-dist = [
        +    { name = "workflow-direct" },
        +    { name = "workflow-transitive", specifier = ">=2.0.0" },
        +]

         [[package]]
         name = "workflow-transitive"
         version = "2.0.0"
         source = { registry = "http://[LOCALHOST]/simple/" }
         sdist = { url = "http://[LOCALHOST]/files/workflow_transitive-2.0.0.tar.gz", hash = "sha256:dfe258f151e3c2c8e7975dcd06a49c3e36fb839d1df7eb8cb64e0405ae31a2a5", upload-time = "2024-03-24T00:00:00Z" }
         wheels = [
             { url = "http://[LOCALHOST]/files/workflow_transitive-2.0.0-py3-none-any.whl", hash = "sha256:1be1acb7f1842ce7bad0b4d5b3a6eda3d2c0fa66fd089dc80dbce947d05b4483", upload-time = "2024-03-24T00:00:00Z" },
         ]
        "#);
    });

    let diff = context.diff_lock(|context| {
        let mut remove_cmd = context.remove();
        remove_cmd.arg("--no-sync").arg("workflow-transitive");
        remove_cmd
    });
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(diff, @r#"
        --- old
        +++ new
        @@ -16,27 +16,23 @@
         wheels = [
             { url = "http://[LOCALHOST]/files/workflow_direct-1.0.0-py3-none-any.whl", hash = "sha256:a1fe63a562d9624824ba4879835665e98e06d0998addb88da0cc483195ed3b5a", upload-time = "2024-03-24T00:00:00Z" },
         ]

         [[package]]
         name = "workflow-project"
         version = "0.1.0"
         source = { virtual = "." }
         dependencies = [
             { name = "workflow-direct" },
        -    { name = "workflow-transitive" },
         ]

         [package.metadata]
        -requires-dist = [
        -    { name = "workflow-direct" },
        -    { name = "workflow-transitive", specifier = ">=2.0.0" },
        -]
        +requires-dist = [{ name = "workflow-direct" }]

         [[package]]
         name = "workflow-transitive"
         version = "2.0.0"
         source = { registry = "http://[LOCALHOST]/simple/" }
         sdist = { url = "http://[LOCALHOST]/files/workflow_transitive-2.0.0.tar.gz", hash = "sha256:dfe258f151e3c2c8e7975dcd06a49c3e36fb839d1df7eb8cb64e0405ae31a2a5", upload-time = "2024-03-24T00:00:00Z" }
         wheels = [
             { url = "http://[LOCALHOST]/files/workflow_transitive-2.0.0-py3-none-any.whl", hash = "sha256:1be1acb7f1842ce7bad0b4d5b3a6eda3d2c0fa66fd089dc80dbce947d05b4483", upload-time = "2024-03-24T00:00:00Z" },
         ]
        "#);
    });

    // Back to where we started.
    let new_lock = context.read("uv.lock");
    let diff = diff_snapshot(&lock, &new_lock, 10);
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(diff, @"");
    });

    Ok(())
}
