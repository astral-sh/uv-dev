use std::process::Command;

use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::{FileWriteStr, PathChild};
use indoc::indoc;

use uv_static::EnvVars;
use uv_test::TestContext;

/// Run uv with a Linux personality confined to the child process.
fn uv_command(context: &TestContext, personality: Option<&str>) -> Command {
    let mut command = if let Some(personality) = personality {
        let mut command = context.external_command("setarch");
        command.arg(personality).arg(uv_test::get_bin!());
        command
    } else {
        context.external_command(uv_test::get_bin!())
    };
    context.add_shared_options(&mut command, false);
    command
}

#[test]
fn python_cache_respects_linux_personality() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let python = context.interpreter();

    // The same real interpreter must observe two different machine names. No alternate-arch
    // installation or global process-personality change is needed for this regression.
    context
        .python_command()
        .args(["-I", "-c", "import platform; print(platform.machine())"])
        .assert()
        .success()
        .stdout("x86_64\n");
    context
        .external_command("setarch")
        .arg("linux32")
        .arg(&python)
        .args(["-I", "-c", "import platform; print(platform.machine())"])
        .assert()
        .success()
        .stdout("i686\n");

    let requirements = context.temp_dir.child("requirements.in");
    requirements.write_str(indoc! {"
        ok==1.0.0; platform_machine == 'x86_64'
        ok==2.0.0; platform_machine != 'x86_64'
    "})?;

    let compile = |personality| {
        let mut command = uv_command(&context, personality);
        command
            .args([
                "pip",
                "compile",
                "--no-index",
                "--no-header",
                "--no-annotate",
            ])
            .arg("--find-links")
            .arg(context.workspace_root.join("test/links"))
            .arg("--python")
            .arg(&python)
            .arg(requirements.path())
            .env(EnvVars::RUST_LOG, "uv_python::interpreter=trace");
        command
    };

    compile(None).assert().success().stdout("ok==1.0.0\n");

    // Confirm that this fixture exercises the interpreter cache, not an uncacheable shim.
    let native = compile(None)
        .arg("-vv")
        .assert()
        .success()
        .stdout("ok==1.0.0\n");
    let cache_hit = format!(
        "skipping query of: {}",
        python.strip_prefix(context.temp_dir.path())?.display()
    );
    let stderr = String::from_utf8_lossy(&native.get_output().stderr);
    assert!(
        stderr.contains(&cache_hit),
        "missing native cache hit: {stderr}"
    );

    compile(Some("linux32"))
        .assert()
        .success()
        .stdout("ok==2.0.0\n");
    let alternate = compile(Some("linux32"))
        .arg("-vv")
        .assert()
        .success()
        .stdout("ok==2.0.0\n");
    let stderr = String::from_utf8_lossy(&alternate.get_output().stderr);
    assert!(
        stderr.contains(&cache_hit),
        "missing alternate cache hit: {stderr}"
    );

    compile(None).assert().success().stdout("ok==1.0.0\n");
    Ok(())
}
