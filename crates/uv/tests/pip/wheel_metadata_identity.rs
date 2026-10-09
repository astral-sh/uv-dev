use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::TcpListener;
use std::process::Stdio;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use serde_json::json;
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use uv_static::EnvVars;
use uv_test::archive::generate_source_archive;
use uv_test::package_server::PackageServer;
use uv_test::packse::{generate_wheel, mount_mismatched_distribution};
use uv_test::uv_snapshot;

#[test]
fn local_wheel_version_mismatch_precedes_dependency_build() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let marker = context.temp_dir.child("backend-ran");
    let dependency = context.temp_dir.child("child-1.0.tar.gz");
    dependency.write_binary(&generate_source_archive(
        &"child".parse()?,
        &"1.0".parse()?,
        "",
        Some(marker.path()),
    )?)?;
    let dependency_url = Url::from_file_path(dependency.path()).expect("absolute fixture path");
    let (_, bytes) = generate_wheel(
        &"demo".parse()?,
        &"2.0".parse()?,
        &[format!("child @ {dependency_url}").parse()?],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    let wheel = context.temp_dir.child("demo-1.0-py3-none-any.whl");
    wheel.write_binary(&bytes)?;
    context
        .temp_dir
        .child("requirements.in")
        .write_str(&format!(
            "demo @ {}",
            Url::from_file_path(wheel.path()).expect("absolute fixture path")
        ))?;

    let output = uv_snapshot!(context.filters(), context.pip_compile()
        .arg("requirements.in").arg("--no-index"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to read `demo @ file://[TEMP_DIR]/demo-1.0-py3-none-any.whl`
      cause: Wheel metadata version `2.0` for `demo` does not match `1.0` from the wheel filename. If this is intentional, set `UV_SKIP_WHEEL_FILENAME_CHECK=1`.
    ");
    assert_eq!(output.status.code(), Some(1));
    assert!(!marker.exists());

    Ok(())
}

#[test]
fn complete_local_wheel_version_mismatch_precedes_dependency_build() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let marker = context.temp_dir.child("backend-ran");
    let dependency = context.temp_dir.child("child-1.0.tar.gz");
    dependency.write_binary(&generate_source_archive(
        &"child".parse()?,
        &"1.0".parse()?,
        "",
        Some(marker.path()),
    )?)?;
    let dependency_url = Url::from_file_path(dependency.path()).expect("absolute fixture path");
    let (_, bytes) = generate_wheel(
        &"demo".parse()?,
        &"2.0".parse()?,
        &[format!("child @ {dependency_url}").parse()?],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    let wheel = context.temp_dir.child("demo-1.0-py3-none-any.whl");
    wheel.write_binary(&bytes)?;
    context
        .temp_dir
        .child("requirements.in")
        .write_str(&format!(
            "demo @ {}",
            Url::from_file_path(wheel.path()).expect("absolute fixture path")
        ))?;

    let output = uv_snapshot!(context.filters(), context.pip_compile()
        .arg("requirements.in").arg("--no-index").arg("--generate-hashes"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to read `demo @ file://[TEMP_DIR]/demo-1.0-py3-none-any.whl`
      cause: Package metadata version `2.0` does not match `1.0` from the wheel filename
    ");
    assert_eq!(output.status.code(), Some(1));
    assert!(!marker.exists());
    Ok(())
}

#[tokio::test]
async fn streamed_wheel_version_mismatch() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let filename = "demo-1.0-py3-none-any.whl";
    let (_, bytes) = generate_wheel(
        &"demo".parse()?,
        &"2.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    Mock::given(method("HEAD"))
        .respond_with(ResponseTemplate::new(405))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/{filename}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes))
        .expect(1)
        .mount(&server)
        .await;
    context
        .temp_dir
        .child("requirements.in")
        .write_str(&format!("demo @ {}/{filename}", server.uri()))?;
    let output = uv_snapshot!(context.filters(), context.pip_compile()
        .arg("requirements.in").arg("--no-index"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download `demo @ http://[LOCALHOST]/demo-1.0-py3-none-any.whl`
      cause: Wheel metadata version `2.0` for `demo` does not match `1.0` from the wheel filename. If this is intentional, set `UV_SKIP_WHEEL_FILENAME_CHECK=1`.
    ");
    assert_eq!(output.status.code(), Some(1));
    Ok(())
}

#[tokio::test]
async fn range_wheel_version_mismatch() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let filename = "demo-1.0-py3-none-any.whl";
    let (_, bytes) = generate_wheel(
        &"demo".parse()?,
        &"2.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    mount_mismatched_distribution(
        &server,
        &format!("/{filename}"),
        filename,
        bytes.clone(),
        bytes,
    )
    .await;
    context
        .temp_dir
        .child("requirements.in")
        .write_str(&format!("demo @ {}/{filename}", server.uri()))?;
    let output = uv_snapshot!(context.filters(), context.pip_compile()
        .arg("requirements.in").arg("--no-index"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download `demo @ http://[LOCALHOST]/demo-1.0-py3-none-any.whl`
      cause: Wheel metadata version `2.0` for `demo` does not match `1.0` from the wheel filename. If this is intentional, set `UV_SKIP_WHEEL_FILENAME_CHECK=1`.
    ");
    assert_eq!(output.status.code(), Some(1));
    Ok(())
}

#[tokio::test]
async fn mismatched_sidecar_is_not_cached() -> Result<()> {
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
    server
        .serve_with(&filename, &bytes, None, json!({ "core-metadata": true }))
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/{filename}.metadata")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Cache-Control", "public, max-age=86400")
                .set_body_string("Metadata-Version: 2.3\nName: demo\nVersion: 2.0\n"),
        )
        .expect(2)
        .mount(server.mock_server())
        .await;
    context
        .temp_dir
        .child("requirements.in")
        .write_str("demo==1.0")?;
    let output = uv_snapshot!(context.filters(), context.pip_compile()
        .arg("requirements.in").arg("--index-url").arg(server.index_url()), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because demo==1.0 has inconsistent metadata and you require demo==1.0, we can conclude that your requirements are unsatisfiable.

    hint: Metadata for `demo` (v1.0) was inconsistent:
      Wheel metadata version `2.0` for `demo` does not match `1.0` from the wheel filename. If this is intentional, set `UV_SKIP_WHEEL_FILENAME_CHECK=1`.
    ");
    assert_eq!(output.status.code(), Some(1));

    // Rejected metadata is fetched again instead of being reused from the cache.
    let output = uv_snapshot!(context.filters(), context.pip_compile()
        .arg("requirements.in").arg("--index-url").arg(server.index_url()), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because demo==1.0 has inconsistent metadata and you require demo==1.0, we can conclude that your requirements are unsatisfiable.

    hint: Metadata for `demo` (v1.0) was inconsistent:
      Wheel metadata version `2.0` for `demo` does not match `1.0` from the wheel filename. If this is intentional, set `UV_SKIP_WHEEL_FILENAME_CHECK=1`.
    ");
    assert_eq!(output.status.code(), Some(1));
    Ok(())
}

#[tokio::test]
async fn cached_sidecar_version_is_rechecked() -> Result<()> {
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
    server
        .serve_with(&filename, &bytes, None, json!({ "core-metadata": true }))
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/{filename}.metadata")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Cache-Control", "public, max-age=86400")
                .set_body_string("Metadata-Version: 2.3\nName: demo\nVersion: 2.0\n"),
        )
        .expect(1)
        .mount(server.mock_server())
        .await;
    context
        .temp_dir
        .child("requirements.in")
        .write_str("demo==1.0")?;
    uv_snapshot!(context.filters(), context.pip_compile()
        .arg("requirements.in")
        .arg("--index-url")
        .arg(server.index_url())
        .env(EnvVars::UV_SKIP_WHEEL_FILENAME_CHECK, "1"), @"
    exit_code: 0 (success)
    ----- stdout -----
    # This file was autogenerated by uv via the following command:
    #    uv pip compile --cache-dir [CACHE_DIR] requirements.in
    demo==1.0
        # via -r requirements.in

    ----- stderr -----
    Resolved 1 package in [TIME]
    ");

    let output = uv_snapshot!(context.filters(), context.pip_compile()
        .arg("requirements.in").arg("--index-url").arg(server.index_url())
        .arg("--offline"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because demo==1.0 has inconsistent metadata and you require demo==1.0, we can conclude that your requirements are unsatisfiable.

    hint: Metadata for `demo` (v1.0) was inconsistent:
      Wheel metadata version `2.0` for `demo` does not match `1.0` from the wheel filename. If this is intentional, set `UV_SKIP_WHEEL_FILENAME_CHECK=1`.
    ");
    assert_eq!(output.status.code(), Some(1));

    uv_snapshot!(context.filters(), context.pip_compile()
        .arg("requirements.in")
        .arg("--index-url")
        .arg(server.index_url())
        .arg("--offline")
        .env(EnvVars::UV_SKIP_WHEEL_FILENAME_CHECK, "1"), @"
    exit_code: 0 (success)
    ----- stdout -----
    # This file was autogenerated by uv via the following command:
    #    uv pip compile --cache-dir [CACHE_DIR] requirements.in --offline
    demo==1.0
        # via -r requirements.in

    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    Ok(())
}

#[test]
fn wheel_metadata_can_omit_local_version() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let (_, bytes) = generate_wheel(
        &"demo".parse()?,
        &"1.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    let wheel = context.temp_dir.child("demo-1.0+local-py3-none-any.whl");
    wheel.write_binary(&bytes)?;
    context
        .temp_dir
        .child("requirements.in")
        .write_str(&format!(
            "demo @ {}",
            Url::from_file_path(wheel.path()).expect("absolute fixture path")
        ))?;
    uv_snapshot!(context.filters(), context.pip_compile()
        .arg("requirements.in")
        .arg("--no-index"), @"
    exit_code: 0 (success)
    ----- stdout -----
    # This file was autogenerated by uv via the following command:
    #    uv pip compile --cache-dir [CACHE_DIR] requirements.in --no-index
    demo @ file://[TEMP_DIR]/demo-1.0+local-py3-none-any.whl
        # via -r requirements.in

    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    Ok(())
}

#[test]
fn complete_wheel_metadata_can_omit_local_version() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let (_, bytes) = generate_wheel(
        &"demo".parse()?,
        &"1.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    let wheel = context.temp_dir.child("demo-1.0+local-py3-none-any.whl");
    wheel.write_binary(&bytes)?;
    context
        .temp_dir
        .child("requirements.in")
        .write_str(&format!(
            "demo @ {}",
            Url::from_file_path(wheel.path()).expect("absolute fixture path")
        ))?;
    uv_snapshot!(context.filters(), context.pip_compile()
        .arg("requirements.in")
        .arg("--no-index")
        .arg("--generate-hashes"), @r"
    exit_code: 0 (success)
    ----- stdout -----
    # This file was autogenerated by uv via the following command:
    #    uv pip compile --cache-dir [CACHE_DIR] requirements.in --no-index --generate-hashes
    demo @ file://[TEMP_DIR]/demo-1.0+local-py3-none-any.whl \
        --hash=sha256:7644bb2ffdaa9ba75b83b45ae61389f32968aff000923cf8d5d5c4fb7309ef25
        # via -r requirements.in

    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    Ok(())
}

#[test]
fn installed_wheel_metadata_can_omit_local_version() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let (_, bytes) = generate_wheel(
        &"demo".parse()?,
        &"1.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    let wheel = context.temp_dir.child("demo-1.0+local-py3-none-any.whl");
    wheel.write_binary(&bytes)?;
    uv_snapshot!(context.filters(), context.pip_install()
        .arg(wheel.path())
        .arg("--no-index"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + demo==1.0+local (from file://[TEMP_DIR]/demo-1.0+local-py3-none-any.whl)
    ");
    Ok(())
}

/// A complete-wheel mismatch rejects the candidate without consuming its dependencies.
#[test]
fn complete_wheel_version_mismatch_backtracks() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let marker = context.temp_dir.child("backend-ran");
    let wheels = context.temp_dir.child("wheels");
    wheels.create_dir_all()?;
    let dependency = wheels.child("child-1.0.tar.gz");
    dependency.write_binary(&generate_source_archive(
        &"child".parse()?,
        &"1.0".parse()?,
        "",
        Some(marker.path()),
    )?)?;
    let (_, invalid) = generate_wheel(
        &"demo".parse()?,
        &"3.0".parse()?,
        &["child==1.0".parse()?],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    let (filename, valid) = generate_wheel(
        &"demo".parse()?,
        &"1.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    wheels
        .child("demo-2.0-py3-none-any.whl")
        .write_binary(&invalid)?;
    wheels.child(filename).write_binary(&valid)?;
    context
        .temp_dir
        .child("requirements.in")
        .write_str("demo")?;

    uv_snapshot!(context.filters(), context.pip_compile()
        .arg("requirements.in")
        .arg("--no-index")
        .arg("--find-links").arg("wheels")
        .arg("--generate-hashes"), @r"
    exit_code: 0 (success)
    ----- stdout -----
    # This file was autogenerated by uv via the following command:
    #    uv pip compile --cache-dir [CACHE_DIR] requirements.in --no-index --generate-hashes
    demo==1.0 \
        --hash=sha256:7644bb2ffdaa9ba75b83b45ae61389f32968aff000923cf8d5d5c4fb7309ef25
        # via -r requirements.in

    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    assert_eq!(
        fs_err::symlink_metadata(marker.path())
            .expect_err("rejected wheel dependencies must not run their backend")
            .kind(),
        ErrorKind::NotFound,
    );
    Ok(())
}

/// Process an inconsistent speculative wheel before learning the constraint that excludes it.
#[tokio::test]
async fn speculative_wheel_version_mismatch_can_be_excluded() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let constrainer_url = format!(
        "http://{}/constrainer-1.0-py3-none-any.whl",
        listener.local_addr()?
    );
    listener.set_nonblocking(true)?;
    let (release, wait) = mpsc::channel();
    let gate = thread::spawn(move || -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        bail!("constrainer metadata was not requested");
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(error.into()),
            }
        };
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        stream.set_write_timeout(Some(Duration::from_secs(30)))?;
        let mut reader = BufReader::new(&mut stream);
        let mut request = String::new();
        reader.read_line(&mut request)?;
        if !request.starts_with("GET /constrainer-1.0-py3-none-any.whl.metadata ") {
            bail!("unexpected metadata request: {request}");
        }
        loop {
            let mut header = String::new();
            if reader.read_line(&mut header)? == 0 {
                bail!("metadata request ended before its headers");
            }
            if header == "\r\n" {
                break;
            }
        }
        drop(reader);
        wait.recv_timeout(Duration::from_secs(30))
            .context("candidate metadata was not processed before the constrainer response")?;
        let metadata =
            "Metadata-Version: 2.3\nName: constrainer\nVersion: 1.0\nRequires-Dist: candidate<2\n";
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{metadata}",
            metadata.len()
        )?;
        Ok(())
    });

    Mock::given(method("GET"))
        .and(path("/simple/constrainer/"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(
                json!({
                    "meta": {"api-version": "1.0"},
                    "name": "constrainer",
                    "files": [{
                        "filename": "constrainer-1.0-py3-none-any.whl",
                        "url": constrainer_url,
                        "hashes": {},
                        "upload-time": "2024-01-01T00:00:00Z",
                        "core-metadata": true
                    }]
                })
                .to_string(),
                "application/vnd.pypi.simple.v1+json",
            ),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/simple/candidate/"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            json!({
                "meta": {"api-version": "1.0"},
                "name": "candidate",
                "files": [
                    {"filename": "candidate-1.0-py3-none-any.whl", "url": format!("{}/candidate-1.0-py3-none-any.whl", server.uri()), "hashes": {}, "upload-time": "2024-01-01T00:00:00Z", "core-metadata": true},
                    {"filename": "candidate-2.0-py3-none-any.whl", "url": format!("{}/candidate-2.0-py3-none-any.whl", server.uri()), "hashes": {}, "upload-time": "2024-01-01T00:00:00Z", "core-metadata": true}
                ]
            }).to_string(),
            "application/vnd.pypi.simple.v1+json",
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/candidate-2.0-py3-none-any.whl.metadata"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("Metadata-Version: 2.3\nName: candidate\nVersion: 3.0\n"),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/candidate-1.0-py3-none-any.whl.metadata"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("Metadata-Version: 2.3\nName: candidate\nVersion: 1.0\n"),
        )
        .mount(&server)
        .await;
    context
        .temp_dir
        .child("requirements.in")
        .write_str("constrainer==1\ncandidate>=1\n")?;

    // The singleton is selected first, but both root requirements are prefetched.
    let mut child = context
        .pip_compile()
        .arg("requirements.in")
        .arg("--index-url")
        .arg(format!("{}/simple", server.uri()))
        .env(EnvVars::UV_CONCURRENT_DOWNLOADS, "2")
        .env(EnvVars::UV_HTTP_RETRIES, "0")
        .env(EnvVars::RUST_LOG, "uv_resolver=trace")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stderr = child.stderr.take().context("child stderr must be piped")?;
    let stderr = thread::spawn(move || -> Result<(Vec<u8>, bool)> {
        let mut reader = BufReader::new(stderr);
        let mut captured = Vec::new();
        let mut processed = false;
        loop {
            let mut line = Vec::new();
            if reader.read_until(b'\n', &mut line)? == 0 {
                break;
            }
            if !processed
                && String::from_utf8_lossy(&line)
                    .contains("`candidate==2.0` has inconsistent metadata")
            {
                processed = true;
                let _ = release.send(());
            }
            captured.extend_from_slice(&line);
        }
        Ok((captured, processed))
    });
    let mut output = child.wait_with_output()?;
    let (captured, processed) = stderr.join().expect("stderr reader must not panic")?;
    output.stderr = captured;
    let gate_result = gate.join().expect("metadata gate must not panic");
    let output = output.assert().success();
    gate_result?;
    assert!(
        processed,
        "{}",
        String::from_utf8_lossy(&output.get_output().stderr)
    );
    insta::with_settings!({filters => context.filters()}, {
        insta::assert_snapshot!(String::from_utf8_lossy(&output.get_output().stdout), @"
        # This file was autogenerated by uv via the following command:
        #    uv pip compile --cache-dir [CACHE_DIR] requirements.in
        candidate==1.0
            # via
            #   -r requirements.in
            #   constrainer
        constrainer==1.0
            # via -r requirements.in
        ");
    });
    Ok(())
}
