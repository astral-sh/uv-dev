use std::path::Path;

use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::fixture::{FileWriteStr, PathChild, PathCreateDir};
use async_zip::base::write::ZipFileWriter;
use async_zip::{Compression, ZipEntryBuilder};
use futures::executor::block_on;
use indoc::{formatdoc, indoc};
use url::Url;

use uv_static::EnvVars;
use uv_test::{copy_dir_ignore, uv_snapshot};

fn write_wheel(
    path: &Path,
    name: &str,
    dist_info_prefix: &str,
    files: &[(&str, &str)],
) -> Result<()> {
    write_wheel_with_metadata(path, name, "0.1.0", dist_info_prefix, "", files)
}

fn write_wheel_with_metadata(
    path: &Path,
    name: &str,
    version: &str,
    dist_info_prefix: &str,
    additional_metadata: &str,
    files: &[(&str, &str)],
) -> Result<()> {
    let mut writer = ZipFileWriter::new(Vec::new());
    let mut record = Vec::new();

    for (file_path, contents) in files {
        let entry = ZipEntryBuilder::new((*file_path).into(), Compression::Stored);
        block_on(writer.write_entry_whole(entry, contents.as_bytes()))?;
        record.push(format!("{file_path},,"));
    }

    let metadata_path = format!("{dist_info_prefix}.dist-info/METADATA");
    let entry = ZipEntryBuilder::new(metadata_path.clone().into(), Compression::Stored);
    block_on(
        writer.write_entry_whole(
            entry,
            format!(
                "Metadata-Version: 2.1\nName: {name}\nVersion: {version}\n{additional_metadata}"
            )
            .as_bytes(),
        ),
    )?;
    record.push(format!("{metadata_path},,"));

    let wheel_path = format!("{dist_info_prefix}.dist-info/WHEEL");
    let entry = ZipEntryBuilder::new(wheel_path.clone().into(), Compression::Stored);
    block_on(writer.write_entry_whole(
        entry,
        b"Wheel-Version: 1.0\nGenerator: uv-test\nRoot-Is-Purelib: true\nTag: py3-none-any\n",
    ))?;
    record.push(format!("{wheel_path},,"));

    let record_path = format!("{dist_info_prefix}.dist-info/RECORD");
    record.push(format!("{record_path},,"));
    let entry = ZipEntryBuilder::new(record_path.into(), Compression::Stored);
    let record = format!("{}\n", record.join("\n"));
    block_on(writer.write_entry_whole(entry, record.as_bytes()))?;

    fs_err::write(path, block_on(writer.close())?)?;
    Ok(())
}

/// Test basic metadata output for a simple workspace with one member.
#[test]
fn workspace_metadata_simple() {
    let context = uv_test::test_context!("3.12");

    // Initialize a workspace with one member
    context.init().arg("foo").assert().success();

    let workspace = context.temp_dir.child("foo");

    uv_snapshot!(context.filters(), context.workspace_metadata().current_dir(&workspace), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "workspace_root": "[TEMP_DIR]/foo",
      "workspace": {
        "path": "[TEMP_DIR]/foo",
        "id": "workspace+[TEMP_DIR]/foo"
      },
      "requires_python": ">=3.12",
      "conflicts": {
        "sets": []
      },
      "members": [
        {
          "name": "foo",
          "path": "[TEMP_DIR]/foo",
          "id": "foo==0.1.0@editable+[TEMP_DIR]/foo/"
        }
      ],
      "resolution": {
        "foo==0.1.0@editable+[TEMP_DIR]/foo/": {
          "name": "foo",
          "version": "0.1.0",
          "source": {
            "editable": "[TEMP_DIR]/foo/"
          },
          "kind": "package",
          "dependencies": []
        },
        "workspace+[TEMP_DIR]/foo": {
          "kind": "workspace",
          "path": "[TEMP_DIR]/foo",
          "dependencies": []
        }
      }
    }

    ----- stderr -----
    warning: The `uv workspace metadata` command is experimental and may change without warning. Pass `--preview-features workspace-metadata` to disable this warning.
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Resolved 1 package in [TIME]
    "#
    );

    assert!(!workspace.child(".venv").exists());
}

#[test]
fn workspace_metadata_quiet() {
    let context = uv_test::test_context!("3.12");
    context.init().arg("foo").assert().success();

    let workspace = context.temp_dir.child("foo");

    uv_snapshot!(context.filters(), context.workspace_metadata().current_dir(&workspace).arg("--quiet"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "workspace_root": "[TEMP_DIR]/foo",
      "workspace": {
        "path": "[TEMP_DIR]/foo",
        "id": "workspace+[TEMP_DIR]/foo"
      },
      "requires_python": ">=3.12",
      "conflicts": {
        "sets": []
      },
      "members": [
        {
          "name": "foo",
          "path": "[TEMP_DIR]/foo",
          "id": "foo==0.1.0@editable+[TEMP_DIR]/foo/"
        }
      ],
      "resolution": {
        "foo==0.1.0@editable+[TEMP_DIR]/foo/": {
          "name": "foo",
          "version": "0.1.0",
          "source": {
            "editable": "[TEMP_DIR]/foo/"
          },
          "kind": "package",
          "dependencies": []
        },
        "workspace+[TEMP_DIR]/foo": {
          "kind": "workspace",
          "path": "[TEMP_DIR]/foo",
          "dependencies": []
        }
      }
    }
    "#);
}

#[test]
fn workspace_metadata_extra_quiet() {
    let context = uv_test::test_context!("3.12");
    context.init().arg("foo").assert().success();

    let workspace = context.temp_dir.child("foo");

    uv_snapshot!(context.filters(), context.workspace_metadata().current_dir(&workspace).arg("--quiet").arg("--quiet"), @r"
    exit_code: 0 (success)
    ");
}

#[test]
fn workspace_metadata_ignores_unusable_environment() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context.init().arg("foo").assert().success();

    let workspace = context.temp_dir.child("foo");
    let environment = workspace.child(".venv");
    environment.create_dir_all()?;

    let empty_output = context
        .workspace_metadata()
        .current_dir(&workspace)
        .assert()
        .success();
    let empty_metadata: serde_json::Value =
        serde_json::from_slice(&empty_output.get_output().stdout)?;

    environment
        .child("pyvenv.cfg")
        .write_str("home = /missing-python\n")?;

    let broken_output = context
        .workspace_metadata()
        .current_dir(&workspace)
        .assert()
        .success();
    let broken_metadata: serde_json::Value =
        serde_json::from_slice(&broken_output.get_output().stdout)?;

    insta::assert_json_snapshot!(serde_json::json!({
        "broken_environment": broken_metadata.get("environment"),
        "empty_environment": empty_metadata.get("environment"),
    }), @r#"
    {
      "broken_environment": null,
      "empty_environment": null
    }
    "#);

    Ok(())
}

#[test]
fn workspace_metadata_script() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_python_names()
        .with_filtered_virtualenv_bin();
    let script = context.temp_dir.child("script.py");
    script.write_str(
        r#"# /// script
# requires-python = ">=3.12"
# dependencies = ["iniconfig"]
# ///

import iniconfig
"#,
    )?;

    uv_snapshot!(
        context.filters(),
        context
            .workspace_metadata()
            .arg("--script")
            .arg(script.path())
            .arg("--sync"),
        @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "workspace_root": "[TEMP_DIR]/",
      "environment": {
        "root": "[CACHE_DIR]/environments-v2/script-[HASH]",
        "python": {
          "path": "[CACHE_DIR]/environments-v2/script-[HASH]/[BIN]/[PYTHON]",
          "version": "3.12.[X]",
          "implementation": "cpython"
        }
      },
      "script": {
        "path": "[TEMP_DIR]/script.py",
        "id": "script+[TEMP_DIR]/script.py"
      },
      "requires_python": ">=3.12",
      "conflicts": {
        "sets": []
      },
      "module_owners": {
        "iniconfig": [
          {
            "package_id": "iniconfig==2.0.0@registry+http://[LOCALHOST]/simple/"
          }
        ]
      },
      "resolution": {
        "iniconfig==2.0.0@registry+http://[LOCALHOST]/simple/": {
          "name": "iniconfig",
          "version": "2.0.0",
          "source": {
            "registry": {
              "url": "http://[LOCALHOST]/simple/"
            }
          },
          "kind": "package",
          "dependencies": [],
          "sdist": {
            "url": "http://[LOCALHOST]/files/iniconfig-2.0.0.tar.gz",
            "hashes": {
              "sha256": "48c42a08c0ec1a24f2fe45f4efdefc9c19ac8e0aa8e82284503ccba80398bec3"
            },
            "upload_time": "2024-03-24T00:00:00Z"
          },
          "wheels": [
            {
              "url": "http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl",
              "hashes": {
                "sha256": "8a0fc44e516906bdecc91af1c3bc12134c9d1647a482446edc62f2f72191416c"
              },
              "upload_time": "2024-03-24T00:00:00Z",
              "filename": "iniconfig-2.0.0-py3-none-any.whl"
            }
          ]
        },
        "script+[TEMP_DIR]/script.py": {
          "kind": "script",
          "path": "[TEMP_DIR]/script.py",
          "dependencies": [
            {
              "id": "iniconfig==2.0.0@registry+http://[LOCALHOST]/simple/"
            }
          ]
        }
      }
    }

    ----- stderr -----
    warning: The `uv workspace metadata` command is experimental and may change without warning. Pass `--preview-features workspace-metadata` to disable this warning.
    Resolved 1 package in [TIME]
    "#
    );

    assert!(!context.temp_dir.child("script.py.lock").exists());

    Ok(())
}

#[test]
fn workspace_metadata_script_no_dependencies() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let script = context.temp_dir.child("script.py");
    script.write_str(
        r#"# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///

print("Hello, world!")
"#,
    )?;

    uv_snapshot!(
        context.filters(),
        context
            .workspace_metadata()
            .arg("--script")
            .arg(script.path()),
        @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "workspace_root": "[TEMP_DIR]/",
      "script": {
        "path": "[TEMP_DIR]/script.py",
        "id": "script+[TEMP_DIR]/script.py"
      },
      "requires_python": ">=3.12",
      "conflicts": {
        "sets": []
      },
      "resolution": {
        "script+[TEMP_DIR]/script.py": {
          "kind": "script",
          "path": "[TEMP_DIR]/script.py",
          "dependencies": []
        }
      }
    }

    ----- stderr -----
    warning: The `uv workspace metadata` command is experimental and may change without warning. Pass `--preview-features workspace-metadata` to disable this warning.
    Resolved in [TIME]
    "#
    );

    assert!(!context.temp_dir.child("script.py.lock").exists());

    Ok(())
}

#[test]
fn workspace_metadata_script_includes_existing_environment() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_python_names()
        .with_filtered_virtualenv_bin();
    let script = context.temp_dir.child("script.py");
    script.write_str(
        r#"# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///
"#,
    )?;

    context
        .workspace_metadata()
        .arg("--script")
        .arg(script.path())
        .arg("--sync")
        .assert()
        .success();

    let assert = context
        .workspace_metadata()
        .arg("--script")
        .arg(script.path())
        .assert()
        .success();
    let metadata: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout)?;

    insta::with_settings!({ filters => context.filters() }, {
        insta::assert_json_snapshot!(metadata["environment"], @r#"
        {
          "python": {
            "implementation": "cpython",
            "path": "[CACHE_DIR]/environments-v2/script-[HASH]/[BIN]/[PYTHON]",
            "version": "3.12.[X]"
          },
          "root": "[CACHE_DIR]/environments-v2/script-[HASH]"
        }
        "#);
    });

    Ok(())
}

#[test]
fn workspace_metadata_script_exact_sync_removes_extraneous_packages() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let script = context.temp_dir.child("script.py");
    script.write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = []
        # ///
        "#
    })?;

    let extraneous = context
        .temp_dir
        .child("metadata_extra-0.1.0-py3-none-any.whl");
    write_wheel(
        extraneous.path(),
        "metadata-extra",
        "metadata_extra-0.1.0",
        &[("extra_module.py", "")],
    )?;

    context
        .pip_install()
        .arg(extraneous.path())
        .assert()
        .success();

    context
        .workspace_metadata()
        .arg("--script")
        .arg(script.path())
        .arg("--sync")
        .arg("--active")
        .env(EnvVars::VIRTUAL_ENV, context.venv.path())
        .assert()
        .success();
    context.pip_show().arg("metadata-extra").assert().success();

    let assert = context
        .workspace_metadata()
        .arg("--script")
        .arg(script.path())
        .arg("--sync")
        .arg("--exact")
        .arg("--active")
        .env(EnvVars::VIRTUAL_ENV, context.venv.path())
        .assert()
        .success();
    let metadata: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout)?;

    insta::assert_json_snapshot!(serde_json::json!({
        "extraneous_installed": context
            .pip_show()
            .arg("metadata-extra")
            .output()?
            .status
            .success(),
        "module_owners": metadata.get("module_owners"),
    }), @r#"
    {
      "extraneous_installed": false,
      "module_owners": null
    }
    "#);

    Ok(())
}

#[test]
fn workspace_metadata_script_dependency_edges() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let child = context
        .temp_dir
        .child("metadata_edge_child-0.1.0-py3-none-any.whl");
    write_wheel(
        child.path(),
        "metadata-edge-child",
        "metadata_edge_child-0.1.0",
        &[],
    )?;
    let child_url = Url::from_file_path(child.path())
        .map_err(|()| anyhow::anyhow!("failed to convert wheel path to file URL"))?;

    let first = context
        .temp_dir
        .child("metadata_edge-1.0.0-py3-none-any.whl");
    write_wheel_with_metadata(
        first.path(),
        "metadata-edge",
        "1.0.0",
        "metadata_edge-1.0.0",
        &format!(
            "Provides-Extra: feature\nRequires-Dist: metadata-edge-child @ {child_url}; extra == 'feature'\n"
        ),
        &[],
    )?;
    let first_url = Url::from_file_path(first.path())
        .map_err(|()| anyhow::anyhow!("failed to convert wheel path to file URL"))?;

    let second = context
        .temp_dir
        .child("metadata_edge-2.0.0-py3-none-any.whl");
    write_wheel_with_metadata(
        second.path(),
        "metadata-edge",
        "2.0.0",
        "metadata_edge-2.0.0",
        &format!(
            "Provides-Extra: feature\nRequires-Dist: metadata-edge-child @ {child_url}; extra == 'feature'\n"
        ),
        &[],
    )?;
    let second_url = Url::from_file_path(second.path())
        .map_err(|()| anyhow::anyhow!("failed to convert wheel path to file URL"))?;

    let script = context.temp_dir.child("script.py");
    script.write_str(&format!(
        r#"# /// script
# requires-python = ">=3.12"
# dependencies = [
#   "metadata-edge[feature] @ {first_url}; sys_platform == 'win32'",
#   "metadata-edge[feature] @ {second_url}; sys_platform != 'win32'",
# ]
# ///
"#
    ))?;

    let assert = context
        .workspace_metadata()
        .arg("--script")
        .arg(script.path())
        .assert()
        .success();
    let metadata: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout)?;

    let resolution = metadata["resolution"]
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("metadata resolution was not an object"))?;
    let script_id = metadata["script"]["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("script ID was not a string"))?;
    let script_node = resolution
        .get(script_id)
        .ok_or_else(|| anyhow::anyhow!("missing resolution node for {script_id}"))?;

    insta::with_settings!({ filters => context.filters() }, {
        insta::assert_json_snapshot!(serde_json::json!({
            "script": metadata["script"],
            "node": script_node,
        }), @r#"
        {
          "node": {
            "dependencies": [
              {
                "id": "metadata-edge[feature]==2.0.0@path+[TEMP_DIR]/metadata_edge-2.0.0-py3-none-any.whl",
                "marker": "sys_platform != 'win32'"
              },
              {
                "id": "metadata-edge[feature]==1.0.0@path+[TEMP_DIR]/metadata_edge-1.0.0-py3-none-any.whl",
                "marker": "sys_platform == 'win32'"
              }
            ],
            "kind": "script",
            "path": "[TEMP_DIR]/script.py"
          },
          "script": {
            "id": "script+[TEMP_DIR]/script.py",
            "path": "[TEMP_DIR]/script.py"
          }
        }
        "#);
    });

    for dependency in script_node["dependencies"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("script dependencies was not an array"))?
    {
        let id = dependency["id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("script dependency ID was not a string"))?;
        anyhow::ensure!(
            resolution.contains_key(id),
            "missing resolution node for {id}"
        );
    }

    Ok(())
}

#[test]
fn workspace_metadata_dependency_edges_include_parent_reachability() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let child = context
        .temp_dir
        .child("metadata_child-0.1.0-py3-none-any.whl");
    write_wheel(child.path(), "metadata-child", "metadata_child-0.1.0", &[])?;
    let child_url = Url::from_file_path(child.path())
        .map_err(|()| anyhow::anyhow!("failed to convert wheel path to file URL"))?;

    let parent = context
        .temp_dir
        .child("metadata_parent-0.1.0-py3-none-any.whl");
    write_wheel_with_metadata(
        parent.path(),
        "metadata-parent",
        "0.1.0",
        "metadata_parent-0.1.0",
        &format!("Requires-Dist: metadata-child @ {child_url}\n"),
        &[],
    )?;
    let parent_url = Url::from_file_path(parent.path())
        .map_err(|()| anyhow::anyhow!("failed to convert wheel path to file URL"))?;

    context.init().arg("project").assert().success();
    let project = context.temp_dir.child("project");
    project.child("pyproject.toml").write_str(&format!(
        r#"[project]
name = "project"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = [
    "metadata-parent @ {parent_url} ; sys_platform == 'linux'",
]
"#
    ))?;

    let assert = context
        .workspace_metadata()
        .current_dir(&project)
        .assert()
        .success();
    let metadata: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout)?;
    let resolution = metadata["resolution"]
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("metadata resolution was not an object"))?;
    let parent_node = resolution
        .iter()
        .find_map(|(id, node)| id.starts_with("metadata-parent==").then_some(node))
        .ok_or_else(|| anyhow::anyhow!("missing metadata-parent resolution node"))?;

    insta::with_settings!({ filters => context.filters() }, {
        insta::assert_json_snapshot!(parent_node["dependencies"], @r#"
        [
          {
            "id": "metadata-child==0.1.0@path+[TEMP_DIR]/metadata_child-0.1.0-py3-none-any.whl",
            "marker": "sys_platform == 'linux'"
          }
        ]
        "#);
    });

    Ok(())
}

#[test]
fn workspace_metadata_sync_centralized_environment() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);

    context.temp_dir.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
        "#,
    )?;

    let assert = context
        .workspace_metadata()
        .arg("--sync")
        .arg("--preview-features")
        .arg("workspace-metadata,centralized-project-envs")
        .assert()
        .success();
    let metadata: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout)?;
    let target = fs_err::read_link(context.temp_dir.child(".venv").path())?;

    assert_eq!(
        metadata["environment"]["root"].as_str().map(Path::new),
        Some(target.as_path())
    );
    assert_eq!(
        target.parent(),
        Some(context.cache_dir.child("environments-v2").path())
    );

    let assert = context
        .workspace_metadata()
        .arg("--preview-features")
        .arg("workspace-metadata,centralized-project-envs")
        .assert()
        .success();
    let metadata: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout)?;

    assert_eq!(
        metadata["environment"]["root"].as_str().map(Path::new),
        Some(target.as_path())
    );

    Ok(())
}

#[test]
fn workspace_metadata_sync_active_environment() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.11"]);

    context.temp_dir.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
        "#,
    )?;

    context
        .venv()
        .arg("--python")
        .arg("3.11")
        .assert()
        .success();
    let active = context.temp_dir.child("active");
    context
        .venv()
        .arg(active.path())
        .arg("--python")
        .arg("3.12")
        .assert()
        .success();

    let assert = context
        .workspace_metadata()
        .arg("--sync")
        .arg("--active")
        .env(EnvVars::VIRTUAL_ENV, active.path())
        .assert()
        .success();
    let metadata: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout)?;

    assert_eq!(
        metadata["environment"]["root"].as_str().map(Path::new),
        Some(active.path())
    );

    let assert = context
        .workspace_metadata()
        .arg("--active")
        .env(EnvVars::VIRTUAL_ENV, active.path())
        .assert()
        .success();
    let metadata: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout)?;

    assert_eq!(
        metadata["environment"]["root"].as_str().map(Path::new),
        Some(active.path())
    );

    Ok(())
}

#[test]
fn workspace_metadata_exact_requires_sync() {
    let context = uv_test::test_context!("3.12");

    uv_snapshot!(context.filters(), context.workspace_metadata().arg("--exact"), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the following required arguments were not provided:
      --sync

    Usage: uv workspace metadata --sync --cache-dir [CACHE_DIR] --exact --exclude-newer <EXCLUDE_NEWER>

    For more information, try '--help'.
    ");
}

#[test]
fn workspace_metadata_exact_sync_removes_extraneous_packages() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let required = context
        .temp_dir
        .child("metadata_required-0.1.0-py3-none-any.whl");
    write_wheel(
        required.path(),
        "metadata-required",
        "metadata_required-0.1.0",
        &[("required_module.py", "")],
    )?;
    let required_url = Url::from_file_path(required.path())
        .map_err(|()| anyhow::anyhow!("failed to convert wheel path to file URL"))?;

    let extraneous = context
        .temp_dir
        .child("metadata_extra-0.1.0-py3-none-any.whl");
    write_wheel(
        extraneous.path(),
        "metadata-extra",
        "metadata_extra-0.1.0",
        &[("extra_module.py", "")],
    )?;

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
            [project]
            name = "module-owner-root"
            version = "0.1.0"
            requires-python = ">=3.12"
            dependencies = ["metadata-required @ {required_url}"]
            "#
        })?;

    context
        .pip_install()
        .arg(extraneous.path())
        .assert()
        .success();

    let assert = context
        .workspace_metadata()
        .arg("--sync")
        .assert()
        .success();
    let metadata: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout)?;
    let extraneous_installed = context
        .pip_show()
        .arg("metadata-extra")
        .output()?
        .status
        .success();
    let required_installed = context
        .pip_show()
        .arg("metadata-required")
        .output()?
        .status
        .success();

    insta::with_settings!({ filters => context.filters() }, {
        insta::assert_json_snapshot!(serde_json::json!({
            "extraneous_installed": extraneous_installed,
            "module_owners": metadata["module_owners"],
            "required_installed": required_installed,
        }), @r#"
        {
          "extraneous_installed": true,
          "module_owners": {
            "required_module": [
              {
                "package_id": "metadata-required==0.1.0@path+[TEMP_DIR]/metadata_required-0.1.0-py3-none-any.whl"
              }
            ]
          },
          "required_installed": true
        }
        "#);
    });

    let assert = context
        .workspace_metadata()
        .arg("--sync")
        .arg("--exact")
        .assert()
        .success();
    let metadata: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout)?;
    let extraneous_installed = context
        .pip_show()
        .arg("metadata-extra")
        .output()?
        .status
        .success();
    let required_installed = context
        .pip_show()
        .arg("metadata-required")
        .output()?
        .status
        .success();

    insta::with_settings!({ filters => context.filters() }, {
        insta::assert_json_snapshot!(serde_json::json!({
            "extraneous_installed": extraneous_installed,
            "module_owners": metadata["module_owners"],
            "required_installed": required_installed,
        }), @r#"
        {
          "extraneous_installed": false,
          "module_owners": {
            "required_module": [
              {
                "package_id": "metadata-required==0.1.0@path+[TEMP_DIR]/metadata_required-0.1.0-py3-none-any.whl"
              }
            ]
          },
          "required_installed": true
        }
        "#);
    });

    Ok(())
}

#[test]
fn workspace_metadata_includes_existing_environment() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_python_names()
        .with_filtered_virtualenv_bin();

    let installed_owner = context
        .temp_dir
        .child("installed_owner-0.1.0-py3-none-any.whl");
    write_wheel(
        installed_owner.path(),
        "installed-owner",
        "installed_owner-0.1.0",
        &[("installed_module.py", "")],
    )?;

    let missing_owner = context
        .temp_dir
        .child("missing_owner-0.1.0-py3-none-any.whl");
    write_wheel(
        missing_owner.path(),
        "missing-owner",
        "missing_owner-0.1.0",
        &[("missing_module.py", "")],
    )?;

    let installed_owner_url = Url::from_file_path(installed_owner.path())
        .map_err(|()| anyhow::anyhow!("failed to convert wheel path to file URL"))?;
    let missing_owner_url = Url::from_file_path(missing_owner.path())
        .map_err(|()| anyhow::anyhow!("failed to convert wheel path to file URL"))?;

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&format!(
            r#"[project]
name = "module-owner-root"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = [
  "installed-owner @ {installed_owner_url}",
  "missing-owner @ {missing_owner_url}",
]
"#
        ))?;

    context.lock().assert().success();
    context
        .pip_install()
        .arg(installed_owner.path())
        .assert()
        .success();

    // Removing the uninstalled wheel makes any accidental synchronization fail.
    fs_err::remove_file(missing_owner.path())?;

    let assert = context
        .workspace_metadata()
        .arg("--frozen")
        .assert()
        .success();
    let metadata: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout)?;

    insta::with_settings!({ filters => context.filters() }, {
        insta::assert_json_snapshot!(serde_json::json!({
            "environment": metadata["environment"],
            "module_owners": metadata["module_owners"],
        }), @r#"
        {
          "environment": {
            "python": {
              "implementation": "cpython",
              "path": "[VENV]/[BIN]/[PYTHON]",
              "version": "3.12.[X]"
            },
            "root": "[VENV]/"
          },
          "module_owners": {
            "installed_module": [
              {
                "package_id": "installed-owner==0.1.0@path+[TEMP_DIR]/installed_owner-0.1.0-py3-none-any.whl"
              }
            ]
          }
        }
        "#);
    });

    context.pip_show().arg("missing-owner").assert().failure();

    Ok(())
}

#[test]
fn workspace_metadata_module_owners_from_locked_wheels() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_python_names()
        .with_filtered_virtualenv_bin();

    let gpu_a = context.temp_dir.child("gpu_a-0.1.0-py3-none-any.whl");
    write_wheel(gpu_a.path(), "gpu-a", "gpu_a-0.1.0", &[("gpu/a.py", "")])?;

    let gpu_b = context.temp_dir.child("gpu_b-0.1.0-py3-none-any.whl");
    write_wheel(gpu_b.path(), "gpu-b", "gpu_b-0.1.0", &[("gpu/b.py", "")])?;

    let typing_extensions = context
        .temp_dir
        .child("typing_extensions-0.1.0-py3-none-any.whl");
    write_wheel(
        typing_extensions.path(),
        "typing-extensions",
        "typing_extensions-0.1.0",
        &[
            ("typing_extensions.py", ""),
            ("café.py", ""),
            ("bogus.pypynonsense.so", ""),
            ("bytecode/__pycache__/compiled.cpython-312.pyc", ""),
        ],
    )?;

    let gpu_a_url = Url::from_file_path(gpu_a.path())
        .map_err(|()| anyhow::anyhow!("failed to convert wheel path to file URL"))?;
    let gpu_b_url = Url::from_file_path(gpu_b.path())
        .map_err(|()| anyhow::anyhow!("failed to convert wheel path to file URL"))?;
    let typing_extensions_url = Url::from_file_path(typing_extensions.path())
        .map_err(|()| anyhow::anyhow!("failed to convert wheel path to file URL"))?;

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&format!(
            r#"[project]
name = "module-owner-root"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = [
  "gpu-a @ {gpu_a_url}",
  "gpu-b @ {gpu_b_url}",
  "typing-extensions @ {typing_extensions_url}",
]
"#
        ))?;

    let mut filters = context.filters();
    filters.push((r#""sha256": "[0-9a-f]{64}""#, r#""sha256": "[SHA256]""#));

    uv_snapshot!(filters, context.workspace_metadata().arg("--sync"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "workspace_root": "[TEMP_DIR]/",
      "environment": {
        "root": "[VENV]/",
        "python": {
          "path": "[VENV]/[BIN]/[PYTHON]",
          "version": "3.12.[X]",
          "implementation": "cpython"
        }
      },
      "workspace": {
        "path": "[TEMP_DIR]/",
        "id": "workspace+[TEMP_DIR]/"
      },
      "requires_python": ">=3.12",
      "conflicts": {
        "sets": []
      },
      "module_owners": {
        "café": [
          {
            "package_id": "typing-extensions==0.1.0@path+[TEMP_DIR]/typing_extensions-0.1.0-py3-none-any.whl"
          }
        ],
        "gpu": [
          {
            "package_id": "gpu-a==0.1.0@path+[TEMP_DIR]/gpu_a-0.1.0-py3-none-any.whl"
          },
          {
            "package_id": "gpu-b==0.1.0@path+[TEMP_DIR]/gpu_b-0.1.0-py3-none-any.whl"
          }
        ],
        "gpu.a": [
          {
            "package_id": "gpu-a==0.1.0@path+[TEMP_DIR]/gpu_a-0.1.0-py3-none-any.whl"
          }
        ],
        "gpu.b": [
          {
            "package_id": "gpu-b==0.1.0@path+[TEMP_DIR]/gpu_b-0.1.0-py3-none-any.whl"
          }
        ],
        "typing_extensions": [
          {
            "package_id": "typing-extensions==0.1.0@path+[TEMP_DIR]/typing_extensions-0.1.0-py3-none-any.whl"
          }
        ]
      },
      "members": [
        {
          "name": "module-owner-root",
          "path": "[TEMP_DIR]/",
          "id": "module-owner-root==0.1.0@virtual+[TEMP_DIR]/"
        }
      ],
      "resolution": {
        "gpu-a==0.1.0@path+[TEMP_DIR]/gpu_a-0.1.0-py3-none-any.whl": {
          "name": "gpu-a",
          "version": "0.1.0",
          "source": {
            "path": "[TEMP_DIR]/gpu_a-0.1.0-py3-none-any.whl"
          },
          "kind": "package",
          "dependencies": [],
          "wheels": [
            {
              "hashes": {
                "sha256": "[SHA256]"
              },
              "filename": "gpu_a-0.1.0-py3-none-any.whl"
            }
          ]
        },
        "gpu-b==0.1.0@path+[TEMP_DIR]/gpu_b-0.1.0-py3-none-any.whl": {
          "name": "gpu-b",
          "version": "0.1.0",
          "source": {
            "path": "[TEMP_DIR]/gpu_b-0.1.0-py3-none-any.whl"
          },
          "kind": "package",
          "dependencies": [],
          "wheels": [
            {
              "hashes": {
                "sha256": "[SHA256]"
              },
              "filename": "gpu_b-0.1.0-py3-none-any.whl"
            }
          ]
        },
        "module-owner-root==0.1.0@virtual+[TEMP_DIR]/": {
          "name": "module-owner-root",
          "version": "0.1.0",
          "source": {
            "virtual": "[TEMP_DIR]/"
          },
          "kind": "package",
          "dependencies": [
            {
              "id": "gpu-a==0.1.0@path+[TEMP_DIR]/gpu_a-0.1.0-py3-none-any.whl"
            },
            {
              "id": "gpu-b==0.1.0@path+[TEMP_DIR]/gpu_b-0.1.0-py3-none-any.whl"
            },
            {
              "id": "typing-extensions==0.1.0@path+[TEMP_DIR]/typing_extensions-0.1.0-py3-none-any.whl"
            }
          ]
        },
        "typing-extensions==0.1.0@path+[TEMP_DIR]/typing_extensions-0.1.0-py3-none-any.whl": {
          "name": "typing-extensions",
          "version": "0.1.0",
          "source": {
            "path": "[TEMP_DIR]/typing_extensions-0.1.0-py3-none-any.whl"
          },
          "kind": "package",
          "dependencies": [],
          "wheels": [
            {
              "hashes": {
                "sha256": "[SHA256]"
              },
              "filename": "typing_extensions-0.1.0-py3-none-any.whl"
            }
          ]
        },
        "workspace+[TEMP_DIR]/": {
          "kind": "workspace",
          "path": "[TEMP_DIR]/",
          "dependencies": []
        }
      }
    }

    ----- stderr -----
    warning: The `uv workspace metadata` command is experimental and may change without warning. Pass `--preview-features workspace-metadata` to disable this warning.
    Resolved 4 packages in [TIME]
    "#);

    Ok(())
}

#[test]
fn workspace_metadata_module_owners_use_installed_package_id() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let py311_dir = context.temp_dir.child("py311");
    fs_err::create_dir_all(py311_dir.path())?;
    let module_owner_311 = py311_dir.child("module_owner-0.1.0-py3-none-any.whl");
    write_wheel(
        module_owner_311.path(),
        "module-owner",
        "module_owner-0.1.0",
        &[("shared.py", "")],
    )?;

    let py312_dir = context.temp_dir.child("py312");
    fs_err::create_dir_all(py312_dir.path())?;
    let module_owner_312 = py312_dir.child("module_owner-0.1.0-py3-none-any.whl");
    write_wheel(
        module_owner_312.path(),
        "module-owner",
        "module_owner-0.1.0",
        &[("shared.py", "")],
    )?;

    let module_owner_311_url = Url::from_file_path(module_owner_311.path())
        .map_err(|()| anyhow::anyhow!("failed to convert wheel path to file URL"))?;
    let module_owner_312_url = Url::from_file_path(module_owner_312.path())
        .map_err(|()| anyhow::anyhow!("failed to convert wheel path to file URL"))?;

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&format!(
            r#"[project]
name = "module-owner-root"
version = "0.1.0"
requires-python = ">=3.11"
dependencies = [
  "module-owner @ {module_owner_311_url} ; python_version < '3.12'",
  "module-owner @ {module_owner_312_url} ; python_version >= '3.12'",
]
"#
        ))?;

    let assert = context
        .workspace_metadata()
        .arg("--sync")
        .assert()
        .success();
    let metadata: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout)?;
    let module_owners = serde_json::to_string_pretty(&metadata["module_owners"])?;

    insta::with_settings!({ filters => context.filters() }, {
        insta::assert_snapshot!(module_owners, @r#"
        {
          "shared": [
            {
              "package_id": "module-owner==0.1.0@path+[TEMP_DIR]/py312/module_owner-0.1.0-py3-none-any.whl"
            }
          ]
        }
        "#);
    });

    Ok(())
}

#[test]
fn workspace_metadata_module_owners_ignore_stale_virtual_package() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let stale_owner = context
        .temp_dir
        .child("module_owner_root-0.1.0-py3-none-any.whl");
    write_wheel(
        stale_owner.path(),
        "module-owner-root",
        "module_owner_root-0.1.0",
        &[("stale.py", "")],
    )?;

    context.temp_dir.child("pyproject.toml").write_str(
        r#"[project]
name = "module-owner-root"
version = "0.1.0"
requires-python = ">=3.12"

[tool.uv]
package = false
"#,
    )?;

    context
        .pip_install()
        .arg(stale_owner.path())
        .assert()
        .success();

    let assert = context
        .workspace_metadata()
        .arg("--sync")
        .assert()
        .success();
    let metadata: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout)?;
    let module_owners = if let Some(module_owners) = metadata.get("module_owners") {
        serde_json::to_string_pretty(module_owners)?
    } else {
        "<missing>".to_string()
    };

    insta::assert_snapshot!(module_owners, @"<missing>");

    Ok(())
}

#[test]
fn workspace_metadata_module_owners_failure_is_error() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let gpu_a = context.temp_dir.child("gpu_a-0.1.0-py3-none-any.whl");
    write_wheel(gpu_a.path(), "gpu-a", "gpu_a-0.1.0", &[("gpu/a.py", "")])?;

    let gpu_a_url = Url::from_file_path(gpu_a.path())
        .map_err(|()| anyhow::anyhow!("failed to convert wheel path to file URL"))?;

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&format!(
            r#"[project]
name = "module-owner-root"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = [
  "gpu-a @ {gpu_a_url}",
]
"#
        ))?;

    context.lock().assert().success();
    fs_err::remove_file(gpu_a.path())?;

    uv_snapshot!(context.filters(), context.workspace_metadata().arg("--frozen").arg("--sync"), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    warning: The `uv workspace metadata` command is experimental and may change without warning. Pass `--preview-features workspace-metadata` to disable this warning.
    error: Failed to collect module owners
      cause: Failed to determine installation plan
      cause: Distribution not found at: file://[TEMP_DIR]/gpu_a-0.1.0-py3-none-any.whl
    "#);

    Ok(())
}

/// Test metadata for a root workspace (workspace with a root package).
#[test]
fn workspace_metadata_root_workspace() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/workspace.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let workspace = context.temp_dir.child("workspace");

    copy_dir_ignore(
        context
            .workspace_root
            .join("test/workspaces/albatross-root-workspace"),
        &workspace,
    )?;

    uv_snapshot!(context.filters(), context.workspace_metadata().current_dir(&workspace), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "workspace_root": "[TEMP_DIR]/workspace",
      "workspace": {
        "path": "[TEMP_DIR]/workspace",
        "id": "workspace+[TEMP_DIR]/workspace"
      },
      "requires_python": ">=3.12",
      "conflicts": {
        "sets": []
      },
      "members": [
        {
          "name": "albatross",
          "path": "[TEMP_DIR]/workspace",
          "id": "albatross==0.1.0@editable+[TEMP_DIR]/workspace/"
        },
        {
          "name": "bird-feeder",
          "path": "[TEMP_DIR]/workspace/packages/bird-feeder",
          "id": "bird-feeder==1.0.0@editable+[TEMP_DIR]/workspace/packages/bird-feeder"
        },
        {
          "name": "seeds",
          "path": "[TEMP_DIR]/workspace/packages/seeds",
          "id": "seeds==1.0.0@editable+[TEMP_DIR]/workspace/packages/seeds"
        }
      ],
      "resolution": {
        "albatross==0.1.0@editable+[TEMP_DIR]/workspace/": {
          "name": "albatross",
          "version": "0.1.0",
          "source": {
            "editable": "[TEMP_DIR]/workspace/"
          },
          "kind": "package",
          "dependencies": [
            {
              "id": "bird-feeder==1.0.0@editable+[TEMP_DIR]/workspace/packages/bird-feeder"
            },
            {
              "id": "iniconfig==2.0.0@registry+http://[LOCALHOST]/simple/"
            }
          ]
        },
        "bird-feeder==1.0.0@editable+[TEMP_DIR]/workspace/packages/bird-feeder": {
          "name": "bird-feeder",
          "version": "1.0.0",
          "source": {
            "editable": "[TEMP_DIR]/workspace/packages/bird-feeder"
          },
          "kind": "package",
          "dependencies": [
            {
              "id": "iniconfig==2.0.0@registry+http://[LOCALHOST]/simple/"
            },
            {
              "id": "seeds==1.0.0@editable+[TEMP_DIR]/workspace/packages/seeds"
            }
          ]
        },
        "idna==3.6@registry+http://[LOCALHOST]/simple/": {
          "name": "idna",
          "version": "3.6",
          "source": {
            "registry": {
              "url": "http://[LOCALHOST]/simple/"
            }
          },
          "kind": "package",
          "dependencies": [],
          "sdist": {
            "url": "http://[LOCALHOST]/files/idna-3.6.tar.gz",
            "hashes": {
              "sha256": "17c3305b5e499cc941947e6a5b235c4ff7b4364a4034cd404239eecf341bc1a6"
            },
            "upload_time": "2024-03-24T00:00:00Z"
          },
          "wheels": [
            {
              "url": "http://[LOCALHOST]/files/idna-3.6-py3-none-any.whl",
              "hashes": {
                "sha256": "30a9a2a1651ab73e8e74b49e5ab084e4d0ad280bc9280d5a7f978ae44a05aa78"
              },
              "upload_time": "2024-03-24T00:00:00Z",
              "filename": "idna-3.6-py3-none-any.whl"
            }
          ]
        },
        "iniconfig==2.0.0@registry+http://[LOCALHOST]/simple/": {
          "name": "iniconfig",
          "version": "2.0.0",
          "source": {
            "registry": {
              "url": "http://[LOCALHOST]/simple/"
            }
          },
          "kind": "package",
          "dependencies": [],
          "sdist": {
            "url": "http://[LOCALHOST]/files/iniconfig-2.0.0.tar.gz",
            "hashes": {
              "sha256": "33672cc386cd5920d7edd83e6e8e231cb0f98a5c3644f7156ca5002710906680"
            },
            "upload_time": "2024-03-24T00:00:00Z"
          },
          "wheels": [
            {
              "url": "http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl",
              "hashes": {
                "sha256": "535b954a261c3adcacbb744f0259ae3e7083c5b20cb79e063293f581356f2b52"
              },
              "upload_time": "2024-03-24T00:00:00Z",
              "filename": "iniconfig-2.0.0-py3-none-any.whl"
            }
          ]
        },
        "seeds==1.0.0@editable+[TEMP_DIR]/workspace/packages/seeds": {
          "name": "seeds",
          "version": "1.0.0",
          "source": {
            "editable": "[TEMP_DIR]/workspace/packages/seeds"
          },
          "kind": "package",
          "dependencies": [
            {
              "id": "idna==3.6@registry+http://[LOCALHOST]/simple/"
            }
          ]
        },
        "workspace+[TEMP_DIR]/workspace": {
          "kind": "workspace",
          "path": "[TEMP_DIR]/workspace",
          "dependencies": []
        }
      }
    }

    ----- stderr -----
    warning: The `uv workspace metadata` command is experimental and may change without warning. Pass `--preview-features workspace-metadata` to disable this warning.
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Resolved 5 packages in [TIME]
    "#
    );

    Ok(())
}

/// Test metadata for a virtual workspace (no root package).
#[test]
fn workspace_metadata_virtual_workspace() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/workspace.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let workspace = context.temp_dir.child("workspace");

    copy_dir_ignore(
        context
            .workspace_root
            .join("test/workspaces/albatross-virtual-workspace"),
        &workspace,
    )?;

    let mut filters = context.filters();
    filters.push((
        r"(?m)^WARN Ignoring non-directory workspace member: `[^\n]+`\n",
        "",
    ));

    uv_snapshot!(filters, context.workspace_metadata().current_dir(&workspace), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "workspace_root": "[TEMP_DIR]/workspace",
      "workspace": {
        "path": "[TEMP_DIR]/workspace",
        "id": "workspace+[TEMP_DIR]/workspace"
      },
      "requires_python": ">=3.12",
      "conflicts": {
        "sets": []
      },
      "members": [
        {
          "name": "albatross",
          "path": "[TEMP_DIR]/workspace/packages/albatross",
          "id": "albatross==0.1.0@editable+[TEMP_DIR]/workspace/packages/albatross"
        },
        {
          "name": "bird-feeder",
          "path": "[TEMP_DIR]/workspace/packages/bird-feeder",
          "id": "bird-feeder==1.0.0@editable+[TEMP_DIR]/workspace/packages/bird-feeder"
        },
        {
          "name": "seeds",
          "path": "[TEMP_DIR]/workspace/packages/seeds",
          "id": "seeds==1.0.0@editable+[TEMP_DIR]/workspace/packages/seeds"
        }
      ],
      "resolution": {
        "albatross==0.1.0@editable+[TEMP_DIR]/workspace/packages/albatross": {
          "name": "albatross",
          "version": "0.1.0",
          "source": {
            "editable": "[TEMP_DIR]/workspace/packages/albatross"
          },
          "kind": "package",
          "dependencies": [
            {
              "id": "bird-feeder==1.0.0@editable+[TEMP_DIR]/workspace/packages/bird-feeder"
            },
            {
              "id": "iniconfig==2.0.0@registry+http://[LOCALHOST]/simple/"
            }
          ]
        },
        "anyio==4.3.0@registry+http://[LOCALHOST]/simple/": {
          "name": "anyio",
          "version": "4.3.0",
          "source": {
            "registry": {
              "url": "http://[LOCALHOST]/simple/"
            }
          },
          "kind": "package",
          "dependencies": [
            {
              "id": "idna==3.6@registry+http://[LOCALHOST]/simple/"
            },
            {
              "id": "sniffio==1.3.1@registry+http://[LOCALHOST]/simple/"
            }
          ],
          "sdist": {
            "url": "http://[LOCALHOST]/files/anyio-4.3.0.tar.gz",
            "hashes": {
              "sha256": "40e76bce278e96f0fc43374abf030a483db9cc9accb20f829d838e8593d5fb81"
            },
            "upload_time": "2024-03-24T00:00:00Z"
          },
          "wheels": [
            {
              "url": "http://[LOCALHOST]/files/anyio-4.3.0-py3-none-any.whl",
              "hashes": {
                "sha256": "0786abaa19b025c388576d6fa37439f4d8dde3d0c81d50bcf4138d1aa5bea724"
              },
              "upload_time": "2024-03-24T00:00:00Z",
              "filename": "anyio-4.3.0-py3-none-any.whl"
            }
          ]
        },
        "bird-feeder==1.0.0@editable+[TEMP_DIR]/workspace/packages/bird-feeder": {
          "name": "bird-feeder",
          "version": "1.0.0",
          "source": {
            "editable": "[TEMP_DIR]/workspace/packages/bird-feeder"
          },
          "kind": "package",
          "dependencies": [
            {
              "id": "anyio==4.3.0@registry+http://[LOCALHOST]/simple/"
            },
            {
              "id": "seeds==1.0.0@editable+[TEMP_DIR]/workspace/packages/seeds"
            }
          ]
        },
        "idna==3.6@registry+http://[LOCALHOST]/simple/": {
          "name": "idna",
          "version": "3.6",
          "source": {
            "registry": {
              "url": "http://[LOCALHOST]/simple/"
            }
          },
          "kind": "package",
          "dependencies": [],
          "sdist": {
            "url": "http://[LOCALHOST]/files/idna-3.6.tar.gz",
            "hashes": {
              "sha256": "17c3305b5e499cc941947e6a5b235c4ff7b4364a4034cd404239eecf341bc1a6"
            },
            "upload_time": "2024-03-24T00:00:00Z"
          },
          "wheels": [
            {
              "url": "http://[LOCALHOST]/files/idna-3.6-py3-none-any.whl",
              "hashes": {
                "sha256": "30a9a2a1651ab73e8e74b49e5ab084e4d0ad280bc9280d5a7f978ae44a05aa78"
              },
              "upload_time": "2024-03-24T00:00:00Z",
              "filename": "idna-3.6-py3-none-any.whl"
            }
          ]
        },
        "iniconfig==2.0.0@registry+http://[LOCALHOST]/simple/": {
          "name": "iniconfig",
          "version": "2.0.0",
          "source": {
            "registry": {
              "url": "http://[LOCALHOST]/simple/"
            }
          },
          "kind": "package",
          "dependencies": [],
          "sdist": {
            "url": "http://[LOCALHOST]/files/iniconfig-2.0.0.tar.gz",
            "hashes": {
              "sha256": "33672cc386cd5920d7edd83e6e8e231cb0f98a5c3644f7156ca5002710906680"
            },
            "upload_time": "2024-03-24T00:00:00Z"
          },
          "wheels": [
            {
              "url": "http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl",
              "hashes": {
                "sha256": "535b954a261c3adcacbb744f0259ae3e7083c5b20cb79e063293f581356f2b52"
              },
              "upload_time": "2024-03-24T00:00:00Z",
              "filename": "iniconfig-2.0.0-py3-none-any.whl"
            }
          ]
        },
        "seeds==1.0.0@editable+[TEMP_DIR]/workspace/packages/seeds": {
          "name": "seeds",
          "version": "1.0.0",
          "source": {
            "editable": "[TEMP_DIR]/workspace/packages/seeds"
          },
          "kind": "package",
          "dependencies": [
            {
              "id": "idna==3.6@registry+http://[LOCALHOST]/simple/"
            }
          ]
        },
        "sniffio==1.3.1@registry+http://[LOCALHOST]/simple/": {
          "name": "sniffio",
          "version": "1.3.1",
          "source": {
            "registry": {
              "url": "http://[LOCALHOST]/simple/"
            }
          },
          "kind": "package",
          "dependencies": [],
          "sdist": {
            "url": "http://[LOCALHOST]/files/sniffio-1.3.1.tar.gz",
            "hashes": {
              "sha256": "77f2e620becbdf061b22f1a43376c215c0d11acb00b418450ce50ddec444aec0"
            },
            "upload_time": "2024-03-24T00:00:00Z"
          },
          "wheels": [
            {
              "url": "http://[LOCALHOST]/files/sniffio-1.3.1-py3-none-any.whl",
              "hashes": {
                "sha256": "70679c208c27416a7f48156e0eb74c0a88a7d7e92eb9f70ec7f97c7d3a6bb183"
              },
              "upload_time": "2024-03-24T00:00:00Z",
              "filename": "sniffio-1.3.1-py3-none-any.whl"
            }
          ]
        },
        "workspace+[TEMP_DIR]/workspace": {
          "kind": "workspace",
          "path": "[TEMP_DIR]/workspace",
          "dependencies": []
        }
      }
    }

    ----- stderr -----
    warning: The `uv workspace metadata` command is experimental and may change without warning. Pass `--preview-features workspace-metadata` to disable this warning.
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Resolved 7 packages in [TIME]
    "#
    );

    Ok(())
}

/// Test metadata when run from a workspace member directory.
#[test]
fn workspace_metadata_from_member() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/workspace.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let workspace = context.temp_dir.child("workspace");

    copy_dir_ignore(
        context
            .workspace_root
            .join("test/workspaces/albatross-root-workspace"),
        &workspace,
    )?;

    let member_dir = workspace.join("packages").join("bird-feeder");

    uv_snapshot!(context.filters(), context.workspace_metadata().current_dir(&member_dir), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "workspace_root": "[TEMP_DIR]/workspace",
      "workspace": {
        "path": "[TEMP_DIR]/workspace",
        "id": "workspace+[TEMP_DIR]/workspace"
      },
      "requires_python": ">=3.12",
      "conflicts": {
        "sets": []
      },
      "members": [
        {
          "name": "albatross",
          "path": "[TEMP_DIR]/workspace",
          "id": "albatross==0.1.0@editable+[TEMP_DIR]/workspace/"
        },
        {
          "name": "bird-feeder",
          "path": "[TEMP_DIR]/workspace/packages/bird-feeder",
          "id": "bird-feeder==1.0.0@editable+[TEMP_DIR]/workspace/packages/bird-feeder"
        },
        {
          "name": "seeds",
          "path": "[TEMP_DIR]/workspace/packages/seeds",
          "id": "seeds==1.0.0@editable+[TEMP_DIR]/workspace/packages/seeds"
        }
      ],
      "resolution": {
        "albatross==0.1.0@editable+[TEMP_DIR]/workspace/": {
          "name": "albatross",
          "version": "0.1.0",
          "source": {
            "editable": "[TEMP_DIR]/workspace/"
          },
          "kind": "package",
          "dependencies": [
            {
              "id": "bird-feeder==1.0.0@editable+[TEMP_DIR]/workspace/packages/bird-feeder"
            },
            {
              "id": "iniconfig==2.0.0@registry+http://[LOCALHOST]/simple/"
            }
          ]
        },
        "bird-feeder==1.0.0@editable+[TEMP_DIR]/workspace/packages/bird-feeder": {
          "name": "bird-feeder",
          "version": "1.0.0",
          "source": {
            "editable": "[TEMP_DIR]/workspace/packages/bird-feeder"
          },
          "kind": "package",
          "dependencies": [
            {
              "id": "iniconfig==2.0.0@registry+http://[LOCALHOST]/simple/"
            },
            {
              "id": "seeds==1.0.0@editable+[TEMP_DIR]/workspace/packages/seeds"
            }
          ]
        },
        "idna==3.6@registry+http://[LOCALHOST]/simple/": {
          "name": "idna",
          "version": "3.6",
          "source": {
            "registry": {
              "url": "http://[LOCALHOST]/simple/"
            }
          },
          "kind": "package",
          "dependencies": [],
          "sdist": {
            "url": "http://[LOCALHOST]/files/idna-3.6.tar.gz",
            "hashes": {
              "sha256": "17c3305b5e499cc941947e6a5b235c4ff7b4364a4034cd404239eecf341bc1a6"
            },
            "upload_time": "2024-03-24T00:00:00Z"
          },
          "wheels": [
            {
              "url": "http://[LOCALHOST]/files/idna-3.6-py3-none-any.whl",
              "hashes": {
                "sha256": "30a9a2a1651ab73e8e74b49e5ab084e4d0ad280bc9280d5a7f978ae44a05aa78"
              },
              "upload_time": "2024-03-24T00:00:00Z",
              "filename": "idna-3.6-py3-none-any.whl"
            }
          ]
        },
        "iniconfig==2.0.0@registry+http://[LOCALHOST]/simple/": {
          "name": "iniconfig",
          "version": "2.0.0",
          "source": {
            "registry": {
              "url": "http://[LOCALHOST]/simple/"
            }
          },
          "kind": "package",
          "dependencies": [],
          "sdist": {
            "url": "http://[LOCALHOST]/files/iniconfig-2.0.0.tar.gz",
            "hashes": {
              "sha256": "33672cc386cd5920d7edd83e6e8e231cb0f98a5c3644f7156ca5002710906680"
            },
            "upload_time": "2024-03-24T00:00:00Z"
          },
          "wheels": [
            {
              "url": "http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl",
              "hashes": {
                "sha256": "535b954a261c3adcacbb744f0259ae3e7083c5b20cb79e063293f581356f2b52"
              },
              "upload_time": "2024-03-24T00:00:00Z",
              "filename": "iniconfig-2.0.0-py3-none-any.whl"
            }
          ]
        },
        "seeds==1.0.0@editable+[TEMP_DIR]/workspace/packages/seeds": {
          "name": "seeds",
          "version": "1.0.0",
          "source": {
            "editable": "[TEMP_DIR]/workspace/packages/seeds"
          },
          "kind": "package",
          "dependencies": [
            {
              "id": "idna==3.6@registry+http://[LOCALHOST]/simple/"
            }
          ]
        },
        "workspace+[TEMP_DIR]/workspace": {
          "kind": "workspace",
          "path": "[TEMP_DIR]/workspace",
          "dependencies": []
        }
      }
    }

    ----- stderr -----
    warning: The `uv workspace metadata` command is experimental and may change without warning. Pass `--preview-features workspace-metadata` to disable this warning.
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Resolved 5 packages in [TIME]
    "#
    );

    Ok(())
}

/// Test metadata for a workspace with multiple packages.
#[test]
fn workspace_metadata_multiple_members() {
    let context = uv_test::test_context!("3.12");

    // Initialize workspace root
    context.init().arg("pkg-a").assert().success();

    let workspace_root = context.temp_dir.child("pkg-a");

    // Add more members
    context
        .init()
        .arg("pkg-b")
        .current_dir(&workspace_root)
        .assert()
        .success();

    context
        .init()
        .arg("pkg-c")
        .current_dir(&workspace_root)
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.workspace_metadata().current_dir(&workspace_root), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "workspace_root": "[TEMP_DIR]/pkg-a",
      "workspace": {
        "path": "[TEMP_DIR]/pkg-a",
        "id": "workspace+[TEMP_DIR]/pkg-a"
      },
      "requires_python": ">=3.12",
      "conflicts": {
        "sets": []
      },
      "members": [
        {
          "name": "pkg-a",
          "path": "[TEMP_DIR]/pkg-a",
          "id": "pkg-a==0.1.0@editable+[TEMP_DIR]/pkg-a/"
        },
        {
          "name": "pkg-b",
          "path": "[TEMP_DIR]/pkg-a/pkg-b",
          "id": "pkg-b==0.1.0@editable+[TEMP_DIR]/pkg-a/pkg-b"
        },
        {
          "name": "pkg-c",
          "path": "[TEMP_DIR]/pkg-a/pkg-c",
          "id": "pkg-c==0.1.0@editable+[TEMP_DIR]/pkg-a/pkg-c"
        }
      ],
      "resolution": {
        "pkg-a==0.1.0@editable+[TEMP_DIR]/pkg-a/": {
          "name": "pkg-a",
          "version": "0.1.0",
          "source": {
            "editable": "[TEMP_DIR]/pkg-a/"
          },
          "kind": "package",
          "dependencies": []
        },
        "pkg-b==0.1.0@editable+[TEMP_DIR]/pkg-a/pkg-b": {
          "name": "pkg-b",
          "version": "0.1.0",
          "source": {
            "editable": "[TEMP_DIR]/pkg-a/pkg-b"
          },
          "kind": "package",
          "dependencies": []
        },
        "pkg-c==0.1.0@editable+[TEMP_DIR]/pkg-a/pkg-c": {
          "name": "pkg-c",
          "version": "0.1.0",
          "source": {
            "editable": "[TEMP_DIR]/pkg-a/pkg-c"
          },
          "kind": "package",
          "dependencies": []
        },
        "workspace+[TEMP_DIR]/pkg-a": {
          "kind": "workspace",
          "path": "[TEMP_DIR]/pkg-a",
          "dependencies": []
        }
      }
    }

    ----- stderr -----
    warning: The `uv workspace metadata` command is experimental and may change without warning. Pass `--preview-features workspace-metadata` to disable this warning.
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Resolved 3 packages in [TIME]
    "#
    );
}

/// Test metadata for a single project (not a workspace).
#[test]
fn workspace_metadata_single_project() {
    let context = uv_test::test_context!("3.12");

    context.init().arg("my-project").assert().success();

    let project = context.temp_dir.child("my-project");

    uv_snapshot!(context.filters(), context.workspace_metadata().current_dir(&project), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "workspace_root": "[TEMP_DIR]/my-project",
      "workspace": {
        "path": "[TEMP_DIR]/my-project",
        "id": "workspace+[TEMP_DIR]/my-project"
      },
      "requires_python": ">=3.12",
      "conflicts": {
        "sets": []
      },
      "members": [
        {
          "name": "my-project",
          "path": "[TEMP_DIR]/my-project",
          "id": "my-project==0.1.0@editable+[TEMP_DIR]/my-project/"
        }
      ],
      "resolution": {
        "my-project==0.1.0@editable+[TEMP_DIR]/my-project/": {
          "name": "my-project",
          "version": "0.1.0",
          "source": {
            "editable": "[TEMP_DIR]/my-project/"
          },
          "kind": "package",
          "dependencies": []
        },
        "workspace+[TEMP_DIR]/my-project": {
          "kind": "workspace",
          "path": "[TEMP_DIR]/my-project",
          "dependencies": []
        }
      }
    }

    ----- stderr -----
    warning: The `uv workspace metadata` command is experimental and may change without warning. Pass `--preview-features workspace-metadata` to disable this warning.
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Resolved 1 package in [TIME]
    "#
    );
}

/// Test metadata with excluded packages.
#[test]
fn workspace_metadata_with_excluded() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/workspace.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let workspace = context.temp_dir.child("workspace");

    copy_dir_ignore(
        context
            .workspace_root
            .join("test/workspaces/albatross-project-in-excluded"),
        &workspace,
    )?;

    uv_snapshot!(context.filters(), context.workspace_metadata().current_dir(&workspace), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "workspace_root": "[TEMP_DIR]/workspace",
      "workspace": {
        "path": "[TEMP_DIR]/workspace",
        "id": "workspace+[TEMP_DIR]/workspace"
      },
      "requires_python": ">=3.12",
      "conflicts": {
        "sets": []
      },
      "members": [
        {
          "name": "albatross",
          "path": "[TEMP_DIR]/workspace",
          "id": "albatross==0.1.0@editable+[TEMP_DIR]/workspace/"
        }
      ],
      "resolution": {
        "albatross==0.1.0@editable+[TEMP_DIR]/workspace/": {
          "name": "albatross",
          "version": "0.1.0",
          "source": {
            "editable": "[TEMP_DIR]/workspace/"
          },
          "kind": "package",
          "dependencies": [
            {
              "id": "iniconfig==2.0.0@registry+http://[LOCALHOST]/simple/"
            }
          ]
        },
        "iniconfig==2.0.0@registry+http://[LOCALHOST]/simple/": {
          "name": "iniconfig",
          "version": "2.0.0",
          "source": {
            "registry": {
              "url": "http://[LOCALHOST]/simple/"
            }
          },
          "kind": "package",
          "dependencies": [],
          "sdist": {
            "url": "http://[LOCALHOST]/files/iniconfig-2.0.0.tar.gz",
            "hashes": {
              "sha256": "33672cc386cd5920d7edd83e6e8e231cb0f98a5c3644f7156ca5002710906680"
            },
            "upload_time": "2024-03-24T00:00:00Z"
          },
          "wheels": [
            {
              "url": "http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl",
              "hashes": {
                "sha256": "535b954a261c3adcacbb744f0259ae3e7083c5b20cb79e063293f581356f2b52"
              },
              "upload_time": "2024-03-24T00:00:00Z",
              "filename": "iniconfig-2.0.0-py3-none-any.whl"
            }
          ]
        },
        "workspace+[TEMP_DIR]/workspace": {
          "kind": "workspace",
          "path": "[TEMP_DIR]/workspace",
          "dependencies": []
        }
      }
    }

    ----- stderr -----
    warning: The `uv workspace metadata` command is experimental and may change without warning. Pass `--preview-features workspace-metadata` to disable this warning.
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Resolved 2 packages in [TIME]
    "#
    );

    Ok(())
}

/// Test metadata for dependency groups defined on a non-package workspace root.
#[test]
fn workspace_metadata_group_only() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/workspace.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let workspace = context.temp_dir.child("workspace");

    copy_dir_ignore(
        context
            .workspace_root
            .join("test/workspaces/albatross-groups-only"),
        &workspace,
    )?;

    uv_snapshot!(context.filters(), context.workspace_metadata().current_dir(&workspace), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "workspace_root": "[TEMP_DIR]/workspace",
      "workspace": {
        "path": "[TEMP_DIR]/workspace",
        "id": "workspace+[TEMP_DIR]/workspace"
      },
      "requires_python": ">=3.12",
      "conflicts": {
        "sets": []
      },
      "resolution": {
        "iniconfig==2.0.0@registry+http://[LOCALHOST]/simple/": {
          "name": "iniconfig",
          "version": "2.0.0",
          "source": {
            "registry": {
              "url": "http://[LOCALHOST]/simple/"
            }
          },
          "kind": "package",
          "dependencies": [],
          "sdist": {
            "url": "http://[LOCALHOST]/files/iniconfig-2.0.0.tar.gz",
            "hashes": {
              "sha256": "33672cc386cd5920d7edd83e6e8e231cb0f98a5c3644f7156ca5002710906680"
            },
            "upload_time": "2024-03-24T00:00:00Z"
          },
          "wheels": [
            {
              "url": "http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl",
              "hashes": {
                "sha256": "535b954a261c3adcacbb744f0259ae3e7083c5b20cb79e063293f581356f2b52"
              },
              "upload_time": "2024-03-24T00:00:00Z",
              "filename": "iniconfig-2.0.0-py3-none-any.whl"
            }
          ]
        },
        "workspace+[TEMP_DIR]/workspace": {
          "kind": "workspace",
          "path": "[TEMP_DIR]/workspace",
          "dependencies": [],
          "dependency_groups": [
            {
              "name": "dev",
              "id": "workspace+[TEMP_DIR]/workspace:dev"
            }
          ]
        },
        "workspace+[TEMP_DIR]/workspace:dev": {
          "kind": {
            "group": "dev"
          },
          "path": "[TEMP_DIR]/workspace",
          "dependencies": [
            {
              "id": "iniconfig==2.0.0@registry+http://[LOCALHOST]/simple/"
            }
          ]
        }
      }
    }

    ----- stderr -----
    warning: The `uv workspace metadata` command is experimental and may change without warning. Pass `--preview-features workspace-metadata` to disable this warning.
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    warning: No `requires-python` value found in the workspace. Defaulting to `>=3.12`.
    Resolved 1 package in [TIME]
    "#
    );

    // With `--sync`, modules provided by the non-project root's dependency group should be
    // attributed to their locked package.
    let assert = context
        .workspace_metadata()
        .arg("--sync")
        .current_dir(&workspace)
        .assert()
        .success();
    let metadata: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout)?;
    let module_owners = serde_json::to_string_pretty(&metadata["module_owners"])?;

    insta::with_settings!({ filters => context.filters() }, {
    insta::assert_snapshot!(module_owners, @r#"
    {
      "iniconfig": [
        {
          "package_id": "iniconfig==2.0.0@registry+http://[LOCALHOST]/simple/"
        }
      ]
    }
    "#);
    });

    Ok(())
}

/// Test metadata error when not in a project.
#[test]
fn workspace_metadata_no_project() {
    let context = uv_test::test_context!("3.12");

    uv_snapshot!(context.filters(), context.workspace_metadata(), @"
    exit_code: 2 (failure)
    ----- stderr -----
    warning: The `uv workspace metadata` command is experimental and may change without warning. Pass `--preview-features workspace-metadata` to disable this warning.
    error: No `pyproject.toml` found in current directory or any parent directory
    "
    );
}

/// Test optional-dependencies, dependency-groups, and build-system
#[test]
fn workspace_metadata_various_dependency_rainbow() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/workspace.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let workspace = context.temp_dir.child("workspace");

    copy_dir_ignore(
        context
            .workspace_root
            .join("test/workspaces/albatross-dependency-rainbow"),
        &workspace,
    )?;

    uv_snapshot!(context.filters(), context.workspace_metadata().current_dir(&workspace), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "workspace_root": "[TEMP_DIR]/workspace",
      "workspace": {
        "path": "[TEMP_DIR]/workspace",
        "id": "workspace+[TEMP_DIR]/workspace"
      },
      "requires_python": ">=3.12",
      "conflicts": {
        "sets": []
      },
      "members": [
        {
          "name": "albatross",
          "path": "[TEMP_DIR]/workspace",
          "id": "albatross==0.1.0@editable+[TEMP_DIR]/workspace/"
        }
      ],
      "resolution": {
        "albatross:dev==0.1.0@editable+[TEMP_DIR]/workspace/": {
          "name": "albatross",
          "version": "0.1.0",
          "source": {
            "editable": "[TEMP_DIR]/workspace/"
          },
          "kind": {
            "group": "dev"
          },
          "dependencies": [
            {
              "id": "idna==3.6@registry+http://[LOCALHOST]/simple/"
            }
          ]
        },
        "albatross==0.1.0@editable+[TEMP_DIR]/workspace/": {
          "name": "albatross",
          "version": "0.1.0",
          "source": {
            "editable": "[TEMP_DIR]/workspace/"
          },
          "kind": "package",
          "dependencies": [
            {
              "id": "iniconfig==2.0.0@registry+http://[LOCALHOST]/simple/"
            }
          ],
          "optional_dependencies": [
            {
              "name": "io",
              "id": "albatross[io]==0.1.0@editable+[TEMP_DIR]/workspace/"
            }
          ],
          "dependency_groups": [
            {
              "name": "dev",
              "id": "albatross:dev==0.1.0@editable+[TEMP_DIR]/workspace/"
            }
          ]
        },
        "albatross[io]==0.1.0@editable+[TEMP_DIR]/workspace/": {
          "name": "albatross",
          "version": "0.1.0",
          "source": {
            "editable": "[TEMP_DIR]/workspace/"
          },
          "kind": {
            "extra": "io"
          },
          "dependencies": [
            {
              "id": "albatross==0.1.0@editable+[TEMP_DIR]/workspace/"
            },
            {
              "id": "anyio==4.3.0@registry+http://[LOCALHOST]/simple/"
            }
          ]
        },
        "anyio==4.3.0@registry+http://[LOCALHOST]/simple/": {
          "name": "anyio",
          "version": "4.3.0",
          "source": {
            "registry": {
              "url": "http://[LOCALHOST]/simple/"
            }
          },
          "kind": "package",
          "dependencies": [
            {
              "id": "idna==3.6@registry+http://[LOCALHOST]/simple/"
            },
            {
              "id": "sniffio==1.3.1@registry+http://[LOCALHOST]/simple/"
            }
          ],
          "sdist": {
            "url": "http://[LOCALHOST]/files/anyio-4.3.0.tar.gz",
            "hashes": {
              "sha256": "40e76bce278e96f0fc43374abf030a483db9cc9accb20f829d838e8593d5fb81"
            },
            "upload_time": "2024-03-24T00:00:00Z"
          },
          "wheels": [
            {
              "url": "http://[LOCALHOST]/files/anyio-4.3.0-py3-none-any.whl",
              "hashes": {
                "sha256": "0786abaa19b025c388576d6fa37439f4d8dde3d0c81d50bcf4138d1aa5bea724"
              },
              "upload_time": "2024-03-24T00:00:00Z",
              "filename": "anyio-4.3.0-py3-none-any.whl"
            }
          ]
        },
        "idna==3.6@registry+http://[LOCALHOST]/simple/": {
          "name": "idna",
          "version": "3.6",
          "source": {
            "registry": {
              "url": "http://[LOCALHOST]/simple/"
            }
          },
          "kind": "package",
          "dependencies": [],
          "sdist": {
            "url": "http://[LOCALHOST]/files/idna-3.6.tar.gz",
            "hashes": {
              "sha256": "17c3305b5e499cc941947e6a5b235c4ff7b4364a4034cd404239eecf341bc1a6"
            },
            "upload_time": "2024-03-24T00:00:00Z"
          },
          "wheels": [
            {
              "url": "http://[LOCALHOST]/files/idna-3.6-py3-none-any.whl",
              "hashes": {
                "sha256": "30a9a2a1651ab73e8e74b49e5ab084e4d0ad280bc9280d5a7f978ae44a05aa78"
              },
              "upload_time": "2024-03-24T00:00:00Z",
              "filename": "idna-3.6-py3-none-any.whl"
            }
          ]
        },
        "iniconfig==2.0.0@registry+http://[LOCALHOST]/simple/": {
          "name": "iniconfig",
          "version": "2.0.0",
          "source": {
            "registry": {
              "url": "http://[LOCALHOST]/simple/"
            }
          },
          "kind": "package",
          "dependencies": [],
          "sdist": {
            "url": "http://[LOCALHOST]/files/iniconfig-2.0.0.tar.gz",
            "hashes": {
              "sha256": "33672cc386cd5920d7edd83e6e8e231cb0f98a5c3644f7156ca5002710906680"
            },
            "upload_time": "2024-03-24T00:00:00Z"
          },
          "wheels": [
            {
              "url": "http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl",
              "hashes": {
                "sha256": "535b954a261c3adcacbb744f0259ae3e7083c5b20cb79e063293f581356f2b52"
              },
              "upload_time": "2024-03-24T00:00:00Z",
              "filename": "iniconfig-2.0.0-py3-none-any.whl"
            }
          ]
        },
        "sniffio==1.3.1@registry+http://[LOCALHOST]/simple/": {
          "name": "sniffio",
          "version": "1.3.1",
          "source": {
            "registry": {
              "url": "http://[LOCALHOST]/simple/"
            }
          },
          "kind": "package",
          "dependencies": [],
          "sdist": {
            "url": "http://[LOCALHOST]/files/sniffio-1.3.1.tar.gz",
            "hashes": {
              "sha256": "77f2e620becbdf061b22f1a43376c215c0d11acb00b418450ce50ddec444aec0"
            },
            "upload_time": "2024-03-24T00:00:00Z"
          },
          "wheels": [
            {
              "url": "http://[LOCALHOST]/files/sniffio-1.3.1-py3-none-any.whl",
              "hashes": {
                "sha256": "70679c208c27416a7f48156e0eb74c0a88a7d7e92eb9f70ec7f97c7d3a6bb183"
              },
              "upload_time": "2024-03-24T00:00:00Z",
              "filename": "sniffio-1.3.1-py3-none-any.whl"
            }
          ]
        },
        "workspace+[TEMP_DIR]/workspace": {
          "kind": "workspace",
          "path": "[TEMP_DIR]/workspace",
          "dependencies": []
        }
      }
    }

    ----- stderr -----
    warning: The `uv workspace metadata` command is experimental and may change without warning. Pass `--preview-features workspace-metadata` to disable this warning.
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Resolved 5 packages in [TIME]
    "#
    );

    Ok(())
}
