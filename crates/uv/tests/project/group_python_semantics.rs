use std::process::Command;

use anyhow::Result;
use assert_fs::prelude::*;
use indoc::{formatdoc, indoc};

use uv_test::{TestContext, uv_snapshot};

fn lock(context: &TestContext, without_metadata: bool) -> Command {
    let mut command = context.lock();
    command.arg("--offline");
    if without_metadata {
        command.args(["--preview-features", "lock-without-metadata"]);
    }
    command
}

/// Member and non-project root groups compare effective Python requirements without reformatting.
#[test]
fn group_python_requirements_are_semantic() -> Result<()> {
    insta::allow_duplicates! {
        for project_root in [true, false] {
            for without_metadata in [false, true] {
                let context = uv_test::test_context!("3.12");
                let prefix = if project_root {
                    indoc! {r#"
                        [project]
                        name = "project"
                        version = "0.1.0"
                        requires-python = ">=3.12"
                    "#}
                } else {
                    context.temp_dir.child("member/pyproject.toml").write_str(indoc! {r#"
                        [project]
                        name = "member"
                        version = "0.1.0"
                        requires-python = ">=3.12"
                    "#})?;
                    indoc! {r#"
                        [tool.uv.workspace]
                        members = ["member"]
                    "#}
                };
                let write_project = |base: &str, combined: &str, empty: bool| {
                    let empty = if empty { "empty = []" } else { "" };
                    context.temp_dir.child("pyproject.toml").write_str(&formatdoc! {r#"
                        {prefix}

                        [tool.uv]
                        default-groups = []

                        [dependency-groups]
                        base = []
                        combined = [{{ include-group = "base" }}]
                        {empty}

                        [tool.uv.dependency-groups]
                        base = {{ requires-python = "{base}" }}
                        combined = {{ requires-python = "{combined}" }}
                    "#})
                };
                write_project(">=3.12", ">=3.11,<3.15", true)?;
                uv_snapshot!(context.filters(), lock(&context, without_metadata), @"
                exit_code: 0 (success)
                ----- stderr -----
                Resolved 1 package in [TIME]
                ");
                let original = context.read("uv.lock");

                // The included base already supplies the stronger lower bound.
                write_project(">=3.12", "<3.15", true)?;
                uv_snapshot!(context.filters(), lock(&context, without_metadata).arg("--locked"), @"
                exit_code: 0 (success)
                ----- stderr -----
                Resolved 1 package in [TIME]
                ");
                uv_snapshot!(context.filters(), lock(&context, without_metadata), @"
                exit_code: 0 (success)
                ----- stderr -----
                Resolved 1 package in [TIME]
                ");
                assert_eq!(original, context.read("uv.lock"));

                // Declaration metadata still records the existence of unrestricted groups.
                if project_root && !without_metadata {
                    write_project(">=3.12", "<3.15", false)?;
                    uv_snapshot!(context.filters(), lock(&context, without_metadata).arg("--locked"), @"
                    exit_code: 1 (failure)
                    ----- stderr -----
                    Resolved 1 package in [TIME]
                    error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

                    hint: To update the lockfile, run `uv lock`.
                    ");
                    assert_eq!(original, context.read("uv.lock"));
                }

                for base in [">=3.12,!=3.13.*", ">=3.12,!=3.13.0.*", ">=3.13"] {
                    write_project(base, "<3.15", true)?;
                    let previous = context.read("uv.lock");
                    uv_snapshot!(context.filters(), lock(&context, without_metadata).arg("--locked"), @"
                    exit_code: 1 (failure)
                    ----- stderr -----
                    Resolved 1 package in [TIME]
                    error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

                    hint: To update the lockfile, run `uv lock`.
                    ");
                    assert_eq!(previous, context.read("uv.lock"));
                    uv_snapshot!(context.filters(), lock(&context, without_metadata), @"
                    exit_code: 0 (success)
                    ----- stderr -----
                    Resolved 1 package in [TIME]
                    ");
                }
            }
        }
        Ok(())
    }
}
