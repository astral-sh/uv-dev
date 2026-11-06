use anyhow::Result;
use assert_fs::prelude::*;
use indoc::formatdoc;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

use uv_static::EnvVars;
use uv_test::package_server::PackageServer;
use uv_test::packse::WheelBuilder;
use uv_test::{TestContext, uv_snapshot};

fn tool_wheel(context: &TestContext, version: &str, lock: Option<&str>) -> Result<()> {
    let dist_info = format!("locked_tool-{version}.dist-info");
    let entry_points = format!("{dist_info}/entry_points.txt");
    let lock_path = format!("{dist_info}/pylock.toml");
    let mut files = vec![
        (
            entry_points.as_str(),
            "[console_scripts]\nlocked-tool = locked_tool.cli:main\n",
        ),
        (
            "locked_tool/cli.py",
            "from importlib.metadata import version\ndef main(): print(version('idna'))\n",
        ),
    ];
    if let Some(lock) = lock {
        files.push((&lock_path, lock));
    }
    let (filename, bytes) = WheelBuilder::new(&"locked-tool".parse()?, &version.parse()?)
        .requires(&["idna>=3.3,<3.5".parse()?])
        .files(&files)
        .build();
    context
        .temp_dir
        .child("wheels")
        .child(filename)
        .write_binary(&bytes)?;
    Ok(())
}

fn lock_entry(
    name: &str,
    version: &str,
    index: &str,
    filename: &str,
    url: &str,
    hash: &str,
) -> String {
    formatdoc! {r#"
        [[packages]]
        name = "{name}"
        version = "{version}"
        index = "{index}"
        wheels = [
            {{ name = "{filename}", url = "{url}", hashes = {{ sha256 = "{hash}" }} }},
        ]
    "#}
}

const LOCK_HEADER: &str = "lock-version = \"1.0\"\ncreated-by = \"test\"\n";

async fn serve_wheel(
    server: &MockServer,
    filename: &str,
    bytes: Vec<u8>,
) -> (String, String, Value) {
    let url = format!("{}/files/{filename}", server.uri());
    let hash = hex::encode(Sha256::digest(&bytes));
    Mock::given(method("GET"))
        .and(path(format!("/files/{filename}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes))
        .mount(server)
        .await;
    let file = json!({
        "filename": filename,
        "url": url,
        "hashes": { "sha256": hash },
        "upload-time": "2024-01-01T00:00:00Z",
    });
    (url, hash, file)
}

async fn serve_index(server: &MockServer, package: &str, files: Vec<Value>) {
    Mock::given(method("GET"))
        .and(path(format!("/simple/{package}/")))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(
                json!({ "meta": { "api-version": "1.0" }, "name": package, "files": files })
                    .to_string(),
                "application/vnd.pypi.simple.v1+json",
            ),
        )
        .mount(server)
        .await;
}

#[test]
fn packaged_lock_required() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    tool_wheel(
        &context,
        "1.0.0",
        Some(&format!("{LOCK_HEADER}packages = []\n")),
    )?;
    tool_wheel(&context, "2.0.0", None)?;

    uv_snapshot!(context.filters(), context.tool_install()
        .args(["--locked", "--preview-features", "locked-tools", "--no-index",
               "--find-links", "wheels", "locked-tool"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `locked-tool==2.0.0` does not contain `pylock.toml` in its `.dist-info` directory; `--locked` requires a packaged lock
    ");

    Ok(())
}

/// A URL with the right name and digest still requires authorization from the configured index.
#[tokio::test]
async fn packaged_lock_rejects_unlisted_artifact() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    let server = PackageServer::new(&"idna".parse()?).await;
    let (filename, wheel) = WheelBuilder::new(&"idna".parse()?, &"3.3".parse()?).build();
    let hash = hex::encode(Sha256::digest(&wheel));
    server.serve(&filename, &wheel, Some(&hash)).await;

    let other_server = MockServer::start().await;
    let path_to_wheel = format!("/{filename}");
    let forged_url = format!("{}{path_to_wheel}", other_server.uri());
    Mock::given(path(path_to_wheel))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(wheel))
        .expect(0)
        .mount(&other_server)
        .await;

    let index = server.index_url();
    let entry = lock_entry("idna", "3.3", &index, &filename, &forged_url, &hash);
    tool_wheel(&context, "1.0.0", Some(&format!("{LOCK_HEADER}{entry}")))?;

    uv_snapshot!(context.filters(), context.tool_install()
        .args(["--locked", "--preview-features", "locked-tools", "--default-index", &index,
               "--find-links", "wheels", "locked-tool"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The packaged lock for `locked-tool==1.0.0` contains unverified artifacts
      cause: URL for `idna==3.3` is not listed by http://[LOCALHOST]/simple: http://[LOCALHOST]/idna-3.3-py3-none-any.whl
    ");

    Ok(())
}

#[tokio::test]
async fn packaged_lock_checks_index_hash() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    let server = PackageServer::new(&"idna".parse()?).await;
    let (filename, wheel) = WheelBuilder::new(&"idna".parse()?, &"3.3".parse()?).build();
    let hash = hex::encode(Sha256::digest(&wheel));
    server.serve(&filename, &wheel, Some(&"0".repeat(64))).await;
    let index = server.index_url();
    let entry = lock_entry(
        "idna",
        "3.3",
        &index,
        &filename,
        &server.file_url(&filename),
        &hash,
    );
    tool_wheel(&context, "1.0.0", Some(&format!("{LOCK_HEADER}{entry}")))?;
    let context = context.with_filter((hash, "[LOCK_HASH]"));

    uv_snapshot!(context.filters(), context.tool_install()
        .args(["--locked", "--preview-features", "locked-tools", "--default-index", &index,
               "--find-links", "wheels", "locked-tool"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Hash mismatch for `idna==3.3`

    Expected:
      sha256:0000000000000000000000000000000000000000000000000000000000000000
      sha256:[LOCK_HASH]

    Computed:
      sha256:[LOCK_HASH]
    ");

    Ok(())
}

#[tokio::test]
async fn packaged_lock_rejects_incompatible_constraint() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    let server = PackageServer::new(&"idna".parse()?).await;
    let (filename, wheel) = WheelBuilder::new(&"idna".parse()?, &"3.3".parse()?).build();
    let hash = hex::encode(Sha256::digest(&wheel));
    server.serve(&filename, &wheel, Some(&hash)).await;
    let index = server.index_url();
    let entry = lock_entry(
        "idna",
        "3.3",
        &index,
        &filename,
        &server.file_url(&filename),
        &hash,
    );
    tool_wheel(&context, "1.0.0", Some(&format!("{LOCK_HEADER}{entry}")))?;
    context
        .temp_dir
        .child("constraints.txt")
        .write_str("idna>=3.4\n")?;

    uv_snapshot!(context.filters(), context.tool_install()
        .args(["--locked", "--preview-features", "locked-tools", "--default-index", &index,
               "--find-links", "wheels", "--constraint", "constraints.txt", "locked-tool"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The packaged lock selects `idna==3.3`, which is incompatible with constraint `idna>=3.4`
    ");

    Ok(())
}

#[tokio::test]
async fn packaged_lock_overrides_and_excludes() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");
    let server = MockServer::start().await;
    let index = format!("{}/simple", server.uri());
    let (override_name, override_wheel) = WheelBuilder::new(&"idna".parse()?, &"3.5".parse()?)
        .requires(&["beta>=1".parse()?, "gamma>=1".parse()?])
        .build();
    let (_, override_hash, override_file) =
        serve_wheel(&server, &override_name, override_wheel).await;
    serve_index(&server, "idna", vec![override_file]).await;
    let (beta_name, beta_wheel) = WheelBuilder::new(&"beta".parse()?, &"1.0".parse()?).build();
    let (beta_url, beta_hash, beta_file) = serve_wheel(&server, &beta_name, beta_wheel).await;
    serve_index(&server, "beta", vec![beta_file]).await;

    let idna_entry = lock_entry(
        "idna",
        "3.3",
        &index,
        "idna-3.3-py3-none-any.whl",
        &format!("{}/unavailable/idna-3.3-py3-none-any.whl", server.uri()),
        &"0".repeat(64),
    );
    let beta_entry = lock_entry("beta", "1.0", &index, &beta_name, &beta_url, &beta_hash);
    tool_wheel(
        &context,
        "1.0.0",
        Some(&format!("{LOCK_HEADER}{idna_entry}{beta_entry}")),
    )?;
    context
        .temp_dir
        .child("overrides.txt")
        .write_str(&format!("idna==3.5 --hash=sha256:{override_hash}\n"))?;
    context
        .temp_dir
        .child("constraints.txt")
        .write_str(&format!("beta @ {beta_url}#sha256={beta_hash}\n"))?;
    context
        .temp_dir
        .child("excludes.txt")
        .write_str("gamma\n")?;

    uv_snapshot!(context.filters(), context.tool_install()
        .args(["--locked", "--preview-features", "locked-tools", "--default-index", &index,
               "--find-links", "wheels", "--override", "overrides.txt",
               "--constraints", "constraints.txt", "--exclude", "excludes.txt", "locked-tool"])
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed [N] packages in [TIME]
     + beta==1.0
     + idna==3.5
     + locked-tool==1.0.0
    Installed 1 executable: locked-tool
    ");

    Ok(())
}
