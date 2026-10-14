//! Native credential storage for tests that need to exercise the host keyring.

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Result, ensure};
use assert_cmd::assert::OutputAssertExt;
use uv_static::EnvVars;
use uv_test::TestContext;

pub(super) struct NativeAuthFixture {
    context: TestContext,
    service: &'static str,
    username: String,
    password: &'static str,
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
        let mut fixture = Self {
            context,
            service,
            username,
            password,
            armed: false,
        };

        let status = fixture
            .token(service)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        ensure!(
            status.code() == Some(2),
            "refusing to overwrite a native credential for {}@{service}",
            fixture.username
        );
        fixture.armed = true;
        fixture
            .context
            .auth_login()
            .arg(service)
            .arg("--username")
            .arg(&fixture.username)
            .arg("--password")
            .arg(password)
            .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth")
            .assert()
            .success()
            .stdout("")
            .stderr(format!(
                "Stored credentials for {}@{service}\n",
                fixture.username
            ));
        Ok(fixture)
    }

    pub(super) fn token(&self, service: &str) -> Command {
        let mut command = self.context.auth_token();
        command
            .arg(service)
            .arg("--username")
            .arg(&self.username)
            .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth");
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

    fn remove_owned_credential(&self) -> Result<()> {
        let output = self.token(self.service).output()?;
        ensure!(
            output.status.success() && output.stdout.trim_ascii_end() == self.password.as_bytes(),
            "refusing to remove an unverified native credential for {}@{}",
            self.username,
            self.service
        );
        let status = self
            .context
            .auth_logout()
            .arg(self.service)
            .arg("--username")
            .arg(&self.username)
            .env(EnvVars::UV_PREVIEW_FEATURES, "native-auth")
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
