//! Native credential storage for tests that need to exercise the host keyring.

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Result, ensure};
use assert_cmd::assert::OutputAssertExt;
use uv_static::EnvVars;
use uv_test::TestContext;

pub(super) struct NativeAuthFixture {
    context: TestContext,
    service: String,
    username: Option<String>,
    secret: &'static str,
    armed: bool,
}

impl NativeAuthFixture {
    pub(super) fn new(service: &'static str, password: &'static str) -> Result<Self> {
        let context = uv_test::test_context_with_versions!(&[]).with_real_home();
        // Each test context has a unique temporary root, so concurrent tests use different
        // accounts even when they exercise the same service.
        let username = format!(
            "uv-test-{}",
            context.root.file_name().unwrap().to_string_lossy()
        );
        let context = context.with_filter((regex::escape(&username), "[USERNAME]"));
        Self::login(context, service.to_owned(), Some(username), password)
    }

    pub(super) fn new_token(token: &'static str) -> Result<Self> {
        let context = uv_test::test_context_with_versions!(&[]).with_real_home();
        // Token credentials use the same implicit account, so isolate them by service.
        let service = format!(
            "https://uv-test-{}.invalid/basic-auth",
            hex::encode(context.root.file_name().unwrap().as_encoded_bytes())
        );
        let context = context.with_filter((regex::escape(&service), "[SERVICE]"));
        Self::login(context, service, None, token)
    }

    fn login(
        context: TestContext,
        service: String,
        username: Option<String>,
        secret: &'static str,
    ) -> Result<Self> {
        let mut fixture = Self {
            context,
            service,
            username,
            secret,
            armed: false,
        };

        let status = fixture
            .own_token()
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        ensure!(
            status.code() == Some(2),
            "refusing to overwrite a native credential for {}",
            fixture.label()
        );
        fixture.armed = true;
        let mut login = fixture.context.auth_login();
        login.arg(&fixture.service);
        if let Some(username) = &fixture.username {
            login.args(["--username", username, "--password", secret]);
        } else {
            login.args(["--token", secret]);
        }
        login
            .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth")
            .assert()
            .success()
            .stdout("")
            .stderr(format!("Stored credentials for {}\n", fixture.label()));
        Ok(fixture)
    }

    pub(super) fn token(&self, service: &str) -> Command {
        let mut command = self.context.auth_token();
        command
            .arg(service)
            .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth");
        if let Some(username) = &self.username {
            command.args(["--username", username]);
        }
        command
    }

    pub(super) fn own_token(&self) -> Command {
        self.token(&self.service)
    }

    pub(super) fn logout(&self) -> Command {
        let mut command = self.context.auth_logout();
        command
            .arg(&self.service)
            .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth");
        if let Some(username) = &self.username {
            command.args(["--username", username]);
        }
        command
    }

    pub(super) fn filters(&self) -> Vec<(&str, &str)> {
        self.context.filters()
    }

    pub(super) fn finish(mut self) -> Result<()> {
        let result = self.remove_owned_credential();
        self.armed = false;
        result
    }

    pub(super) fn finish_after_logout(mut self) -> Result<()> {
        let status = self
            .own_token()
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        ensure!(
            status.code() == Some(2),
            "native credential remained after logout for {}",
            self.label()
        );
        self.armed = false;
        Ok(())
    }

    fn label(&self) -> String {
        self.username.as_ref().map_or_else(
            || self.service.clone(),
            |username| format!("{username}@{}", self.service),
        )
    }

    fn remove_owned_credential(&self) -> Result<()> {
        let output = self.own_token().output()?;
        ensure!(
            output.status.success() && output.stdout.trim_ascii_end() == self.secret.as_bytes(),
            "refusing to remove an unverified native credential for {}",
            self.label()
        );
        let status = self
            .logout()
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        ensure!(status.success(), "failed to remove native test credential");
        Ok(())
    }
}

impl Drop for NativeAuthFixture {
    fn drop(&mut self) {
        if self.armed
            && let Err(error) = self.remove_owned_credential()
        {
            let _ = writeln!(
                std::io::stderr().lock(),
                "native test credential cleanup failed: {error:#}"
            );
        }
    }
}
