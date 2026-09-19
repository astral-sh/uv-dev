use std::path::PathBuf;

use anyhow::Result;
use assert_fs::prelude::*;
use indoc::indoc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use uv_fs::Simplified;
use uv_static::EnvVars;
use uv_test::uv_snapshot;

/// Remote scripts must remain available until Python exits, including unsuccessful exits.
#[tokio::test]
async fn remote_script_temporary_file_lifetime() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    let (_, interpreter) = context
        .python_versions
        .first()
        .ok_or_else(|| anyhow::anyhow!("the test requires Python 3.12"))?;
    let downloads = context.temp_dir.child("downloads");
    downloads.create_dir_all()?;
    let downloads_path = downloads.simple_canonicalize()?;
    let recorded_path = context.temp_dir.child("script-path.txt");

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/lifetime.py"))
        .respond_with(ResponseTemplate::new(200).set_body_string(indoc! {r#"
            # /// script
            # requires-python = ">=3.12"
            # dependencies = []
            # ///

            from pathlib import Path
            import sys

            script = Path(__file__).resolve()
            print(f"script exists: {script.is_file()}")
            Path(sys.argv[1]).write_text(str(script), encoding="utf-8")
            raise SystemExit(int(sys.argv[2]))
        "#}))
        .expect(2)
        .mount(&server)
        .await;

    let command = |exit_code: u8| {
        let mut command = context.run();
        command.env_clear();
        context.add_shared_env(&mut command, false);
        // Winsock expands `%SystemRoot%` when loading provider DLLs.
        #[cfg(windows)]
        if let Some(system_root) = std::env::var_os("SYSTEMROOT") {
            command.env("SYSTEMROOT", system_root);
        }
        command
            .args(["--no-config", "--no-project", "--no-index", "--no-build"])
            .arg("--python")
            .arg(interpreter)
            .arg(format!("{}/lifetime.py", server.uri()))
            .arg(recorded_path.path())
            .arg(exit_code.to_string())
            .env(EnvVars::UV_PYTHON_DOWNLOADS, "never")
            .env(EnvVars::UV_HTTP_RETRIES, "0")
            .env(EnvVars::NO_PROXY, "*")
            .env("TMPDIR", &downloads_path)
            .env("TEMP", &downloads_path)
            .env("TMP", &downloads_path);
        command
    };

    let assert_cleanup = || -> Result<()> {
        let script = PathBuf::from(fs_err::read_to_string(recorded_path.path())?);
        assert_eq!(script.simplified().parent(), Some(downloads_path.as_path()));
        assert_eq!(script.extension().and_then(|ext| ext.to_str()), Some("py"));
        assert_eq!(
            fs_err::symlink_metadata(&script)
                .expect_err("the downloaded script must be removed after Python exits")
                .kind(),
            std::io::ErrorKind::NotFound,
        );
        Ok(())
    };

    uv_snapshot!(context.filters(), command(0), @"
    exit_code: 0 (success)
    ----- stdout -----
    script exists: True
    ");
    assert_cleanup()?;
    assert_eq!(
        server
            .received_requests()
            .await
            .map(|requests| requests.len()),
        Some(1)
    );

    uv_snapshot!(context.filters(), command(7), @"
    exit_code: 7 (failure)
    ----- stdout -----
    script exists: True
    ");
    assert_cleanup()?;

    assert_eq!(
        server
            .received_requests()
            .await
            .map(|requests| requests.len()),
        Some(2)
    );
    server.verify().await;
    Ok(())
}
