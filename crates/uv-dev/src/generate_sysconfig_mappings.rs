//! Generate sysconfig mappings for supported python-build-standalone *nix platforms.
use anstream::println;
use anyhow::{Result, bail};
use pretty_assertions::StrComparison;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::PathBuf;

use uv_client::{BaseClientBuilder, RetryState};

use crate::ROOT_DIR;
use crate::generate_all::Mode;

/// Contains current supported targets
const TARGETS_YML_URL: &str = "https://raw.githubusercontent.com/astral-sh/python-build-standalone/refs/tags/20260901/cpython-unix/targets.yml";

// Preserve compiler paths embedded in older downloadable python-build-standalone releases.
const HISTORICAL_CC_VALUES: [&str; 3] = [
    "/usr/bin/aarch64-linux-gnu-gcc",
    "/usr/bin/riscv64-linux-gnu-clang",
    "/usr/bin/riscv64-linux-gnu-gcc",
];
const HISTORICAL_CXX_VALUES: [&str; 3] = [
    "/usr/bin/aarch64-linux-gnu-g++",
    "/usr/bin/riscv64-linux-gnu-clang++",
    "/usr/bin/riscv64-linux-gnu-g++",
];

#[derive(clap::Args)]
pub(crate) struct Args {
    #[arg(long, default_value_t, value_enum)]
    pub(crate) mode: Mode,
}

#[derive(Debug, Deserialize)]
struct TargetConfig {
    host_cc: Option<String>,
    host_cxx: Option<String>,
    target_cc: Option<String>,
    target_cxx: Option<String>,
}

pub(crate) async fn main(args: &Args) -> Result<()> {
    let reference_string = generate().await?;
    let filename = "generated_mappings.rs";
    let reference_path = PathBuf::from(ROOT_DIR)
        .join("crates")
        .join("uv-python")
        .join("src")
        .join("sysconfig")
        .join(filename);

    match args.mode {
        Mode::DryRun => {
            println!("{reference_string}");
        }
        Mode::Check => match fs_err::read_to_string(reference_path) {
            Ok(current) => {
                if current == reference_string {
                    println!("Up-to-date: {filename}");
                } else {
                    let comparison = StrComparison::new(&current, &reference_string);
                    bail!(
                        "{filename} changed, please run `cargo dev generate-sysconfig-metadata`:\n{comparison}"
                    );
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                bail!("{filename} not found, please run `cargo dev generate-sysconfig-metadata`");
            }
            Err(err) => {
                bail!(
                    "{filename} changed, please run `cargo dev generate-sysconfig-metadata`:\n{err}"
                );
            }
        },
        Mode::Write => match fs_err::read_to_string(&reference_path) {
            Ok(current) => {
                if current == reference_string {
                    println!("Up-to-date: {filename}");
                } else {
                    println!("Updating: {filename}");
                    fs_err::write(reference_path, reference_string.as_bytes())?;
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                println!("Updating: {filename}");
                fs_err::write(reference_path, reference_string.as_bytes())?;
            }
            Err(err) => {
                bail!(
                    "{filename} changed, please run `cargo dev generate-sysconfig-metadata`:\n{err}"
                );
            }
        },
    }

    Ok(())
}

async fn generate() -> Result<String> {
    println!("Downloading python-build-standalone cpython-unix/targets.yml ...");
    let parsed = download_targets(TARGETS_YML_URL, BaseClientBuilder::default()).await?;

    let mut replacements: BTreeMap<&str, BTreeMap<String, String>> = BTreeMap::new();

    for targets_config in parsed.values() {
        for sysconfig_cc_entry in ["CC", "LDSHARED", "BLDSHARED", "LINKCC"] {
            if let Some(ref from_cc) = targets_config.host_cc {
                replacements
                    .entry(sysconfig_cc_entry)
                    .or_default()
                    .insert(from_cc.to_owned(), "cc".to_string());
            }
            if let Some(ref from_cc) = targets_config.target_cc {
                replacements
                    .entry(sysconfig_cc_entry)
                    .or_default()
                    .insert(from_cc.to_owned(), "cc".to_string());
            }
        }
        for sysconfig_cxx_entry in ["CXX", "LDCXXSHARED"] {
            if let Some(ref from_cxx) = targets_config.host_cxx {
                replacements
                    .entry(sysconfig_cxx_entry)
                    .or_default()
                    .insert(from_cxx.to_owned(), "c++".to_string());
            }
            if let Some(ref from_cxx) = targets_config.target_cxx {
                replacements
                    .entry(sysconfig_cxx_entry)
                    .or_default()
                    .insert(from_cxx.to_owned(), "c++".to_string());
            }
        }
    }

    for sysconfig_cc_entry in ["CC", "LDSHARED", "BLDSHARED", "LINKCC"] {
        for from_cc in HISTORICAL_CC_VALUES {
            replacements
                .entry(sysconfig_cc_entry)
                .or_default()
                .insert(from_cc.to_string(), "cc".to_string());
        }
    }
    for sysconfig_cxx_entry in ["CXX", "LDCXXSHARED"] {
        for from_cxx in HISTORICAL_CXX_VALUES {
            replacements
                .entry(sysconfig_cxx_entry)
                .or_default()
                .insert(from_cxx.to_string(), "c++".to_string());
        }
    }

    let mut output = String::new();

    // Opening statements
    output.push_str("//! DO NOT EDIT\n");
    output.push_str("//!\n");
    output.push_str("//! Generated with `cargo run dev generate-sysconfig-metadata`\n");
    output.push_str("//! Targets from <https://github.com/astral-sh/python-build-standalone/blob/20260901/cpython-unix/targets.yml>\n");
    output.push_str("//!\n");

    // Disable clippy/fmt
    output.push_str("#![allow(clippy::all)]\n");
    output.push_str("#![cfg_attr(any(), rustfmt::skip)]\n\n");

    // Begin main code
    output.push_str("use std::collections::BTreeMap;\n");
    output.push_str("use std::sync::LazyLock;\n\n");
    output.push_str("use crate::sysconfig::replacements::{ReplacementEntry, ReplacementMode};\n\n");

    output.push_str(
        "/// Mapping for sysconfig keys to lookup and replace with the appropriate entry.\n",
    );
    output.push_str("pub(crate) static DEFAULT_VARIABLE_UPDATES: LazyLock<BTreeMap<String, Vec<ReplacementEntry>>> = LazyLock::new(|| {\n");
    output.push_str("    BTreeMap::from_iter([\n");

    // Add Replacement Entries for CC, CXX, etc.
    for (key, entries) in &replacements {
        writeln!(output, "        (\"{key}\".to_string(), vec![")?;
        for (from, to) in entries {
            writeln!(
                output,
                "            ReplacementEntry {{ mode: ReplacementMode::Partial {{ from: \"{from}\".to_string() }}, to: \"{to}\".to_string() }},"
            )?;
        }
        writeln!(output, "        ]),")?;
    }

    // Add AR case last
    output.push_str("        (\"AR\".to_string(), vec![\n");
    output.push_str("            ReplacementEntry {\n");
    output.push_str("                mode: ReplacementMode::Full,\n");
    output.push_str("                to: \"ar\".to_string(),\n");
    output.push_str("            },\n");
    output.push_str("        ]),\n");

    // Closing
    output.push_str("    ])\n});\n");

    Ok(output)
}

async fn download_targets(
    url: &str,
    client_builder: BaseClientBuilder<'_>,
) -> Result<BTreeMap<String, TargetConfig>> {
    let retry_policy = client_builder.retry_policy();
    // One retry budget covers both the request and reading its complete body.
    let client = client_builder.retries(0).build()?;
    let url = url.parse()?;
    let http_client = client.for_host(&url);
    let mut retry_state = RetryState::start(retry_policy, url.clone());
    let body = loop {
        let result: Result<String> = async {
            let response = retry_state.send(http_client.get(url.as_str())).await?;
            Ok(response.error_for_status()?.text().await?)
        }
        .await;
        match result {
            Ok(body) => break body,
            Err(error) => {
                let Some(backoff) = retry_state.should_retry(error.as_ref(), 0) else {
                    return Err(error);
                };
                retry_state.sleep_backoff(backoff).await;
            }
        }
    };

    Ok(serde_yaml::from_str(&body)?)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::env;
    use std::time::Duration;

    use anyhow::{Result, bail};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;

    use uv_client::{BaseClientBuilder, DEFAULT_RETRIES};
    use uv_static::EnvVars;

    use crate::generate_all::Mode;

    use super::{Args, TargetConfig, download_targets, main};

    const TARGETS: &str = "linux:\n  host_cc: /usr/bin/test-gcc\n";

    #[derive(Clone, Copy)]
    enum Reply {
        Disconnect,
        Http(u16, &'static str),
        Truncated,
    }

    async fn request_targets(
        replies: &[Reply],
    ) -> Result<(Result<BTreeMap<String, TargetConfig>>, usize)> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/targets.yml", listener.local_addr()?);
        let replies = replies.to_vec();
        let (stop, mut stopped) = oneshot::channel();
        let mut server = tokio::spawn(async move {
            let mut requests = 0;
            loop {
                let (mut stream, _) = tokio::select! {
                    () = async { let _ = (&mut stopped).await; } => break,
                    connection = listener.accept() => connection?,
                };
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    if request.len() == 8192 {
                        bail!("test request headers exceed the bound");
                    }
                    request.push(stream.read_u8().await?);
                }
                assert!(request.starts_with(b"GET /targets.yml HTTP/1.1\r\n"));
                let reply = replies
                    .get(requests)
                    .copied()
                    .unwrap_or(Reply::Http(503, ""));
                requests += 1;
                match reply {
                    Reply::Disconnect => {}
                    Reply::Http(status, body) => {
                        let response = format!(
                            "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        stream.write_all(response.as_bytes()).await?;
                    }
                    Reply::Truncated => {
                        stream
                            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\nlinux:")
                            .await?;
                    }
                }
                stream.shutdown().await?;
            }
            Ok::<_, anyhow::Error>(requests)
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()?;
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            download_targets(
                &url,
                BaseClientBuilder::default()
                    .custom_client(client)
                    .no_retry_delay(true),
            ),
        )
        .await;
        let _ = stop.send(());
        let requests = match tokio::time::timeout(Duration::from_secs(5), &mut server).await {
            Ok(result) => result??,
            Err(error) => {
                server.abort();
                let _ = server.await;
                return Err(error.into());
            }
        };
        Ok((result?, requests))
    }

    #[tokio::test]
    async fn targets_retry_transient_failures() -> Result<()> {
        for first in [
            Reply::Disconnect,
            Reply::Http(503, "unavailable"),
            Reply::Truncated,
        ] {
            let (targets, requests) = request_targets(&[first, Reply::Http(200, TARGETS)]).await?;
            let targets = targets?;
            assert_eq!(
                targets["linux"].host_cc.as_deref(),
                Some("/usr/bin/test-gcc")
            );
            assert_eq!(requests, 2);
        }
        Ok(())
    }

    #[tokio::test]
    async fn targets_retry_budget_is_shared() -> Result<()> {
        let (targets, requests) = request_targets(&[
            Reply::Http(503, "unavailable"),
            Reply::Truncated,
            Reply::Http(503, "unavailable"),
            Reply::Truncated,
            Reply::Http(200, TARGETS),
        ])
        .await?;
        assert!(targets.is_err());
        assert_eq!(requests, DEFAULT_RETRIES as usize + 1);
        Ok(())
    }

    #[tokio::test]
    async fn targets_permanent_errors_are_not_retried() -> Result<()> {
        for reply in [Reply::Http(403, "forbidden"), Reply::Http(404, "missing")] {
            let (targets, requests) = request_targets(&[reply, Reply::Http(200, TARGETS)]).await?;
            assert!(targets.is_err());
            assert_eq!(requests, 1);
        }
        Ok(())
    }

    #[tokio::test]
    async fn targets_invalid_yaml_is_not_retried() -> Result<()> {
        let (targets, requests) = request_targets(&[
            Reply::Http(200, "linux: [not-a-target"),
            Reply::Http(200, TARGETS),
        ])
        .await?;
        assert!(targets.is_err());
        assert_eq!(requests, 1);
        Ok(())
    }

    #[tokio::test]
    async fn test_generate_sysconfig_mappings() -> Result<()> {
        // Skip this test in CI to avoid redundancy with the dedicated CI job
        if env::var_os(EnvVars::CI).is_some() {
            return Ok(());
        }

        let mode = if env::var(EnvVars::UV_UPDATE_SCHEMA).as_deref() == Ok("1") {
            Mode::Write
        } else {
            Mode::Check
        };
        main(&Args { mode }).await
    }
}
