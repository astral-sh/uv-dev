//! Remote requirements and constraints files.

use anyhow::Result;
use indoc::indoc;
use wiremock::matchers::{basic_auth, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use uv_test::packse::PackseServer;
use uv_test::packse::scenario::Scenario;
use uv_test::uv_snapshot;

async fn remote_constraints_fixture() -> Result<(MockServer, PackseServer)> {
    let scenario: Scenario = toml::from_str(indoc! {r#"
        name = "remote-constraints"
        [root]
        [expected]
        satisfiable = true
        [packages.typing-extensions.versions."4.7.1"]
        [packages.typing-extensions.versions."4.8.2"]
    "#})?;
    let index = PackseServer::from_scenario(&scenario);
    let constraints = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/constraints.txt"))
        .respond_with(ResponseTemplate::new(200).set_body_string("typing-extensions==4.7.1\n"))
        .expect(1)
        .mount(&constraints)
        .await;
    Ok((constraints, index))
}

/// A remote constraint selects the older available version.
#[tokio::test]
async fn constraint_url_on_command_line() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let (constraints, index) = remote_constraints_fixture().await?;
    uv_snapshot!(context.pip_install()
        .arg("-c")
        .arg(format!("{}/constraints.txt", constraints.uri()))
        .arg("typing_extensions>=4.0")
        .arg("--index-url")
        .arg(index.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + typing-extensions==4.7.1
    ");
    context.assert_installed("typing_extensions", "4.7.1");
    Ok(())
}

/// A local requirements file can include a remote constraint.
#[tokio::test]
async fn constraint_url_in_requirements_file() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let (constraints, index) = remote_constraints_fixture().await?;
    fs_err::write(
        context.temp_dir.join("requirements.txt"),
        format!(
            "typing-extensions>=4.0\n-c {}/constraints.txt\n",
            constraints.uri()
        ),
    )?;
    uv_snapshot!(context.pip_install()
        .args(["-r", "requirements.txt", "--index-url"])
        .arg(index.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + typing-extensions==4.7.1
    ");
    context.assert_installed("typing_extensions", "4.7.1");
    Ok(())
}

/// Re-read an authenticated remote requirements file when its contents change.
#[tokio::test]
async fn install_remote_requirements_txt() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filtered_http_retries();
    let server = MockServer::start().await;
    let original_requirements = Mock::given(method("GET"))
        .and(path("/requirements.txt"))
        .and(basic_auth("user", "password"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok==1.0.0"))
        .with_priority(1)
        .expect(1)
        .mount_as_scoped(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/requirements.txt"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1..)
        .mount(&server)
        .await;

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("-r")
        .arg(format!("{}/requirements.txt", server.uri())), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Error while accessing remote requirements file: http://[LOCALHOST]/requirements.txt
    "
    );

    let requirements_url = format!("http://user:password@{}/requirements.txt", server.address());
    let links = context.workspace_root.join("test/links");
    uv_snapshot!(context.pip_install()
        .arg("-r")
        .arg(&requirements_url)
        .arg("--no-index")
        .arg("--find-links")
        .arg(&links), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + ok==1.0.0
    "
    );

    drop(original_requirements);
    Mock::given(method("GET"))
        .and(path("/requirements.txt"))
        .and(basic_auth("user", "password"))
        .respond_with(ResponseTemplate::new(200).set_body_string("tqdm==1000.0.0"))
        .with_priority(1)
        .expect(1)
        .mount(&server)
        .await;

    uv_snapshot!(context.pip_install()
        .arg("-r")
        .arg(&requirements_url)
        .arg("--no-index")
        .arg("--find-links")
        .arg(&links), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + tqdm==1000.0.0
    "
    );

    context.assert_command("import ok; import tqdm").success();

    Ok(())
}

/// Install a package from a relative include in a remote `requirements.txt`.
#[tokio::test]
async fn install_remote_requirements_txt_with_relative_include() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/nested/requirements.txt"))
        .respond_with(ResponseTemplate::new(200).set_body_string("-r child.txt"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/nested/child.txt"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok==1.0.0"))
        .expect(1)
        .mount(&server)
        .await;

    uv_snapshot!(context.pip_install()
        .arg("-r")
        .arg(format!("{}/nested/requirements.txt", server.uri()))
        .arg("--no-index")
        .arg("--find-links")
        .arg(context.workspace_root.join("test/links")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + ok==1.0.0
    "
    );

    context.assert_installed("ok", "1.0.0");

    Ok(())
}

/// Avoid exposing expanded environment variables from remote requirements files.
#[tokio::test]
async fn install_remote_requirements_txt_redacts_nested_url() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/requirements.txt"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(
                "-r https://user/name:${UV_TEST_SECRET}@example.com/requirements.txt",
            ),
        )
        .mount(&server)
        .await;

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("-r")
        .arg(format!("{}/requirements.txt", server.uri()))
        .env("UV_TEST_SECRET", "super-secret"), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Invalid requirements input in `http://[LOCALHOST]/requirements.txt` at position 0: ambiguous user/pass authority in URL (not percent-encoded?): https:***@example.com/requirements.txt
      cause: ambiguous user/pass authority in URL (not percent-encoded?): https:***@example.com/requirements.txt
    "
    );

    Ok(())
}
