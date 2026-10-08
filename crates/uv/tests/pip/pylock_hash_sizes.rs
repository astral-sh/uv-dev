use std::collections::BTreeMap;

use anyhow::Result;
use assert_cmd::prelude::*;
use assert_fs::prelude::*;
use indoc::indoc;
use serde_json::json;
use sha2::{Digest, Sha256};
use toml_edit::{DocumentMut, Item, Value};
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

use uv_test::package_server::PackageServer;
use uv_test::packse::generate_wheel;
use uv_test::uv_snapshot;

async fn serve_hashless_wheel(
    server: &PackageServer,
    filename: &str,
    bytes: &[u8],
    size: Option<u64>,
) {
    let mut metadata = json!({ "core-metadata": true });
    if let Some(size) = size {
        metadata["size"] = json!(size);
    }
    server.serve_with(filename, bytes, None, metadata).await;
    Mock::given(method("GET"))
        .and(path(format!("/{filename}.metadata")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("Metadata-Version: 2.3\nName: demo\nVersion: 1.0\n"),
        )
        .mount(server.mock_server())
        .await;
}

fn set_recorded_wheel(lock: &mut DocumentMut, url: &str, size: u64) -> Result<()> {
    let package = lock["package"]
        .as_array_of_tables_mut()
        .expect("lock contains packages")
        .iter_mut()
        .find(|package| package.get("name").and_then(Item::as_str) == Some("demo"))
        .expect("demo is locked");
    let wheel = package["wheels"]
        .as_array_mut()
        .expect("demo has wheels")
        .get_mut(0)
        .and_then(Value::as_inline_table_mut)
        .expect("wheel is an inline table");
    wheel.insert("url", Value::from(url));
    wheel.insert("size", Value::from(i64::try_from(size)?));
    wheel.remove("hash");
    Ok(())
}

#[tokio::test]
async fn compile_hash_completion_without_advisory_size() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let name = "demo".parse()?;
    let server = PackageServer::new(&name).await;
    let (filename, bytes) = generate_wheel(
        &name,
        &"1.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    serve_hashless_wheel(&server, &filename, &bytes, None).await;
    context
        .temp_dir
        .child("requirements.in")
        .write_str("demo==1.0")?;
    context
        .pip_compile()
        .arg("requirements.in")
        .arg("--index-url")
        .arg(server.index_url())
        .arg("-o")
        .arg("pylock.toml")
        .assert()
        .success();

    let export: toml::Value = toml::from_str(&context.read("pylock.toml"))?;
    let wheel = &export["packages"][0]["wheels"][0];
    let digest = hex::encode(Sha256::digest(&bytes));
    assert_eq!(wheel["hashes"]["sha256"].as_str(), Some(digest.as_str()));
    assert!(wheel.get("size").is_none());
    context
        .pip_sync()
        .arg("pylock.toml")
        .arg("--preview-features")
        .arg("pylock")
        .arg("--no-index")
        .assert()
        .success();
    context.assert_installed("demo", "1.0");
    Ok(())
}

#[tokio::test]
async fn compile_hash_completion_refreshes_advisory_size() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let name = "demo".parse()?;
    let server = PackageServer::new(&name).await;
    let (filename, bytes) = generate_wheel(
        &name,
        &"1.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    serve_hashless_wheel(&server, &filename, &bytes, Some(1)).await;
    context
        .temp_dir
        .child("requirements.in")
        .write_str("demo==1.0")?;
    context
        .pip_compile()
        .arg("requirements.in")
        .arg("--index-url")
        .arg(server.index_url())
        .arg("-o")
        .arg("pylock.toml")
        .assert()
        .success();

    let export: toml::Value = toml::from_str(&context.read("pylock.toml"))?;
    let wheel = &export["packages"][0]["wheels"][0];
    let digest = hex::encode(Sha256::digest(&bytes));
    assert_eq!(wheel["hashes"]["sha256"].as_str(), Some(digest.as_str()));
    assert_eq!(
        wheel.get("size").and_then(toml::Value::as_integer),
        Some(i64::try_from(bytes.len())?),
    );
    context
        .pip_sync()
        .arg("pylock.toml")
        .arg("--preview-features")
        .arg("pylock")
        .arg("--no-index")
        .assert()
        .success();
    context.assert_installed("demo", "1.0");
    Ok(())
}

#[tokio::test]
async fn export_hash_completion_checks_recorded_size() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let name = "demo".parse()?;
    let server = PackageServer::new(&name).await;
    let (filename, bytes) = generate_wheel(
        &name,
        &"1.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    let actual_size = u64::try_from(bytes.len())?;
    serve_hashless_wheel(&server, &filename, &bytes, Some(actual_size)).await;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["demo"]
    "#})?;
    context
        .lock()
        .arg("--default-index")
        .arg(server.index_url())
        .assert()
        .success();
    let lock_path = context.temp_dir.child("uv.lock");
    let mut lock = context.read("uv.lock").parse::<DocumentMut>()?;
    let output_path = context.temp_dir.child("pylock.toml");

    set_recorded_wheel(&mut lock, &server.file_url(&filename), actual_size)?;
    lock_path.write_str(&lock.to_string())?;
    context
        .export()
        .arg("--frozen")
        .arg("--no-emit-project")
        .arg("--format")
        .arg("pylock.toml")
        .arg("-o")
        .arg(output_path.path())
        .assert()
        .success();
    let export: toml::Value = toml::from_str(&context.read("pylock.toml"))?;
    let wheel = &export["packages"][0]["wheels"][0];
    assert_eq!(
        wheel["size"].as_integer(),
        Some(i64::try_from(actual_size)?)
    );
    let digest = hex::encode(Sha256::digest(&bytes));
    assert_eq!(wheel["hashes"]["sha256"].as_str(), Some(digest.as_str()));

    set_recorded_wheel(&mut lock, &server.file_url(&filename), actual_size + 1)?;
    lock_path.write_str(&lock.to_string())?;
    output_path.write_str("previous export\n")?;
    uv_snapshot!(context.filters(), context.export()
        .arg("--frozen")
        .arg("--no-emit-project")
        .arg("--format")
        .arg("pylock.toml")
        .arg("-o")
        .arg(output_path.path()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Artifact `http://[LOCALHOST]/demo-1.0-py3-none-any.whl` has size 914, but the lockfile records 915
    ");
    assert_eq!(context.read("pylock.toml"), "previous export\n");
    Ok(())
}

#[tokio::test]
async fn export_local_hash_completion_checks_recorded_size() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let name = "demo".parse()?;
    let server = PackageServer::new(&name).await;
    let (filename, bytes) = generate_wheel(
        &name,
        &"1.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    let actual_size = u64::try_from(bytes.len())?;
    serve_hashless_wheel(&server, &filename, &bytes, Some(actual_size)).await;
    let local_wheel = context.temp_dir.child(&filename);
    local_wheel.write_binary(&bytes)?;
    let file_url = Url::from_file_path(local_wheel.path()).expect("absolute fixture path");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["demo"]
    "#})?;
    context
        .lock()
        .arg("--default-index")
        .arg(server.index_url())
        .assert()
        .success();
    let lock_path = context.temp_dir.child("uv.lock");
    let mut lock = context.read("uv.lock").parse::<DocumentMut>()?;
    let output_path = context.temp_dir.child("pylock.toml");

    set_recorded_wheel(&mut lock, file_url.as_str(), actual_size)?;
    lock_path.write_str(&lock.to_string())?;
    context
        .export()
        .arg("--frozen")
        .arg("--no-emit-project")
        .arg("--format")
        .arg("pylock.toml")
        .arg("-o")
        .arg(output_path.path())
        .assert()
        .success();
    let export: toml::Value = toml::from_str(&context.read("pylock.toml"))?;
    let wheel = &export["packages"][0]["wheels"][0];
    assert_eq!(
        wheel["size"].as_integer(),
        Some(i64::try_from(actual_size)?)
    );
    let digest = hex::encode(Sha256::digest(&bytes));
    assert_eq!(wheel["hashes"]["sha256"].as_str(), Some(digest.as_str()));

    set_recorded_wheel(&mut lock, file_url.as_str(), actual_size + 1)?;
    lock_path.write_str(&lock.to_string())?;
    output_path.write_str("previous export\n")?;
    uv_snapshot!(context.filters(), context.export()
        .arg("--frozen")
        .arg("--no-emit-project")
        .arg("--format")
        .arg("pylock.toml")
        .arg("-o")
        .arg(output_path.path()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Artifact `file://[TEMP_DIR]/demo-1.0-py3-none-any.whl` has size 914, but the lockfile records 915
    ");
    assert_eq!(context.read("pylock.toml"), "previous export\n");
    Ok(())
}
