use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::{fixture::PathChild, prelude::FileWriteStr};
use uv_static::EnvVars;

use uv_test::{assert_path_missing, uv_snapshot};

#[cfg(feature = "native-auth")]
use crate::native_auth_fixture::NativeAuthFixture;

#[cfg(feature = "test-python")]
#[tokio::test]
async fn invalid_s3_endpoint_url() {
    let context = uv_test::test_context!("3.12");
    let server = cloud_auth_challenge().await;

    uv_snapshot!(context.filters(), context.pip_install()
        .args(["--dry-run", "--default-index"])
        .arg(server.uri())
        .arg("iniconfig")
        .env(EnvVars::UV_S3_ENDPOINT_URL, "not-a-url"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to fetch: http://[LOCALHOST]/iniconfig/
      cause: Invalid `UV_S3_ENDPOINT_URL`
      cause: relative URL without a base
    ");
}

#[cfg(feature = "test-python")]
#[tokio::test]
async fn invalid_gcs_endpoint_url() {
    let context = uv_test::test_context!("3.12");
    let server = cloud_auth_challenge().await;

    uv_snapshot!(context.filters(), context.pip_install()
        .args(["--dry-run", "--default-index"])
        .arg(server.uri())
        .arg("iniconfig")
        .env(EnvVars::UV_GCS_ENDPOINT_URL, "not-a-url"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to fetch: http://[LOCALHOST]/iniconfig/
      cause: Invalid `UV_GCS_ENDPOINT_URL`
      cause: relative URL without a base
    ");
}

#[cfg(feature = "test-python")]
#[tokio::test]
async fn invalid_azure_endpoint_url() {
    let context = uv_test::test_context!("3.12");
    let server = cloud_auth_challenge().await;

    uv_snapshot!(context.filters(), context.pip_install()
        .args(["--dry-run", "--default-index"])
        .arg(server.uri())
        .arg("iniconfig")
        .env(EnvVars::UV_AZURE_ENDPOINT_URL, "not-a-url"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to fetch: http://[LOCALHOST]/iniconfig/
      cause: Invalid `UV_AZURE_ENDPOINT_URL`
      cause: relative URL without a base
    ");
}

#[cfg(feature = "test-python")]
async fn cloud_auth_challenge() -> wiremock::MockServer {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    let server = MockServer::start().await;
    // A challenge triggers lazy credential lookup; an invalid endpoint must prevent a retry.
    Mock::given(method("GET"))
        .and(path("/iniconfig/"))
        .respond_with(
            ResponseTemplate::new(401).insert_header("WWW-Authenticate", r#"Basic realm="test""#),
        )
        .expect(0..=1)
        .mount(&server)
        .await;
    server
}

#[tokio::test]
#[cfg(all(
    feature = "native-auth",
    feature = "test-python",
    feature = "test-pypi"
))]
async fn add_package_native_auth_realm() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_real_home();
    let proxy = crate::pypi_proxy::start().await;
    let credential = NativeAuthFixture::guard_vacant_entry(proxy.uri(), Some("public"), "heron")?;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc::indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.11, <4"
        dependencies = []
        "#
    })?;

    // Try to add a package without credentials.
    uv_snapshot!(context.filters(), context.add().arg("anyio").arg("--default-index").arg(proxy.username_url("public", "/basic-auth/simple"))
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to add dependencies
      cause: No solution found when resolving dependencies
      cause: Because anyio was not found in the package registry and your project depends on anyio, we can conclude that your project's requirements are unsatisfiable.

    hint: An index URL (http://[LOCALHOST]/basic-auth/simple) could not be queried due to a lack of valid authentication credentials (401 Unauthorized)

    hint: If you want to add the package regardless of the failed resolution, provide the `--frozen` flag to skip locking and syncing
    "
    );

    // Login to the domain
    uv_snapshot!(context.filters(), context.auth_login()
        .arg(proxy.uri())
        .arg("--username")
        .arg("public")
        .arg("--password")
        .arg("heron")
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Stored credentials for public@http://[LOCALHOST]/
    "
    );

    // Try to add the original package without credentials again. This should use credentials
    // stored in the system keyring.
    uv_snapshot!(context.filters(), context.add().arg("anyio").arg("--default-index").arg(proxy.username_url("public", "/basic-auth/simple"))
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + sniffio==1.3.1
    "
    );

    // Logout of the domain
    uv_snapshot!(context.filters(), context.auth_logout()
        .arg(proxy.uri())
        .arg("--username")
        .arg("public")
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Removed credentials for public@http://[LOCALHOST]/
    "
    );
    credential.finish_after_logout()?;

    // Authentication should fail again
    uv_snapshot!(context.filters(), context.add().arg("iniconfig").arg("--default-index").arg(proxy.username_url("public", "/basic-auth/simple"))
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to add dependencies
      cause: No solution found when resolving dependencies
      cause: Because iniconfig was not found in the package registry and your project depends on iniconfig, we can conclude that your project's requirements are unsatisfiable.

    hint: An index URL (http://[LOCALHOST]/basic-auth/simple) could not be queried due to a lack of valid authentication credentials (401 Unauthorized)

    hint: If you want to add the package regardless of the failed resolution, provide the `--frozen` flag to skip locking and syncing
    "
    );

    Ok(())
}

#[tokio::test]
#[cfg(all(
    feature = "native-auth",
    feature = "test-python",
    feature = "test-pypi"
))]
async fn add_package_native_auth() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_real_home();
    let proxy = crate::pypi_proxy::start().await;
    let credential = NativeAuthFixture::guard_vacant_entry(
        proxy.url("/basic-auth/simple"),
        Some("public"),
        "heron",
    )?;

    // Configure `pyproject.toml` with native keyring provider.
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc::indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.11, <4"
        dependencies = []
        "#
    })?;

    // Try to add a package without credentials.
    uv_snapshot!(context.filters(), context.add().arg("anyio").arg("--default-index").arg(proxy.username_url("public", "/basic-auth/simple"))
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to add dependencies
      cause: No solution found when resolving dependencies
      cause: Because anyio was not found in the package registry and your project depends on anyio, we can conclude that your project's requirements are unsatisfiable.

    hint: An index URL (http://[LOCALHOST]/basic-auth/simple) could not be queried due to a lack of valid authentication credentials (401 Unauthorized)

    hint: If you want to add the package regardless of the failed resolution, provide the `--frozen` flag to skip locking and syncing
    "
    );

    // Login to the index
    uv_snapshot!(context.filters(), context.auth_login()
        .arg(proxy.url("/basic-auth/simple"))
        .arg("--username")
        .arg("public")
        .arg("--password")
        .arg("heron")
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Stored credentials for public@http://[LOCALHOST]/basic-auth
    "
    );

    // Try to add the original package without credentials again. This should use
    // credentials stored in the system keyring.
    uv_snapshot!(context.filters(), context.add().arg("anyio").arg("--default-index").arg(proxy.username_url("public", "/basic-auth/simple"))
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + sniffio==1.3.1
    "
    );

    // Logout of the index
    uv_snapshot!(context.filters(), context.auth_logout()
        .arg(proxy.url("/basic-auth/simple"))
        .arg("--username")
        .arg("public")
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Removed credentials for public@http://[LOCALHOST]/basic-auth
    "
    );
    credential.finish_after_logout()?;

    // Authentication should fail again
    uv_snapshot!(context.filters(), context.add().arg("iniconfig").arg("--default-index").arg(proxy.username_url("public", "/basic-auth/simple"))
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to add dependencies
      cause: No solution found when resolving dependencies
      cause: Because iniconfig was not found in the package registry and your project depends on iniconfig, we can conclude that your project's requirements are unsatisfiable.

    hint: An index URL (http://[LOCALHOST]/basic-auth/simple) could not be queried due to a lack of valid authentication credentials (401 Unauthorized)

    hint: If you want to add the package regardless of the failed resolution, provide the `--frozen` flag to skip locking and syncing
    "
    );

    Ok(())
}

#[tokio::test]
#[cfg(feature = "native-auth")]
async fn token_native_auth() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_real_home();
    let proxy = crate::pypi_proxy::start().await;

    // Without persisted credentials
    uv_snapshot!(context.filters(), context.auth_token()
        .arg(proxy.url("/basic-auth/simple"))
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to fetch credentials for `http://[LOCALHOST]/basic-auth/simple`
    ");

    // Without persisted credentials (with a username in the request)
    uv_snapshot!(context.filters(), context.auth_token()
        .arg(proxy.url("/basic-auth/simple"))
        .arg("--username")
        .arg("public")
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to fetch credentials for `public@http://[LOCALHOST]/basic-auth/simple`
    ");

    let account = NativeAuthFixture::guard_vacant_entry(
        proxy.url("/basic-auth/simple"),
        Some("public"),
        "heron",
    )?;
    uv_snapshot!(context.filters(), context.auth_login()
        .arg(proxy.url("/basic-auth/simple"))
        .arg("--username")
        .arg("public")
        .arg("--password")
        .arg("heron")
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Stored credentials for public@http://[LOCALHOST]/basic-auth
    "
    );

    // Show the credentials
    uv_snapshot!(context.filters(), context.auth_token()
        .arg(proxy.url("/basic-auth/simple"))
        .arg("--username")
        .arg("public")
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 0 (success)
    ----- stdout -----
    heron
    ");

    // Without the username
    // TODO(zanieb): Add a hint here if we can?
    uv_snapshot!(context.filters(), context.auth_token()
        .arg(proxy.url("/basic-auth/simple"))
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to fetch credentials for `http://[LOCALHOST]/basic-auth/simple`
    ");

    // With a mismatched username
    // TODO(zanieb): Add a hint here if we can?
    uv_snapshot!(context.filters(), context.auth_token()
        .arg(proxy.url("/basic-auth/simple"))
        .arg("--username")
        .arg("private")
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to fetch credentials for `private@http://[LOCALHOST]/basic-auth/simple`
    ");

    let token = NativeAuthFixture::guard_vacant_entry(
        proxy.url("/basic-auth/simple"),
        None,
        "token-secret",
    )?;
    uv_snapshot!(context.filters(), context.auth_login()
        .arg(proxy.url("/basic-auth/simple"))
        .arg("--token")
        .arg("token-secret")
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Stored credentials for http://[LOCALHOST]/basic-auth
    "
    );

    // Retrieve the token without a username
    uv_snapshot!(context.filters(), context.auth_token()
        .arg(proxy.url("/basic-auth/simple"))
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 0 (success)
    ----- stdout -----
    token-secret
    ");

    // An embedded username selects the account credential instead of the token credential.
    uv_snapshot!(context.filters(), context.auth_token()
        .arg(proxy.username_url("public", "/basic-auth/simple"))
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 0 (success)
    ----- stdout -----
    heron
    ");

    // Conflict between --username and URL username is rejected
    uv_snapshot!(context.filters(), context.auth_token()
        .arg(proxy.username_url("public", "/basic-auth/simple"))
        .arg("--username")
        .arg("different")
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Cannot specify a username both via the URL and CLI; found `--username different` and `public`
    ");

    token.finish()?;
    account.finish()
}

#[tokio::test]
#[cfg(feature = "native-auth")]
async fn token_native_auth_realm() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_real_home();
    let proxy = crate::pypi_proxy::start().await;

    // Without persisted credentials
    uv_snapshot!(context.filters(), context.auth_token()
        .arg(proxy.uri())
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to fetch credentials for `http://[LOCALHOST]/`
    ");

    // Without persisted credentials (with a username in the request)
    uv_snapshot!(context.filters(), context.auth_token()
        .arg(proxy.uri())
        .arg("--username")
        .arg("public")
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to fetch credentials for `public@http://[LOCALHOST]/`
    ");

    let account = NativeAuthFixture::guard_vacant_entry(proxy.uri(), Some("public"), "heron")?;
    uv_snapshot!(context.filters(), context.auth_login()
        .arg(proxy.uri())
        .arg("--username")
        .arg("public")
        .arg("--password")
        .arg("heron")
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Stored credentials for public@http://[LOCALHOST]/
    "
    );

    // Show the credentials
    uv_snapshot!(context.filters(), context.auth_token()
        .arg(proxy.uri())
        .arg("--username")
        .arg("public")
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 0 (success)
    ----- stdout -----
    heron
    ");

    // Show the credentials for a child URL
    uv_snapshot!(context.filters(), context.auth_token()
        .arg(proxy.url("/basic-auth/simple"))
        .arg("--username")
        .arg("public")
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 0 (success)
    ----- stdout -----
    heron
    ");

    // Without the username (defaults to __token__ which wasn't stored)
    uv_snapshot!(context.filters(), context.auth_token()
        .arg(proxy.uri())
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to fetch credentials for `http://[LOCALHOST]/`
    ");

    // Without the username (defaults to __token__ which wasn't stored)
    uv_snapshot!(context.filters(), context.auth_token()
        .arg(proxy.url("/basic-auth/simple"))
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to fetch credentials for `http://[LOCALHOST]/basic-auth/simple`
    ");

    // With a mismatched username
    // TODO(zanieb): Add a hint here if we can?
    uv_snapshot!(context.filters(), context.auth_token()
        .arg(proxy.uri())
        .arg("--username")
        .arg("private")
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to fetch credentials for `private@http://[LOCALHOST]/`
    ");

    // With a mismatched port
    uv_snapshot!(context.filters(), context.auth_token()
        .arg("https://192.0.2.1:1000")
        .arg("--username")
        .arg("public")
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to fetch credentials for `public@https://192.0.2.1:1000/`
    ");

    let token = NativeAuthFixture::guard_vacant_entry(proxy.uri(), None, "realm-token-secret")?;
    uv_snapshot!(context.filters(), context.auth_login()
        .arg(proxy.uri())
        .arg("--token")
        .arg("realm-token-secret")
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Stored credentials for http://[LOCALHOST]/
    "
    );

    // Retrieve the token without a username
    uv_snapshot!(context.filters(), context.auth_token()
        .arg(proxy.uri())
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 0 (success)
    ----- stdout -----
    realm-token-secret
    ");

    // An embedded username selects the host account credential for the child URL.
    uv_snapshot!(context.filters(), context.auth_token()
        .arg(proxy.username_url("public", "/basic-auth/simple"))
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 0 (success)
    ----- stdout -----
    heron
    ");

    token.finish()?;
    account.finish()
}

#[test]
fn login_missing_arguments() {
    let context = uv_test::test_context_with_versions!(&[]);
    let credentials_dir = context.temp_dir.child("credentials");
    let context = context.with_env(EnvVars::UV_CREDENTIALS_DIR, credentials_dir.as_os_str());

    // Without a service name
    uv_snapshot!(context.filters(), context.auth_login(), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the following required arguments were not provided:
      <SERVICE>

    Usage: uv auth login --cache-dir [CACHE_DIR] <SERVICE>

    For more information, try '--help'.
    ");

    // Without a username (or token)
    uv_snapshot!(context.filters(), context.auth_login()
        .arg("https://example.invalid/simple"), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No username provided; did you mean to provide `--username` or `--token`?
    ");

    // Without a password
    uv_snapshot!(context.filters(), context.auth_login()
        .arg("https://example.invalid/simple")
        .arg("--username")
        .arg("public"), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No password provided; did you mean to provide `--password` or `--token`?
    ");

    assert_path_missing(credentials_dir.child("credentials.toml"));
}

#[tokio::test]
#[cfg(feature = "native-auth")]
async fn logout_native_auth() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_real_home();
    let proxy = crate::pypi_proxy::start().await;
    let service = proxy.url("/basic-auth/simple");
    let token_status = context
        .auth_token()
        .arg(&service)
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    anyhow::ensure!(
        token_status.code() == Some(2),
        "refusing to remove an existing native token credential"
    );
    let credential =
        NativeAuthFixture::guard_vacant_entry(service.clone(), Some("public"), "heron")?;

    // Without a service name
    uv_snapshot!(context.filters(), context.auth_logout(), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the following required arguments were not provided:
      <SERVICE>

    Usage: uv auth logout --cache-dir [CACHE_DIR] <SERVICE>

    For more information, try '--help'.
    ");

    // Logout before logging in
    uv_snapshot!(context.filters(), context.auth_logout()
        .arg(proxy.url("/basic-auth/simple"))
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Unable to remove credentials for `http://[LOCALHOST]/basic-auth`
      cause: No matching entry found in secure storage
    ");

    // Logout before logging in (with a username)
    uv_snapshot!(context.filters(), context.auth_logout()
        .arg(proxy.url("/basic-auth/simple"))
        .arg("--username")
        .arg("public")
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Unable to remove credentials for `public@http://[LOCALHOST]/basic-auth`
      cause: No matching entry found in secure storage
    ");

    // Login with a username
    uv_snapshot!(context.filters(), context.auth_login()
        .arg(proxy.url("/basic-auth/simple"))
        .arg("--username")
        .arg("public")
        .arg("--password")
        .arg("heron")
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Stored credentials for public@http://[LOCALHOST]/basic-auth
    "
    );

    // Logout without a username
    // TODO(zanieb): Add a hint here if we can?
    uv_snapshot!(context.filters(), context.auth_logout()
        .arg(proxy.url("/basic-auth/simple"))
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Unable to remove credentials for `http://[LOCALHOST]/basic-auth`
      cause: No matching entry found in secure storage
    ");

    // Logout with a username
    uv_snapshot!(context.filters(), context.auth_logout()
        .arg(proxy.url("/basic-auth/simple"))
        .arg("--username")
        .arg("public")
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Removed credentials for public@http://[LOCALHOST]/basic-auth
    ");
    credential.finish_after_logout()?;

    // Login again
    let credential = NativeAuthFixture::guard_vacant_entry(service, Some("public"), "heron")?;
    context
        .auth_login()
        .arg(proxy.url("/basic-auth/simple"))
        .arg("--username")
        .arg("public")
        .arg("--password")
        .arg("heron")
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth")
        .assert()
        .success();

    // Logout with a username in the URL
    uv_snapshot!(context.filters(), context.auth_logout()
        .arg(proxy.username_url("public", "/basic-auth/simple"))
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Removed credentials for public@http://[LOCALHOST]/basic-auth
    ");
    credential.finish_after_logout()?;

    // Conflict between --username and a URL username is rejected
    uv_snapshot!(context.filters(), context.auth_logout()
        .arg(proxy.username_url("public", "/basic-auth/simple"))
        .arg("--username")
        .arg("foo")
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Cannot specify a username both via the URL and CLI; found `--username foo` and `public`
    ");

    // Conflict between --token and a URL username is rejected
    uv_snapshot!(context.filters(), context.auth_login()
        .arg(proxy.username_url("public", "/basic-auth/simple"))
        .arg("--token")
        .arg("foo")
        .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth"), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: When using `--token`, a username cannot not be provided; found: public
    ");

    Ok(())
}

#[test]
#[cfg(feature = "native-auth")]
fn logout_token_native_auth() -> Result<()> {
    let fixture = NativeAuthFixture::new_token("test-token")?;

    uv_snapshot!(fixture.own_token(), @r"
    exit_code: 0 (success)
    ----- stdout -----
    test-token
    "
    );

    uv_snapshot!(fixture.filters(), fixture.logout(), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Removed credentials for [SERVICE]
    ");

    fixture.finish_after_logout()
}

#[test]
fn login_service_url_normalizes_to_https() {
    let context = uv_test::test_context_with_versions!(&[]);

    // A domain-only service name gets https:// prepended
    uv_snapshot!(context.auth_login()
        .arg("example.com")
        .arg("--username")
        .arg("test")
        .arg("--password")
        .arg("domain-password"), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Stored credentials for test@https://example.com/
    ");
    uv_snapshot!(context.auth_token().args(["https://example.com/", "--username", "test"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    domain-password
    ");

    uv_snapshot!(context.auth_login()
        .arg("https://example.com")
        .arg("--username")
        .arg("test")
        .arg("--password")
        .arg("https-password"), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Stored credentials for test@https://example.com/
    ");
    uv_snapshot!(context.auth_token().args(["example.com", "--username", "test"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    https-password
    ");

    // A domain-only service with a path also gets https:// prepended
    uv_snapshot!(context.auth_login()
        .arg("example.com/simple")
        .arg("--username")
        .arg("test")
        .arg("--password")
        .arg("path-password"), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Stored credentials for test@https://example.com/
    ");
    uv_snapshot!(context.auth_token().args(["https://example.com/", "--username", "test"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    path-password
    ");
}

#[test]
fn login_rejects_invalid_service_url() {
    let context = uv_test::test_context_with_versions!(&[]);
    uv_snapshot!(context.auth_login()
        .arg("not a valid url")
        .arg("--username")
        .arg("test")
        .arg("--password")
        .arg("test"), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: invalid value 'not a valid url' for '<SERVICE>': invalid international domain name

    For more information, try '--help'.
    ");
}

#[test]
fn login_with_embedded_credentials() {
    let context = uv_test::test_context_with_versions!(&[]);
    uv_snapshot!(context.auth_login()
        .arg("https://test:password@example.com/simple"), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Stored credentials for test@https://example.com/
    ");
    uv_snapshot!(context.auth_token().args(["https://example.com/", "--username", "test"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    password
    ");
}

#[test]
fn login_with_embedded_username_and_cli_password() {
    let context = uv_test::test_context_with_versions!(&[]);
    uv_snapshot!(context.auth_login()
        .arg("https://test@example.com/simple")
        .arg("--password")
        .arg("password"), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Stored credentials for test@https://example.com/
    ");
    uv_snapshot!(context.auth_token().args(["https://example.com/", "--username", "test"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    password
    ");
}

#[test]
fn login_url_credential_conflicts() {
    let context = uv_test::test_context_with_versions!(&[]);

    // Conflict between --username and URL username is rejected
    uv_snapshot!(context.auth_login()
        .arg("https://test@example.com/simple")
        .arg("--username")
        .arg("different")
        .arg("--password")
        .arg("password"), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Cannot specify a username both via the URL and CLI; found `--username different` and `test`
    ");

    // Conflict between --password and URL password is rejected
    uv_snapshot!(context.auth_login()
        .arg("https://test:password@example.com/simple")
        .arg("--password")
        .arg("different"), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Cannot specify a password both via the URL and CLI
    ");

    // Conflict between --token and URL credentials is rejected
    uv_snapshot!(context.auth_login()
        .arg("https://test:password@example.com/simple")
        .arg("--token")
        .arg("some-token"), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: When using `--token`, a username cannot not be provided; found: test
    ");
}

#[tokio::test]
async fn login_text_store() {
    let context = uv_test::test_context_with_versions!(&[]);
    let proxy = crate::pypi_proxy::start().await;

    // Login with a username and password
    uv_snapshot!(context.filters(), context.auth_login()
        .arg(proxy.url("/basic-auth/simple"))
        .arg("--username")
        .arg("public")
        .arg("--password")
        .arg("heron"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Stored credentials for public@http://[LOCALHOST]/basic-auth
    "
    );

    // Login with a token
    uv_snapshot!(context.filters(), context.auth_login()
        .arg("https://example.com/simple")
        .arg("--token")
        .arg("test-token"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Stored credentials for https://example.com/
    "
    );

    // Empty username should fail
    uv_snapshot!(context.filters(), context.auth_login()
        .arg("https://example.com/simple")
        .arg("--username")
        .arg("")
        .arg("--password")
        .arg("testpass"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Username cannot be empty
    "
    );

    // Empty password should fail
    uv_snapshot!(context.filters(), context.auth_login()
        .arg("https://example.com/simple")
        .arg("--username")
        .arg("testuser")
        .arg("--password")
        .arg(""), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Password cannot be empty
    "
    );

    // HTTP should fail
    uv_snapshot!(context.filters(), context.auth_login()
        .arg("http://example.com/simple")
        .arg("--username")
        .arg("testuser")
        .arg("--password")
        .arg("testpass"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: invalid value 'http://example.com/simple' for '<SERVICE>': HTTPS is required for non-local hosts

    For more information, try '--help'.
    ");

    // Other protocol should fail
    uv_snapshot!(context.filters(), context.auth_login()
        .arg("ftp://example.com/simple")
        .arg("--username")
        .arg("testuser")
        .arg("--password")
        .arg("testpass"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: invalid value 'ftp://example.com/simple' for '<SERVICE>': Unsupported scheme: ftp

    For more information, try '--help'.
    ");

    // HTTP should be allowed on localhost
    uv_snapshot!(context.filters(), context.auth_login()
        .arg("http://127.0.0.1/simple")
        .arg("--username")
        .arg("testuser")
        .arg("--password")
        .arg("testpass"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Stored credentials for testuser@http://127.0.0.1/
    ");

    // HTTP should be allowed on localhost
    uv_snapshot!(context.filters(), context.auth_login()
        .arg("http://localhost/simple")
        .arg("--username")
        .arg("testuser")
        .arg("--password")
        .arg("testpass"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Stored credentials for testuser@http://localhost/
    ");

    // HTTP should be allowed on IPv6 loopback
    uv_snapshot!(context.filters(), context.auth_login()
        .arg("http://[::1]:1324/simple")
        .arg("--username")
        .arg("testuser")
        .arg("--password")
        .arg("testpass"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Stored credentials for testuser@http://[::1]:1324/
    ");
}

#[test]
fn login_text_store_empty_file() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    context.temp_dir.child("credentials.toml").write_str("")?;

    uv_snapshot!(context.auth_login()
        .args(["https://example.com/simple", "--token", "test-token"])
        .env(EnvVars::UV_CREDENTIALS_DIR, context.temp_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Stored credentials for https://example.com/
    ");

    uv_snapshot!(context.auth_token()
        .arg("https://example.com/simple")
        .env(EnvVars::UV_CREDENTIALS_DIR, context.temp_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    test-token
    ");
    Ok(())
}

#[test]
fn login_text_store_comments_only_file() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    context
        .temp_dir
        .child("credentials.toml")
        .write_str("# No credentials have been configured yet.\n")?;

    uv_snapshot!(context.auth_login()
        .args(["https://example.com/simple", "--token", "test-token"])
        .env(EnvVars::UV_CREDENTIALS_DIR, context.temp_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Stored credentials for https://example.com/
    ");

    uv_snapshot!(context.auth_token()
        .arg("https://example.com/simple")
        .env(EnvVars::UV_CREDENTIALS_DIR, context.temp_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    test-token
    ");
    Ok(())
}

#[test]
#[expect(clippy::disallowed_types)]
fn login_password_stdin() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);

    // Create a temporary file with the password
    let password_file = context.temp_dir.child("password.txt");
    password_file.write_str("secret-password")?;

    // Login with password from stdin
    uv_snapshot!(context.auth_login()
        .arg("https://example.com/simple")
        .arg("--username")
        .arg("testuser")
        .arg("--password")
        .arg("-")
        .stdin(std::fs::File::open(password_file)?), @"
    exit_code: 0 (success)
    ----- stderr -----
    Stored credentials for testuser@https://example.com/
    "
    );

    // Verify the credentials work by retrieving the token
    uv_snapshot!(context.auth_token()
        .arg("https://example.com/simple")
        .arg("--username")
        .arg("testuser"), @"
    exit_code: 0 (success)
    ----- stdout -----
    secret-password
    "
    );

    Ok(())
}

#[test]
#[expect(clippy::disallowed_types)]
fn login_token_stdin() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);

    // Create a temporary file with the token
    let token_file = context.temp_dir.child("token.txt");
    token_file.write_str("secret-token")?;

    // Login with token from stdin
    uv_snapshot!(context.auth_login()
        .arg("https://example.com/simple")
        .arg("--token")
        .arg("-")
        .stdin(std::fs::File::open(token_file)?), @"
    exit_code: 0 (success)
    ----- stderr -----
    Stored credentials for https://example.com/
    "
    );

    // Verify the credentials work by retrieving the token
    uv_snapshot!(context.auth_token()
        .arg("https://example.com/simple"), @"
    exit_code: 0 (success)
    ----- stdout -----
    secret-token
    "
    );

    Ok(())
}

#[tokio::test]
async fn token_text_store() {
    let context = uv_test::test_context_with_versions!(&[]);
    let proxy = crate::pypi_proxy::start().await;

    // Login first
    context
        .auth_login()
        .arg(proxy.url("/basic-auth/simple"))
        .arg("--username")
        .arg("public")
        .arg("--password")
        .arg("heron")
        .assert()
        .success();

    // Retrieve the token
    uv_snapshot!(context.filters(), context.auth_token()
        .arg(proxy.url("/basic-auth/simple"))
        .arg("--username")
        .arg("public"), @"
    exit_code: 0 (success)
    ----- stdout -----
    heron
    "
    );

    // Login with token
    context
        .auth_login()
        .arg("https://example.com/simple")
        .arg("--token")
        .arg("test-token")
        .assert()
        .success();

    // Retrieve token without username
    uv_snapshot!(context.filters(), context.auth_token()
        .arg("https://example.com/simple"), @"
    exit_code: 0 (success)
    ----- stdout -----
    test-token
    "
    );

    // Empty username should fail
    uv_snapshot!(context.filters(), context.auth_token()
        .arg("https://example.com/simple")
        .arg("--username")
        .arg(""), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Username cannot be empty
    "
    );
}

#[tokio::test]
async fn logout_text_store() {
    let context = uv_test::test_context_with_versions!(&[]);
    let proxy = crate::pypi_proxy::start().await;

    // Login first
    context
        .auth_login()
        .arg(proxy.url("/basic-auth/simple"))
        .arg("--username")
        .arg("public")
        .arg("--password")
        .arg("heron")
        .assert()
        .success();

    // Logout
    uv_snapshot!(context.filters(), context.auth_logout()
        .arg(proxy.url("/basic-auth/simple"))
        .arg("--username")
        .arg("public"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed credentials for public@http://[LOCALHOST]/basic-auth
    "
    );

    // Login with token then logout
    context
        .auth_login()
        .arg("https://example.com/simple")
        .arg("--token")
        .arg("test-token")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.auth_logout()
        .arg("https://example.com/simple"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed credentials for https://example.com/
    "
    );

    // Empty username should fail
    uv_snapshot!(context.filters(), context.auth_logout()
        .arg("https://example.com/simple")
        .arg("--username")
        .arg(""), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Username cannot be empty
    "
    );
}

#[tokio::test]
async fn auth_disabled_provider_uses_text_store() {
    let context = uv_test::test_context_with_versions!(&[]);
    let proxy = crate::pypi_proxy::start().await;

    // Login with disabled provider should use text store
    uv_snapshot!(context.filters(), context.auth_login()
        .arg(proxy.url("/basic-auth/simple"))
        .arg("--username")
        .arg("public")
        .arg("--password")
        .arg("heron")
        .arg("--keyring-provider")
        .arg("disabled"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Stored credentials for public@http://[LOCALHOST]/basic-auth
    "
    );

    // Token retrieval should work with disabled provider
    uv_snapshot!(context.filters(), context.auth_token()
        .arg(proxy.url("/basic-auth/simple"))
        .arg("--username")
        .arg("public")
        .arg("--keyring-provider")
        .arg("disabled"), @"
    exit_code: 0 (success)
    ----- stdout -----
    heron
    "
    );
}

#[test]
fn login_text_store_strips_simple_suffix() {
    let context = uv_test::test_context_with_versions!(&[]);

    // Login with `/simple` suffix - should strip it and store credentials for the root URL
    uv_snapshot!(context.auth_login()
        .arg("https://example.com/simple")
        .arg("--username")
        .arg("testuser")
        .arg("--password")
        .arg("testpass"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Stored credentials for testuser@https://example.com/
    "
    );

    // Login with `/+simple` suffix (devpi format) - should also strip it
    uv_snapshot!(context.auth_login()
        .arg("https://devpi.example.com/root/+simple")
        .arg("--username")
        .arg("devpiuser")
        .arg("--password")
        .arg("devpipass"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Stored credentials for devpiuser@https://devpi.example.com/root
    "
    );

    // Login with `/Simple` (case insensitive) - should strip it
    uv_snapshot!(context.auth_login()
        .arg("https://registry.example.com/Simple")
        .arg("--username")
        .arg("caseuser")
        .arg("--password")
        .arg("casepass"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Stored credentials for caseuser@https://registry.example.com/
    "
    );

    // Login without `/simple` suffix - should store as-is
    uv_snapshot!(context.auth_login()
        .arg("https://custom.example.com/api/v1")
        .arg("--username")
        .arg("apiuser")
        .arg("--password")
        .arg("apipass"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Stored credentials for apiuser@https://custom.example.com/api/v1
    "
    );

    // Login with trailing slash and `/simple` - should strip both
    uv_snapshot!(context.auth_login()
        .arg("https://trailing.example.com/simple/")
        .arg("--username")
        .arg("slashuser")
        .arg("--password")
        .arg("slashpass"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Stored credentials for slashuser@https://trailing.example.com/
    "
    );
}

#[test]
fn logout_text_store_strips_simple_suffix() {
    let context = uv_test::test_context_with_versions!(&[]);

    // Login with `/simple` suffix first
    context
        .auth_login()
        .arg("https://example.com/simple")
        .arg("--username")
        .arg("testuser")
        .arg("--password")
        .arg("testpass")
        .assert()
        .success();

    // Logout using the same URL with `/simple` - should work
    uv_snapshot!(context.auth_logout()
        .arg("https://example.com/simple")
        .arg("--username")
        .arg("testuser"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed credentials for testuser@https://example.com/
    "
    );

    // Login with `/+simple` suffix
    context
        .auth_login()
        .arg("https://devpi.example.com/root/+simple")
        .arg("--username")
        .arg("devpiuser")
        .arg("--password")
        .arg("devpipass")
        .assert()
        .success();

    // Logout using URL with `/+simple` - should work
    uv_snapshot!(context.auth_logout()
        .arg("https://devpi.example.com/root/+simple")
        .arg("--username")
        .arg("devpiuser"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed credentials for devpiuser@https://devpi.example.com/root
    "
    );
}

#[test]
fn token_text_store_strips_simple_suffix() {
    let context = uv_test::test_context_with_versions!(&[]);

    // Login with `/simple` suffix
    context
        .auth_login()
        .arg("https://example.com/simple")
        .arg("--username")
        .arg("testuser")
        .arg("--password")
        .arg("testpass")
        .assert()
        .success();

    // Retrieve token using URL with `/simple` - should work
    uv_snapshot!(context.auth_token()
        .arg("https://example.com/simple")
        .arg("--username")
        .arg("testuser"), @"
    exit_code: 0 (success)
    ----- stdout -----
    testpass
    "
    );

    // Login with token and `/simple` suffix
    context
        .auth_login()
        .arg("https://token.example.com/simple")
        .arg("--token")
        .arg("secret-token")
        .assert()
        .success();

    // Retrieve token using URL with `/simple` - should work
    uv_snapshot!(context.auth_token()
        .arg("https://token.example.com/simple"), @"
    exit_code: 0 (success)
    ----- stdout -----
    secret-token
    "
    );
}

#[test]
fn token_text_store_username() {
    let context = uv_test::test_context_with_versions!(&[]);

    // Login with specific username
    context
        .auth_login()
        .arg("https://example.com/simple")
        .arg("--username")
        .arg("testuser")
        .arg("--password")
        .arg("testpass")
        .assert()
        .success();

    // Retrieve token with matching username
    uv_snapshot!(context.auth_token()
        .arg("https://example.com/simple")
        .arg("--username")
        .arg("testuser"), @"
    exit_code: 0 (success)
    ----- stdout -----
    testpass
    "
    );

    // Retrieve token without username
    uv_snapshot!(context.auth_token()
        .arg("https://example.com/simple"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to fetch credentials for `https://example.com/simple`
    "
    );

    // Try to retrieve token with different username - should fail
    uv_snapshot!(context.auth_token()
        .arg("https://example.com/simple")
        .arg("--username")
        .arg("wronguser"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to fetch credentials for `wronguser@https://example.com/simple`
    "
    );

    // Login with token (no username)
    context
        .auth_login()
        .arg("https://token.example.com/simple")
        .arg("--token")
        .arg("test-token")
        .assert()
        .success();

    // Retrieve token without specifying username - should work
    uv_snapshot!(context.auth_token()
        .arg("https://token.example.com/simple"), @"
    exit_code: 0 (success)
    ----- stdout -----
    test-token
    "
    );

    // Login with username at different URL
    context
        .auth_login()
        .arg("https://userexample.com/simple")
        .arg("--username")
        .arg("testuser")
        .arg("--password")
        .arg("testpass")
        .assert()
        .success();

    // Retrieve token without username should fail
    uv_snapshot!(context.auth_token()
        .arg("https://userexample.com/simple"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to fetch credentials for `https://userexample.com/simple`
    "
    );
}

#[test]
fn logout_text_store_multiple_usernames() {
    let context = uv_test::test_context_with_versions!(&[]);

    // Login with two different usernames for the same service
    context
        .auth_login()
        .arg("https://example.com/simple")
        .arg("--username")
        .arg("user1")
        .arg("--password")
        .arg("pass1")
        .assert()
        .success();

    context
        .auth_login()
        .arg("https://example.com/simple")
        .arg("--username")
        .arg("user2")
        .arg("--password")
        .arg("pass2")
        .assert()
        .success();

    // Logout one specific username
    uv_snapshot!(context.auth_logout()
        .arg("https://example.com/simple")
        .arg("--username")
        .arg("user1"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed credentials for user1@https://example.com/
    "
    );

    // Verify the first user is gone but second remains
    uv_snapshot!(context.auth_token()
        .arg("https://example.com/simple")
        .arg("--username")
        .arg("user1"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to fetch credentials for `user1@https://example.com/simple`
    "
    );

    uv_snapshot!(context.auth_token()
        .arg("https://example.com/simple")
        .arg("--username")
        .arg("user2"), @"
    exit_code: 0 (success)
    ----- stdout -----
    pass2
    "
    );

    // Try to logout without specifying username (defaults to `__token__`)
    uv_snapshot!(context.auth_logout()
        .arg("https://example.com/simple"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No matching entry found for `https://example.com/`
    "
    );
}

#[test]
#[cfg(feature = "native-auth")]
fn native_auth_path_login_matches_child_path() -> Result<()> {
    let fixture = NativeAuthFixture::new("https://example.invalid/api", "testpass")?;

    uv_snapshot!(fixture.token("https://example.invalid/api/v1"), @r"
    exit_code: 0 (success)
    ----- stdout -----
    testpass
    "
    );

    fixture.finish()
}

#[test]
#[cfg(feature = "native-auth")]
fn native_auth_host_fallback() -> Result<()> {
    let fixture = NativeAuthFixture::new("https://example.invalid/", "hostpass")?;

    uv_snapshot!(fixture.token("https://example.invalid/any/path"), @r"
    exit_code: 0 (success)
    ----- stdout -----
    hostpass
    "
    );

    uv_snapshot!(fixture.filters(), fixture.token("https://other.invalid/any/path"), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to fetch credentials for `[USERNAME]@https://other.invalid/any/path`
    "
    );

    fixture.finish()
}

/// Test credential helper with basic auth credentials
#[test]
fn bazel_helper_basic_auth() {
    let context = uv_test::test_context_with_versions!(&[]);

    // Store credentials
    uv_snapshot!(context.filters(), context.auth_login()
        .arg("https://test.example.com")
        .arg("--username").arg("testuser")
        .arg("--password").arg("testpass"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Stored credentials for testuser@https://test.example.com/
    ");

    uv_snapshot!(context.filters(), context.auth_helper()
        .arg("--protocol=bazel")
        .arg("get"),
        input=r#"{"uri":"https://test.example.com/path"}"#,
        @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {"headers":{"Authorization":["Basic dGVzdHVzZXI6dGVzdHBhc3M="]}}

    ----- stderr -----
    warning: The `uv auth helper` command is experimental and may change without warning. Pass `--preview-features auth-helper` to disable this warning
    "#
    );
}

/// Test credential helper with token credentials
#[test]
fn bazel_helper_token() {
    let context = uv_test::test_context_with_versions!(&[]);

    // Store token
    uv_snapshot!(context.filters(), context.auth_login()
        .arg("https://api.example.com")
        .arg("--token").arg("mytoken123"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Stored credentials for https://api.example.com/
    ");

    // Test credential helper - tokens are stored as Basic auth with __token__ username
    uv_snapshot!(context.filters(), context.auth_helper()
        .arg("--protocol=bazel")
        .arg("get"),
        input=r#"{"uri":"https://api.example.com/v1/endpoint"}"#,
        @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {"headers":{"Authorization":["Basic X190b2tlbl9fOm15dG9rZW4xMjM="]}}

    ----- stderr -----
    warning: The `uv auth helper` command is experimental and may change without warning. Pass `--preview-features auth-helper` to disable this warning
    "#
    );
}

#[test]
fn bazel_helper_invalid_bearer_token() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    context
        .temp_dir
        .child("credentials.toml")
        .write_str(indoc::indoc! { r#"
            [[credential]]
            service = "https://example.com"
            scheme = "bearer"
            token = "secret\nvalue"
        "# })?;

    uv_snapshot!(context.filters(), context.auth_helper()
        .arg("--protocol=bazel")
        .arg("get")
        .arg("--offline")
        .env(EnvVars::UV_PREVIEW_FEATURES, "auth-helper")
        .env(EnvVars::UV_CREDENTIALS_DIR, context.temp_dir.as_os_str()),
        input=r#"{"uri":"https://example.com"}"#,
        @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Invalid authorization header
      cause: failed to parse header value
    "
    );

    Ok(())
}

/// Test credential helper with no credentials found
#[test]
fn bazel_helper_no_credentials() {
    let context = uv_test::test_context_with_versions!(&[]);
    uv_snapshot!(context.filters(), context.auth_helper()
        .arg("--protocol=bazel")
        .arg("get"),
        input=r#"{"uri":"https://unknown.example.com/path"}"#,
        @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {"headers":{}}

    ----- stderr -----
    warning: The `uv auth helper` command is experimental and may change without warning. Pass `--preview-features auth-helper` to disable this warning
    "#
    );
}

/// Test credential helper with invalid JSON input
#[test]
fn bazel_helper_invalid_json() {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.filters(), context.auth_helper()
        .arg("--protocol=bazel")
        .arg("get"),
        input="not json",
        @"
    exit_code: 2 (failure)
    ----- stderr -----
    warning: The `uv auth helper` command is experimental and may change without warning. Pass `--preview-features auth-helper` to disable this warning
    error: Failed to parse credential request as JSON
      cause: expected ident at line 1 column 2
    "
    );
}

/// Test credential helper with invalid URI
#[test]
fn bazel_helper_invalid_uri() {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.filters(), context.auth_helper()
        .arg("--protocol=bazel")
        .arg("get"),
        input=r#"{"uri":"not a url"}"#,
        @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    warning: The `uv auth helper` command is experimental and may change without warning. Pass `--preview-features auth-helper` to disable this warning
    error: Failed to parse credential request as JSON
      cause: relative URL without a base: "not a url" at line 1 column 18
    "#
    );
}

/// Test credential helper with username in URI
#[test]
fn bazel_helper_username_in_uri() {
    let context = uv_test::test_context_with_versions!(&[]);

    // Store credentials with specific username
    uv_snapshot!(context.filters(), context.auth_login()
        .arg("https://test.example.com")
        .arg("--username").arg("specificuser")
        .arg("--password").arg("specificpass"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Stored credentials for specificuser@https://test.example.com/
    ");

    // Test with username in URI
    uv_snapshot!(context.filters(), context.auth_helper()
        .arg("--protocol=bazel")
        .arg("get"),
        input=r#"{"uri":"https://specificuser@test.example.com/path"}"#,
        @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {"headers":{"Authorization":["Basic c3BlY2lmaWN1c2VyOnNwZWNpZmljcGFzcw=="]}}

    ----- stderr -----
    warning: The `uv auth helper` command is experimental and may change without warning. Pass `--preview-features auth-helper` to disable this warning
    "#
    );
}

/// Test credential helper with unknown username in URI
#[test]
fn bazel_helper_unknown_username_in_uri() {
    let context = uv_test::test_context_with_versions!(&[]);

    // Store credentials with specific username
    uv_snapshot!(context.filters(), context.auth_login()
        .arg("https://test.example.com")
        .arg("--username").arg("specificuser")
        .arg("--password").arg("specificpass"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Stored credentials for specificuser@https://test.example.com/
    ");

    // Test with username in URI
    uv_snapshot!(context.filters(), context.auth_helper()
        .arg("--protocol=bazel")
        .arg("get"),
        input=r#"{"uri":"https://differentuser@test.example.com/path"}"#,
        @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {"headers":{}}

    ----- stderr -----
    warning: The `uv auth helper` command is experimental and may change without warning. Pass `--preview-features auth-helper` to disable this warning
    "#
    );
}
