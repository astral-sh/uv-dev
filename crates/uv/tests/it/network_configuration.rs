use anyhow::Result;
use assert_fs::fixture::{FileWriteStr, PathChild};
use uv_static::EnvVars;
use uv_test::uv_snapshot;

/// Test that invalid proxy URL in uv.toml produces a helpful error message.
#[test]
fn proxy_invalid_url_in_uv_toml() {
    let context = uv_test::test_context_with_versions!(&[]);

    let uv_toml = context.temp_dir.child("uv.toml");
    uv_toml
        .write_str(indoc::indoc! {r#"
            http-proxy = "ftp://proxy.example.com:8080"
        "#})
        .unwrap();

    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg("iniconfig")
        .env_remove(EnvVars::HTTP_PROXY)
        .env_remove(EnvVars::HTTPS_PROXY), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to parse: uv.toml
      cause: TOML parse error at line 1, column 14
               |
             1 | http-proxy = "ftp://proxy.example.com:8080"
               |              ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
             invalid proxy URL scheme `ftp` in `ftp://proxy.example.com:8080/`: expected http, https, socks5, or socks5h
    "#);
}

/// Test that invalid proxy URL (not a URL) in uv.toml produces a helpful error message.
#[test]
fn proxy_invalid_url_not_a_url_in_uv_toml() {
    let context = uv_test::test_context_with_versions!(&[]);

    let uv_toml = context.temp_dir.child("uv.toml");
    uv_toml
        .write_str(indoc::indoc! {r#"
            http-proxy = "not a valid url"
        "#})
        .unwrap();

    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg("iniconfig")
        .env_remove(EnvVars::HTTP_PROXY)
        .env_remove(EnvVars::HTTPS_PROXY), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to parse: uv.toml
      cause: TOML parse error at line 1, column 14
               |
             1 | http-proxy = "not a valid url"
               |              ^^^^^^^^^^^^^^^^^
             invalid proxy URL: invalid international domain name
    "#);
}

/// Test that a SOCKS proxy URL without a host produces a configuration error.
#[test]
fn proxy_url_without_host() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);

    context
        .temp_dir
        .child("proxy.toml")
        .write_str("http-proxy = \"socks5:foo\"\n")?;
    context.temp_dir.child("requirements.in").write_str("")?;

    uv_snapshot!(context.filters(), context
        .pip_compile()
        .arg("requirements.in")
        .arg("--offline")
        .arg("--config-file")
        .arg("proxy.toml"), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to parse: proxy.toml
      cause: TOML parse error at line 1, column 14
               |
             1 | http-proxy = "socks5:foo"
               |              ^^^^^^^^^^^^
             invalid proxy URL: empty host
    "#);

    Ok(())
}
