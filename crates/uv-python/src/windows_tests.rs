//! Exercise Windows interpreter discovery with executable batch-file fixtures.

use std::assert_matches;
use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use anyhow::Result;
use assert_fs::{TempDir, prelude::*};
use temp_env::with_vars;
use test_log::test;

use uv_cache::Cache;
use uv_platform_tags::Os;
use uv_static::EnvVars;

use crate::discovery::find_python_installation;
use crate::test_utils::mock_interpreter_response;
use crate::{
    EnvironmentPreference, ImplementationName, PythonNotFound, PythonPreference, PythonRequest,
    PythonSource,
};

struct TestContext {
    tempdir: TempDir,
    cache: Cache,
    search_path: Vec<PathBuf>,
}

impl TestContext {
    fn new() -> Result<Self> {
        Ok(Self {
            tempdir: TempDir::new()?,
            cache: Cache::temp()?,
            search_path: Vec::new(),
        })
    }

    fn add_python(&mut self, directory: &str, version: &str) -> Result<PathBuf> {
        let path = self.add_search_path(directory)?.join("python.bat");
        let response = mock_interpreter_response(
            &path,
            &version.parse().expect("Test uses a valid Python version"),
            ImplementationName::CPython,
            true,
            false,
        )?;
        Self::write_response(&path, &response)?;
        Ok(path)
    }

    fn add_search_path(&mut self, name: &str) -> Result<PathBuf> {
        let directory = self.tempdir.child(name);
        directory.create_dir_all()?;
        let directory = directory.to_path_buf();
        self.search_path.push(directory.clone());
        Ok(directory)
    }

    fn write_response(path: &Path, response: &str) -> Result<()> {
        // `type` is a cmd.exe built-in, so the fixture also works with an isolated PATH. Keep the
        // response in a separate file to avoid cmd.exe interpreting JSON metacharacters.
        fs_err::write(path.with_extension("json"), response)?;
        fs_err::write(path, "@echo off\r\ntype \"%~dpn0.json\"\r\n")?;
        Ok(())
    }

    fn run<F, R>(&self, closure: F) -> R
    where
        F: FnOnce() -> R,
    {
        let path = env::join_paths(&self.search_path).expect("Test search paths are valid");
        let mut vars: Vec<(&str, Option<&OsStr>)> = EnvVars::all_names()
            .iter()
            .copied()
            .map(|name| (name, None))
            .collect();
        vars.extend([
            (EnvVars::UV_PYTHON_NO_REGISTRY, Some(OsStr::new("1"))),
            (EnvVars::PATH, Some(path.as_os_str())),
            (
                EnvVars::UV_PYTHON_INSTALL_DIR,
                Some(self.tempdir.path().as_os_str()),
            ),
            (EnvVars::PWD, Some(self.tempdir.path().as_os_str())),
        ]);
        with_vars(&vars, closure)
    }
}

#[test]
fn find_python_batch_file() -> Result<()> {
    let mut context = TestContext::new()?;
    let expected = context.add_python("Python with spaces", "3.12.1")?;

    let python = context.run(|| {
        find_python_installation(
            &PythonRequest::Default,
            EnvironmentPreference::OnlySystem,
            PythonPreference::OnlySystem,
            &context.cache,
        )
    })??;
    assert_eq!(python.source(), &PythonSource::SearchPathFirst);
    assert_eq!(python.interpreter().sys_executable(), expected);
    assert_eq!(python.interpreter().platform().os(), &Os::Windows);
    assert_eq!(
        python.interpreter().python_full_version().to_string(),
        "3.12.1"
    );
    Ok(())
}

#[test]
fn find_python_batch_file_version_fallback() -> Result<()> {
    let mut context = TestContext::new()?;
    context.add_python("older", "3.11.9")?;
    let expected = context.add_python("matching", "3.12.1")?;

    let python = context.run(|| {
        find_python_installation(
            &PythonRequest::parse("3.12"),
            EnvironmentPreference::OnlySystem,
            PythonPreference::OnlySystem,
            &context.cache,
        )
    })??;
    assert_eq!(python.interpreter().sys_executable(), expected);

    let missing = context.run(|| {
        find_python_installation(
            &PythonRequest::parse("3.13"),
            EnvironmentPreference::OnlySystem,
            PythonPreference::OnlySystem,
            &context.cache,
        )
    })?;
    assert_matches!(missing, Err(PythonNotFound { .. }));
    Ok(())
}

#[test]
fn find_python_batch_file_after_invalid_response() -> Result<()> {
    let mut context = TestContext::new()?;
    let invalid = context.add_search_path("invalid")?.join("python.bat");
    TestContext::write_response(&invalid, "not interpreter metadata")?;
    let expected = context.add_python("valid", "3.12.1")?;

    let python = context.run(|| {
        find_python_installation(
            &PythonRequest::Default,
            EnvironmentPreference::OnlySystem,
            PythonPreference::OnlySystem,
            &context.cache,
        )
    })??;
    assert_eq!(python.interpreter().sys_executable(), expected);
    Ok(())
}
