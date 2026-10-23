use std::process::Command;

use fs_err as fs;
use uv_static::EnvVars;

fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_uv-build"));
    command
        .env(EnvVars::NO_COLOR, "1")
        .env(EnvVars::UV_NO_WRAP, "1")
        .env_remove(EnvVars::RUST_LOG)
        .env_remove(EnvVars::UV_PREVIEW)
        .env_remove(EnvVars::UV_PREVIEW_FEATURES);
    command
}

#[test]
fn backend_hints_and_filename_protocol() -> anyhow::Result<()> {
    let project = tempfile::tempdir()?;
    fs::create_dir_all(project.path().join("src/project"))?;
    fs::write(project.path().join("src/project/__init__.py"), "")?;
    fs::create_dir(project.path().join("dist"))?;
    let pyproject = "[project]\nname = 'project'\nversion = '1.0.0'\n[build-system]\nrequires = ['uv_build']\nbuild-backend = 'uv_build'\n";
    fs::write(
        project.path().join("pyproject.toml"),
        format!("{pyproject}\n[tool.uv.build-backend]\nsource-include = ['**/@test']\n"),
    )?;
    let output = command()
        .current_dir(project.path())
        .args(["build-sdist", "dist"])
        .output()?;
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    insta::assert_snapshot!(String::from_utf8(output.stderr)?, @"
    error: Unsupported glob expression in: tool.uv.build-backend.source-include
      cause: Invalid character `@` at position 3 in glob `**/@test`

    hint: Characters can be escaped with a backslash
    ");

    fs::write(project.path().join("pyproject.toml"), pyproject)?;
    let output = command()
        .current_dir(project.path())
        .args(["build-wheel", "dist"])
        .output()?;
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout)?,
        "project-1.0.0-py3-none-any.whl\n"
    );
    assert!(
        project
            .path()
            .join("dist/project-1.0.0-py3-none-any.whl")
            .is_file()
    );
    Ok(())
}

#[test]
fn argument_errors_use_the_same_boundary() -> anyhow::Result<()> {
    let output = command().output()?;
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    insta::assert_snapshot!(String::from_utf8(output.stderr)?, @"
    error: Missing command
    ");
    Ok(())
}
