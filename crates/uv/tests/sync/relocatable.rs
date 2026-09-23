use std::path::Path;

use anyhow::{Context, Result};
use assert_fs::{fixture::ChildPath, prelude::*};
use indoc::indoc;
use insta::allow_duplicates;

use uv_cache::CacheBucket;
use uv_fs::{Simplified, normalize_path};
use uv_static::EnvVars;
use uv_test::{TestContext, site_packages_path, uv_snapshot};

/// Editable source paths are relative to the selected environment's site-packages directory.
#[test]
fn editable_preview_nested_environment() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let project = context.temp_dir.child("project");
    project.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [build-system]
        requires = ["uv-build"]
        build-backend = "uv_build"
    "#})?;
    project
        .child("src/project/__init__.py")
        .write_str("VALUE = 'hello'\n")?;

    uv_snapshot!(context.filters(), context.sync()
        .current_dir(project.path())
        .env(EnvVars::UV_PROJECT_ENVIRONMENT, "envs/deep/.venv")
        .args(["--offline", "--preview-features", "relocatable-envs-default"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: envs/deep/.venv
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + project==0.1.0 (from file://[TEMP_DIR]/project)
    ");

    let site_packages = site_packages_path(&project.join("envs/deep/.venv"), "python3.12");
    let source = project.join("src").simple_canonicalize()?;
    let contents = fs_err::read_to_string(site_packages.join("project.pth"))?;
    assert_relative_source(&site_packages, contents.trim_end(), &source)?;
    assert_pth_record(&context, &site_packages, 1);

    let relocated = context.temp_dir.child("relocated");
    fs_err::rename(project.path(), relocated.path())?;
    relocated
        .child("src/project/__init__.py")
        .write_str("VALUE = 'edited after moving'\n")?;

    uv_snapshot!(context.filters(), context.run()
        .current_dir(relocated.path())
        .env_remove(EnvVars::VIRTUAL_ENV)
        .env(EnvVars::UV_PROJECT_ENVIRONMENT, "envs/deep/.venv")
        .args(["--no-sync", "python", "-I", "-c", "import project; print(project.VALUE)"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    edited after moving
    ");

    Ok(())
}

/// Rewriting an installed editable must not change a hardlinked wheel in the shared cache.
#[test]
fn editable_shared_cache_hardlink() -> Result<()> {
    editable_shared_cache("hardlink")
}

/// Rewriting an installed editable must not follow a symlink into the shared cache.
#[cfg(unix)]
#[test]
fn editable_shared_cache_symlink() -> Result<()> {
    editable_shared_cache("symlink")
}

fn editable_shared_cache(link_mode: &str) -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let project = context.temp_dir.child("project");
    write_editable_backend(&project)?;

    // Populate the archive through an ordinary installation, which retains the backend's bytes.
    uv_snapshot!(context.filters(), context.sync()
        .current_dir(project.path())
        .env(EnvVars::UV_PROJECT_ENVIRONMENT, "ordinary")
        .args(["--offline", "--link-mode", link_mode]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: ordinary
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + project==0.1.0 (from file://[TEMP_DIR]/project)
    ");

    let archive_files = context.cache_files(CacheBucket::Archive)?;
    let archived_pth = archive_files
        .iter()
        .find(|path| {
            path.ends_with("project.pth")
                && path
                    .parent()
                    .is_some_and(|parent| parent.join("project-0.1.0.dist-info").is_dir())
        })
        .context("editable wheel is missing from the archive")?;
    let archive = archived_pth
        .parent()
        .context("editable wheel has no archive directory")?;
    let original = [
        ("project.pth", "project.pth"),
        ("paths.pth", "paths.pth"),
        ("executable.pth", "executable.pth"),
        (".invalid.pth", ".invalid.pth"),
        ("project-0.1.0.data/purelib/project.pth", "project.pth"),
        ("project-0.1.0.data/purelib/purelib.pth", "purelib.pth"),
        ("project-0.1.0.data/platlib/platlib.pth", "platlib.pth"),
    ]
    .into_iter()
    .map(|(archive_name, installed_name)| {
        Ok((
            archive_name,
            installed_name,
            fs_err::read(archive.join(archive_name))?,
        ))
    })
    .collect::<Result<Vec<_>>>()?;
    let ordinary_site_packages = site_packages_path(&project.join("ordinary"), "python3.12");
    for (_, installed_name, contents) in &original {
        assert_eq!(
            fs_err::read(ordinary_site_packages.join(installed_name))?,
            *contents
        );
    }
    assert_pth_record(&context, &ordinary_site_packages, 2);

    uv_snapshot!(context.filters(), context.sync()
        .current_dir(project.path())
        .env(EnvVars::UV_PROJECT_ENVIRONMENT, "envs/deep/.venv")
        .args(["--offline", "--link-mode", link_mode, "--preview-features", "relocatable-envs-default"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: envs/deep/.venv
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + project==0.1.0 (from file://[TEMP_DIR]/project)
    ");

    let site_packages = site_packages_path(&project.join("envs/deep/.venv"), "python3.12");
    let source = project.path().simple_canonicalize()?;
    let project_pth = fs_err::read_to_string(site_packages.join("project.pth"))?;
    assert_relative_source(&site_packages, project_pth.trim_end(), &source.join("src"))?;
    for name in ["purelib.pth", "platlib.pth"] {
        let contents = fs_err::read_to_string(site_packages.join(name))?;
        assert_relative_source(&site_packages, contents.trim_end(), &source.join("src"))?;
    }

    let paths_pth = fs_err::read_to_string(site_packages.join("paths.pth"))?;
    let paths = paths_pth.lines().collect::<Vec<_>>();
    assert_eq!(paths.len(), 6);
    assert_eq!(paths[0], "# Editable source paths");
    assert_eq!(paths[1], "");
    assert_relative_source(&site_packages, paths[2], &source.join("src"))?;
    assert_relative_source(&site_packages, paths[3], &source.join("generated"))?;
    let original_paths = fs_err::read_to_string(archive.join("paths.pth"))?;
    assert_eq!(
        paths[4],
        original_paths
            .lines()
            .nth(4)
            .context("outside-source path is missing")?
    );
    assert_eq!(paths[5], "relative-entry");

    for (archive_name, installed_name, contents) in &original {
        assert_eq!(fs_err::read(archive.join(archive_name))?, *contents);
        if *installed_name == "executable.pth" || *installed_name == ".invalid.pth" {
            assert_eq!(fs_err::read(site_packages.join(installed_name))?, *contents);
        }
    }
    // Wheel-root and `.data/purelib` entries can leave duplicate rows for the same installed file.
    assert_pth_record(&context, &site_packages, 2);

    // Another ordinary environment must be able to reuse the same unmodified editable wheel.
    uv_snapshot!(context.filters(), context.sync()
        .current_dir(project.path())
        .env(EnvVars::UV_PROJECT_ENVIRONMENT, "other")
        .args(["--offline", "--link-mode", link_mode]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: other
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + project==0.1.0 (from file://[TEMP_DIR]/project)
    ");
    let other_site_packages = site_packages_path(&project.join("other"), "python3.12");
    for (_, installed_name, contents) in &original {
        assert_eq!(
            fs_err::read(other_site_packages.join(installed_name))?,
            *contents
        );
    }

    uv_snapshot!(context.filters(), context.run()
        .current_dir(project.path())
        .env_remove(EnvVars::VIRTUAL_ENV)
        .env(EnvVars::UV_PROJECT_ENVIRONMENT, "other")
        .args(["--no-sync", "python", "-I", "-c", "import project; print(project.VALUE)"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    hello
    ");

    Ok(())
}

fn assert_relative_source(site_packages: &Path, entry: &str, source: &Path) -> Result<()> {
    let entry = Path::new(entry);
    assert!(!entry.is_absolute());
    assert_eq!(
        normalize_path(site_packages.simple_canonicalize()?.join(entry)),
        normalize_path(source),
    );
    Ok(())
}

fn assert_pth_record(context: &TestContext, site_packages: &Path, project_entries: usize) {
    let check = indoc! {r#"
        import base64
        import csv
        import hashlib
        import pathlib
        import sys

        site_packages = pathlib.Path(sys.argv[1])
        record = site_packages / "project-0.1.0.dist-info" / "RECORD"
        project_entries = 0
        with record.open(encoding="utf-8", newline="") as stream:
            for name, digest, size in csv.reader(stream):
                if not name.endswith(".pth"):
                    continue
                if name == "project.pth":
                    project_entries += 1
                contents = (site_packages / name).read_bytes()
                expected = base64.urlsafe_b64encode(hashlib.sha256(contents).digest()).rstrip(b"=").decode()
                assert digest == f"sha256={expected}", (name, digest, expected)
                assert int(size) == len(contents), (name, size, len(contents))
        assert project_entries == int(sys.argv[2]), project_entries
        print("RECORD matches")
    "#};
    allow_duplicates! {
        uv_snapshot!(context.filters(), context.python_command()
            .args(["-I", "-S", "-c", check])
            .arg(site_packages)
            .arg(project_entries.to_string()), @"
        exit_code: 0 (success)
        ----- stdout -----
        RECORD matches
        ");
    }
}

fn write_editable_backend(project: &ChildPath) -> Result<()> {
    project.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [build-system]
        requires = []
        backend-path = ["."]
        build-backend = "backend"
    "#})?;
    project
        .child("src/project/__init__.py")
        .write_str("VALUE = 'hello'\n")?;
    project.child("generated").create_dir_all()?;
    project.child("backend.py").write_str(indoc! {r##"
        import base64
        import csv
        import hashlib
        import io
        import pathlib
        import zipfile

        def build_editable(wheel_directory, config_settings=None, metadata_directory=None):
            root = pathlib.Path(__file__).resolve().parent
            source = root / "src"
            files = {
                "project.pth": f"{source}\n".encode(),
                "paths.pth": (
                    "# Editable source paths\n\n"
                    f"{source}\n{root / 'generated'}\n{root.parent / 'outside'}\n"
                    "relative-entry\n"
                ).encode(),
                "executable.pth": f"{source}\nimport sys\n".encode(),
                ".invalid.pth": f"{source}\n".encode() + b"\xff\n",
                "project-0.1.0.data/purelib/project.pth": f"{source}\n".encode(),
                "project-0.1.0.data/purelib/purelib.pth": f"{source}\n".encode(),
                "project-0.1.0.data/platlib/platlib.pth": f"{source}\n".encode(),
                "project-0.1.0.dist-info/METADATA": (
                    "Metadata-Version: 2.1\nName: project\nVersion: 0.1.0\n"
                ).encode(),
                "project-0.1.0.dist-info/WHEEL": (
                    "Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n"
                ).encode(),
            }
            record = io.StringIO(newline="")
            writer = csv.writer(record, lineterminator="\n")
            for name, contents in files.items():
                digest = base64.urlsafe_b64encode(hashlib.sha256(contents).digest()).rstrip(b"=").decode()
                writer.writerow((name, f"sha256={digest}", len(contents)))
            writer.writerow(("project-0.1.0.dist-info/RECORD", "", ""))
            files["project-0.1.0.dist-info/RECORD"] = record.getvalue().encode()
            filename = "project-0.1.0-py3-none-any.whl"
            with zipfile.ZipFile(pathlib.Path(wheel_directory) / filename, "w") as wheel:
                for name, contents in files.items():
                    wheel.writestr(name, contents)
            return filename
    "##})?;
    Ok(())
}
