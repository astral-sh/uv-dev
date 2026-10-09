use std::fmt::Write;

use anyhow::Result;
use assert_fs::prelude::*;

use uv_static::EnvVars;
use uv_test::packse::{PackseServer, scenario::Scenario};
use uv_test::{get_bin, uv_snapshot};

#[test]
fn adjust_open_file_limit() {
    let context = uv_test::test_context!("3.12");
    let python = &context.python_versions[0].1;

    let mut command = context.external_command("sh");
    command
        .arg("-c")
        .arg("ulimit -S -n 128; exec \"$@\"")
        .arg("sh")
        .arg(get_bin!())
        .arg("run")
        .arg("--no-project")
        .arg("--")
        .arg(python)
        .arg("-c")
        .arg("import resource; print(resource.getrlimit(resource.RLIMIT_NOFILE)[0] > 128)")
        .env(EnvVars::UV_CACHE_DIR, context.cache_dir.path());

    uv_snapshot!(context.filters(), command, @r"
    exit_code: 0 (success)
    ----- stdout -----
    True
    ");
}

#[test]
fn run_open_file_limit_override() {
    let context = uv_test::test_context!("3.12");
    let python = &context.python_versions[0].1;

    let mut command = context.run();
    command
        .arg("--no-project")
        .arg("--")
        .arg(python)
        .arg("-c")
        .arg(
            "import resource; soft, hard = resource.getrlimit(resource.RLIMIT_NOFILE); print(soft); print(hard > soft)",
        )
        .env(EnvVars::UV_RUN_RLIMIT_NOFILE, "128");

    uv_snapshot!(context.filters(), command, @r"
    exit_code: 0 (success)
    ----- stdout -----
    128
    True
    ");
}

#[test]
fn run_open_file_limit_override_invalid() {
    let context = uv_test::test_context!("3.12");
    let python = &context.python_versions[0].1;

    let mut command = context.run();
    command
        .arg("--no-project")
        .arg("--")
        .arg(python)
        .arg("-c")
        .arg("pass")
        .env(EnvVars::UV_RUN_RLIMIT_NOFILE, "invalid");

    uv_snapshot!(context.filters(), command, @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to parse environment variable `UV_RUN_RLIMIT_NOFILE` with invalid value `invalid`: invalid digit found in string
    ");
}

#[test]
fn run_open_file_limit_override_exceeds_hard_limit() {
    let context = uv_test::test_context!("3.12");
    let python = &context.python_versions[0].1;

    let mut command = context.external_command("sh");
    command
        .arg("-c")
        .arg("ulimit -S -n 128; ulimit -H -n 128; exec \"$@\"")
        .arg("sh")
        .arg(get_bin!())
        .arg("run")
        .arg("--no-project")
        .arg("--")
        .arg(python)
        .arg("-c")
        .arg("pass")
        .env(EnvVars::UV_CACHE_DIR, context.cache_dir.path())
        .env(EnvVars::UV_RUN_RLIMIT_NOFILE, "256");

    uv_snapshot!(context.filters(), command, @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to apply `UV_RUN_RLIMIT_NOFILE` value `256`
      cause: requested open file limit (256) exceeds the hard limit (128)
    ");
}

/// Wide metadata requests must wait before opening cache locks under a low hard descriptor limit.
#[test]
fn metadata_downloads_respect_low_file_limit() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let mut fixture =
        String::from("name = \"metadata-file-limit\"\n[root]\n[expected]\nsatisfiable = true\n");
    let mut requirements = String::new();
    for number in 0..96 {
        writeln!(
            fixture,
            "[packages.package-{number:03}.versions.\"1.0.0\"]\nsdist = false"
        )?;
        writeln!(requirements, "package-{number:03}==1.0.0")?;
    }
    let server = PackseServer::from_scenario(&toml::from_str::<Scenario>(&fixture)?);
    context
        .temp_dir
        .child("requirements.in")
        .write_str(&requirements)?;
    uv_snapshot!(context.filters(), context.external_command("sh")
        .arg("-c").arg("ulimit -S -n 64; ulimit -H -n 64; exec \"$@\"")
        .arg("sh").arg(get_bin!())
        .args(["pip", "compile", "requirements.in", "--quiet", "--no-header", "--no-annotate", "--no-build", "--python"])
        .arg(&context.python_versions[0].1)
        .arg("--index-url").arg(server.index_url())
        .env(EnvVars::UV_CACHE_DIR, context.cache_dir.path())
        .env(EnvVars::UV_CONCURRENT_DOWNLOADS, "1"), @"
    exit_code: 0 (success)
    ----- stdout -----
    package-000==1.0.0
    package-001==1.0.0
    package-002==1.0.0
    package-003==1.0.0
    package-004==1.0.0
    package-005==1.0.0
    package-006==1.0.0
    package-007==1.0.0
    package-008==1.0.0
    package-009==1.0.0
    package-010==1.0.0
    package-011==1.0.0
    package-012==1.0.0
    package-013==1.0.0
    package-014==1.0.0
    package-015==1.0.0
    package-016==1.0.0
    package-017==1.0.0
    package-018==1.0.0
    package-019==1.0.0
    package-020==1.0.0
    package-021==1.0.0
    package-022==1.0.0
    package-023==1.0.0
    package-024==1.0.0
    package-025==1.0.0
    package-026==1.0.0
    package-027==1.0.0
    package-028==1.0.0
    package-029==1.0.0
    package-030==1.0.0
    package-031==1.0.0
    package-032==1.0.0
    package-033==1.0.0
    package-034==1.0.0
    package-035==1.0.0
    package-036==1.0.0
    package-037==1.0.0
    package-038==1.0.0
    package-039==1.0.0
    package-040==1.0.0
    package-041==1.0.0
    package-042==1.0.0
    package-043==1.0.0
    package-044==1.0.0
    package-045==1.0.0
    package-046==1.0.0
    package-047==1.0.0
    package-048==1.0.0
    package-049==1.0.0
    package-050==1.0.0
    package-051==1.0.0
    package-052==1.0.0
    package-053==1.0.0
    package-054==1.0.0
    package-055==1.0.0
    package-056==1.0.0
    package-057==1.0.0
    package-058==1.0.0
    package-059==1.0.0
    package-060==1.0.0
    package-061==1.0.0
    package-062==1.0.0
    package-063==1.0.0
    package-064==1.0.0
    package-065==1.0.0
    package-066==1.0.0
    package-067==1.0.0
    package-068==1.0.0
    package-069==1.0.0
    package-070==1.0.0
    package-071==1.0.0
    package-072==1.0.0
    package-073==1.0.0
    package-074==1.0.0
    package-075==1.0.0
    package-076==1.0.0
    package-077==1.0.0
    package-078==1.0.0
    package-079==1.0.0
    package-080==1.0.0
    package-081==1.0.0
    package-082==1.0.0
    package-083==1.0.0
    package-084==1.0.0
    package-085==1.0.0
    package-086==1.0.0
    package-087==1.0.0
    package-088==1.0.0
    package-089==1.0.0
    package-090==1.0.0
    package-091==1.0.0
    package-092==1.0.0
    package-093==1.0.0
    package-094==1.0.0
    package-095==1.0.0
    ");
    Ok(())
}
