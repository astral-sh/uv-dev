#[cfg(feature = "test-universal")]
use std::process::Command;

use anyhow::{Context, Result, bail};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::{formatdoc, indoc};
use insta::{assert_json_snapshot, assert_snapshot};
use url::Url;

use uv_static::EnvVars;
#[cfg(feature = "test-universal")]
use uv_test::TestContext;
use uv_test::uv_snapshot;

/// The workspace discovered while resolving settings is reused by `uv tree`.
#[test]
fn tree_reuses_settings_workspace_discovery() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
            [project]
            name = "root"
            version = "0.1.0"
            requires-python = ">=3.12"

            [tool.uv.workspace]
            members = ["member"]
        "#})?;
    let member = context.temp_dir.child("member");
    member.create_dir_all()?;
    member
        .child("pyproject.toml")
        .write_str("[project]\nname = \"member\"\nversion = \"0.1.0\"\n")?;

    context.lock().assert().success();

    uv_snapshot!(context.filters(), context.tree()
        .arg("--frozen")
        .arg("--universal")
        .env(EnvVars::RUST_LOG, "uv_workspace=trace"), @"
    exit_code: 0 (success)
    ----- stdout -----
    root v0.1.0
    member v0.1.0

    ----- stderr -----
    DEBUG Found workspace root: `[TEMP_DIR]/`
    TRACE Discovering workspace members for: `[TEMP_DIR]/`
    DEBUG Adding root workspace member: `[TEMP_DIR]/`
    TRACE Processing workspace member: `member`
    DEBUG Adding discovered workspace member: `[TEMP_DIR]/member`
    DEBUG Found project root: `[TEMP_DIR]/`
    ");

    Ok(())
}

#[test]
fn tree_centralized_environment_no_cache() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    fs_err::remove_dir_all(&context.venv)?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context.tree()
        .arg("--no-cache")
        .arg("--preview-features")
        .arg("centralized-project-envs"), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0

    ----- stderr -----
    warning: The `centralized-project-envs` feature has no effect when `--no-cache` is enabled
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Resolved 1 package in [TIME]
    ");

    assert!(!context.venv.exists());
    Ok(())
}

#[cfg(feature = "test-universal")]
#[test]
fn nested_dependencies() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "tree-root"
        ]
    "#,
    )?;

    uv_snapshot!(context.filters(), context.tree().arg("--universal"), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    └── tree-root v3.0.2
        ├── tree-branch-a v1.7.0
        ├── tree-branch-b v8.1.7
        ├── tree-branch-c v2.1.2
        ├── tree-branch-d v3.1.3
        │   └── tree-shared-leaf v2.1.5
        └── tree-branch-e v3.0.1
            └── tree-shared-leaf v2.1.5

    ----- stderr -----
    Resolved 8 packages in [TIME]
    "
    );

    // `uv tree` should update the lockfile
    let lock = context.read("uv.lock");
    assert!(!lock.is_empty());

    Ok(())
}

#[cfg(feature = "test-universal")]
#[test]
fn json_output() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    setup_json_output(&context)?;

    uv_snapshot!(context.filters(), context.tree()
        .arg("--preview-features")
        .arg("json-output")
        .arg("--format")
        .arg("json")
        .arg("--universal"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "workspace_root": "[TEMP_DIR]/",
      "workspace": {
        "path": "[TEMP_DIR]/",
        "id": "workspace+[TEMP_DIR]/"
      },
      "roots": [
        {
          "id": "project:dev==0.1.0@virtual+[TEMP_DIR]/"
        },
        {
          "id": "project==0.1.0@virtual+[TEMP_DIR]/"
        }
      ],
      "inverted": false,
      "members": [
        {
          "name": "project",
          "path": "[TEMP_DIR]/",
          "id": "project==0.1.0@virtual+[TEMP_DIR]/"
        }
      ],
      "resolution": {
        "package-a==1.0.0@directory+[TEMP_DIR]/packages/package-a": {
          "name": "package-a",
          "version": "1.0.0",
          "source": {
            "directory": "[TEMP_DIR]/packages/package-a"
          },
          "kind": "package",
          "dependencies": [],
          "optional_dependencies": [
            {
              "name": "feature",
              "id": "package-a[feature]==1.0.0@directory+[TEMP_DIR]/packages/package-a"
            }
          ]
        },
        "package-a[feature]==1.0.0@directory+[TEMP_DIR]/packages/package-a": {
          "name": "package-a",
          "version": "1.0.0",
          "source": {
            "directory": "[TEMP_DIR]/packages/package-a"
          },
          "kind": {
            "extra": "feature"
          },
          "dependencies": [
            {
              "id": "package-a==1.0.0@directory+[TEMP_DIR]/packages/package-a"
            },
            {
              "id": "package-b==1.0.0@directory+[TEMP_DIR]/packages/package-b"
            }
          ]
        },
        "package-b==1.0.0@directory+[TEMP_DIR]/packages/package-b": {
          "name": "package-b",
          "version": "1.0.0",
          "source": {
            "directory": "[TEMP_DIR]/packages/package-b"
          },
          "kind": "package",
          "dependencies": []
        },
        "package-c==1.0.0@directory+[TEMP_DIR]/packages/package-c": {
          "name": "package-c",
          "version": "1.0.0",
          "source": {
            "directory": "[TEMP_DIR]/packages/package-c"
          },
          "kind": "package",
          "dependencies": []
        },
        "project:dev==0.1.0@virtual+[TEMP_DIR]/": {
          "name": "project",
          "version": "0.1.0",
          "source": {
            "virtual": "[TEMP_DIR]/"
          },
          "kind": {
            "group": "dev"
          },
          "dependencies": [
            {
              "id": "package-c==1.0.0@directory+[TEMP_DIR]/packages/package-c"
            }
          ]
        },
        "project==0.1.0@virtual+[TEMP_DIR]/": {
          "name": "project",
          "version": "0.1.0",
          "source": {
            "virtual": "[TEMP_DIR]/"
          },
          "kind": "package",
          "dependencies": [
            {
              "id": "package-a[feature]==1.0.0@directory+[TEMP_DIR]/packages/package-a"
            }
          ],
          "dependency_groups": [
            {
              "name": "dev",
              "id": "project:dev==0.1.0@virtual+[TEMP_DIR]/"
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
    Resolved 4 packages in [TIME]
    "#);

    let assert = context
        .tree()
        .arg("--preview-features")
        .arg("json-output")
        .arg("--format")
        .arg("json")
        .arg("--universal")
        .arg("--quiet")
        .output()?
        .assert()
        .success();
    assert!(assert.get_output().stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout)?;
    let package_names = report["resolution"]
        .as_object()
        .context("dependency graph resolution should be an object")?
        .values()
        .filter(|node| node["kind"] == "package")
        .map(|node| {
            node["name"]
                .as_str()
                .context("dependency graph node should have a name")
        })
        .collect::<Result<Vec<_>>>()?;
    assert_json_snapshot!(package_names, @r#"
    [
      "package-a",
      "package-b",
      "package-c",
      "project"
    ]
    "#);

    Ok(())
}

#[cfg(feature = "test-universal")]
#[test]
fn json_output_depth() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    setup_json_output(&context)?;

    uv_snapshot!(context.filters(), context.tree()
        .arg("--preview-features")
        .arg("json-output")
        .arg("--format")
        .arg("json")
        .arg("--universal")
        .arg("--depth")
        .arg("1"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "workspace_root": "[TEMP_DIR]/",
      "workspace": {
        "path": "[TEMP_DIR]/",
        "id": "workspace+[TEMP_DIR]/"
      },
      "roots": [
        {
          "id": "project:dev==0.1.0@virtual+[TEMP_DIR]/"
        },
        {
          "id": "project==0.1.0@virtual+[TEMP_DIR]/"
        }
      ],
      "inverted": false,
      "members": [
        {
          "name": "project",
          "path": "[TEMP_DIR]/",
          "id": "project==0.1.0@virtual+[TEMP_DIR]/"
        }
      ],
      "resolution": {
        "package-a==1.0.0@directory+[TEMP_DIR]/packages/package-a": {
          "name": "package-a",
          "version": "1.0.0",
          "source": {
            "directory": "[TEMP_DIR]/packages/package-a"
          },
          "kind": "package",
          "dependencies": [],
          "optional_dependencies": [
            {
              "name": "feature",
              "id": "package-a[feature]==1.0.0@directory+[TEMP_DIR]/packages/package-a"
            }
          ]
        },
        "package-a[feature]==1.0.0@directory+[TEMP_DIR]/packages/package-a": {
          "name": "package-a",
          "version": "1.0.0",
          "source": {
            "directory": "[TEMP_DIR]/packages/package-a"
          },
          "kind": {
            "extra": "feature"
          },
          "dependencies": [
            {
              "id": "package-a==1.0.0@directory+[TEMP_DIR]/packages/package-a"
            }
          ]
        },
        "package-c==1.0.0@directory+[TEMP_DIR]/packages/package-c": {
          "name": "package-c",
          "version": "1.0.0",
          "source": {
            "directory": "[TEMP_DIR]/packages/package-c"
          },
          "kind": "package",
          "dependencies": []
        },
        "project:dev==0.1.0@virtual+[TEMP_DIR]/": {
          "name": "project",
          "version": "0.1.0",
          "source": {
            "virtual": "[TEMP_DIR]/"
          },
          "kind": {
            "group": "dev"
          },
          "dependencies": [
            {
              "id": "package-c==1.0.0@directory+[TEMP_DIR]/packages/package-c"
            }
          ]
        },
        "project==0.1.0@virtual+[TEMP_DIR]/": {
          "name": "project",
          "version": "0.1.0",
          "source": {
            "virtual": "[TEMP_DIR]/"
          },
          "kind": "package",
          "dependencies": [
            {
              "id": "package-a[feature]==1.0.0@directory+[TEMP_DIR]/packages/package-a"
            }
          ],
          "dependency_groups": [
            {
              "name": "dev",
              "id": "project:dev==0.1.0@virtual+[TEMP_DIR]/"
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
    Resolved 4 packages in [TIME]
    "#);

    Ok(())
}

#[cfg(feature = "test-universal")]
#[test]
fn json_output_inverted_depth() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    setup_json_output(&context)?;

    uv_snapshot!(context.filters(), context.tree()
        .arg("--preview-features")
        .arg("json-output")
        .arg("--format")
        .arg("json")
        .arg("--universal")
        .arg("--invert")
        .arg("--depth")
        .arg("1"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "workspace_root": "[TEMP_DIR]/",
      "workspace": {
        "path": "[TEMP_DIR]/",
        "id": "workspace+[TEMP_DIR]/"
      },
      "roots": [
        {
          "id": "package-b==1.0.0@directory+[TEMP_DIR]/packages/package-b"
        },
        {
          "id": "package-c==1.0.0@directory+[TEMP_DIR]/packages/package-c"
        }
      ],
      "inverted": true,
      "members": [
        {
          "name": "project",
          "path": "[TEMP_DIR]/",
          "id": "project==0.1.0@virtual+[TEMP_DIR]/"
        }
      ],
      "resolution": {
        "package-a==1.0.0@directory+[TEMP_DIR]/packages/package-a": {
          "name": "package-a",
          "version": "1.0.0",
          "source": {
            "directory": "[TEMP_DIR]/packages/package-a"
          },
          "kind": "package",
          "dependencies": [
            {
              "id": "package-a[feature]==1.0.0@directory+[TEMP_DIR]/packages/package-a"
            }
          ],
          "optional_dependencies": [
            {
              "name": "feature",
              "id": "package-a[feature]==1.0.0@directory+[TEMP_DIR]/packages/package-a"
            }
          ]
        },
        "package-a[feature]==1.0.0@directory+[TEMP_DIR]/packages/package-a": {
          "name": "package-a",
          "version": "1.0.0",
          "source": {
            "directory": "[TEMP_DIR]/packages/package-a"
          },
          "kind": {
            "extra": "feature"
          },
          "dependencies": []
        },
        "package-b==1.0.0@directory+[TEMP_DIR]/packages/package-b": {
          "name": "package-b",
          "version": "1.0.0",
          "source": {
            "directory": "[TEMP_DIR]/packages/package-b"
          },
          "kind": "package",
          "dependencies": [
            {
              "id": "package-a[feature]==1.0.0@directory+[TEMP_DIR]/packages/package-a"
            }
          ]
        },
        "package-c==1.0.0@directory+[TEMP_DIR]/packages/package-c": {
          "name": "package-c",
          "version": "1.0.0",
          "source": {
            "directory": "[TEMP_DIR]/packages/package-c"
          },
          "kind": "package",
          "dependencies": [
            {
              "id": "project:dev==0.1.0@virtual+[TEMP_DIR]/"
            }
          ]
        },
        "project:dev==0.1.0@virtual+[TEMP_DIR]/": {
          "name": "project",
          "version": "0.1.0",
          "source": {
            "virtual": "[TEMP_DIR]/"
          },
          "kind": {
            "group": "dev"
          },
          "dependencies": []
        },
        "project==0.1.0@virtual+[TEMP_DIR]/": {
          "name": "project",
          "version": "0.1.0",
          "source": {
            "virtual": "[TEMP_DIR]/"
          },
          "kind": "package",
          "dependencies": [],
          "dependency_groups": [
            {
              "name": "dev",
              "id": "project:dev==0.1.0@virtual+[TEMP_DIR]/"
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
    Resolved 4 packages in [TIME]
    "#);

    Ok(())
}

#[cfg(feature = "test-universal")]
#[test]
fn json_output_projected_members_respect_depth() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    setup_json_output(&context)?;

    let projected_members = |depth: Option<u8>| -> Result<Vec<String>> {
        let mut command = context.tree();
        command
            .arg("--preview-features")
            .arg("json-output")
            .arg("--format")
            .arg("json")
            .arg("--universal")
            .arg("--invert")
            .arg("--package")
            .arg("package-b");
        if let Some(depth) = depth {
            command.arg("--depth").arg(depth.to_string());
        }
        let assert = command.output()?.assert().success();
        let report: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout)?;
        report["members"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|member| {
                member["name"]
                    .as_str()
                    .context("workspace member should have a name")
                    .map(ToOwned::to_owned)
            })
            .collect()
    };

    assert_json_snapshot!(projected_members(None)?, @r#"
    [
      "project"
    ]
    "#);
    assert_json_snapshot!(projected_members(Some(1))?, @r#"[]"#);

    Ok(())
}

#[test]
fn json_output_root_contexts_respect_depth() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context.temp_dir.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [project.optional-dependencies]
        feature = ["extra-dependency"]

        [dependency-groups]
        dev = ["group-dependency"]

        [tool.uv.sources]
        extra-dependency = { path = "extra-dependency" }
        group-dependency = { path = "group-dependency" }
        "#,
    )?;

    for package in ["extra-dependency", "group-dependency"] {
        let directory = context.temp_dir.child(package);
        directory.create_dir_all()?;
        directory
            .child("pyproject.toml")
            .write_str(&formatdoc! {r#"
            [project]
            name = "{package}"
            version = "1.0.0"
            requires-python = ">=3.12"
        "#})?;
    }

    let projected_contexts = |depth: usize| -> Result<(Vec<String>, Vec<String>)> {
        let output = context
            .tree()
            .arg("--preview-features")
            .arg("json-output")
            .arg("--format")
            .arg("json")
            .arg("--universal")
            .arg("--depth")
            .arg(depth.to_string())
            .output()?;
        output.clone().assert().success();

        let report: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        let roots = report["roots"]
            .as_array()
            .context("dependency graph roots should be an array")?;
        let resolution = report["resolution"]
            .as_object()
            .context("dependency graph resolution should be an object")?;
        let context_kind = |node: &serde_json::Value| {
            let kind = &node["kind"];
            if let Some(extra) = kind["extra"].as_str() {
                Some(format!("extra: {extra}"))
            } else {
                kind["group"]
                    .as_str()
                    .map(|group| format!("group: {group}"))
            }
        };
        let root_kinds = roots
            .iter()
            .map(|root| -> Result<String> {
                let id = root["id"]
                    .as_str()
                    .context("dependency graph root should have an ID")?;
                let node = resolution
                    .get(id)
                    .context("dependency graph root should be in the resolution")?;
                let kind = &node["kind"];
                if kind == "package" {
                    Ok("package".to_owned())
                } else if let Some(context) = context_kind(node) {
                    Ok(context)
                } else {
                    bail!("dependency graph root should have a known kind")
                }
            })
            .collect::<Result<Vec<_>>>()?;
        let contexts = resolution
            .values()
            .filter_map(context_kind)
            .collect::<Vec<_>>();
        Ok((root_kinds, contexts))
    };

    let (root_kinds, contexts) = projected_contexts(0)?;
    assert_json_snapshot!(root_kinds, @r#"
    [
      "package"
    ]
    "#);
    assert_json_snapshot!(contexts, @r#"[]"#);

    let (root_kinds, contexts) = projected_contexts(1)?;
    assert_json_snapshot!(root_kinds, @r#"
    [
      "group: dev",
      "package",
      "extra: feature"
    ]
    "#);
    assert_json_snapshot!(contexts, @r#"
    [
      "group: dev",
      "extra: feature"
    ]
    "#);

    Ok(())
}

#[test]
fn json_output_virtual_root() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context.temp_dir.child("pyproject.toml").write_str(
        r#"
        [dependency-groups]
        dev = ["package-a"]

        [tool.uv.sources]
        package-a = { workspace = true }

        [tool.uv.workspace]
        members = ["package-a"]
        "#,
    )?;

    let package_a = context.temp_dir.child("package-a");
    package_a.create_dir_all()?;
    package_a.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "package-a"
        version = "1.0.0"
        requires-python = ">=3.12"
        "#,
    )?;

    uv_snapshot!(context.filters(), context.tree()
        .arg("--preview-features")
        .arg("json-output")
        .arg("--format")
        .arg("json")
        .arg("--universal")
        // A workspace-owned group's direct requirements are at depth zero, even though the JSON
        // graph represents the group itself as a root node.
        .arg("--depth")
        .arg("0")
        .arg("--only-group")
        .arg("dev"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "workspace_root": "[TEMP_DIR]/",
      "workspace": {
        "path": "[TEMP_DIR]/",
        "id": "workspace+[TEMP_DIR]/"
      },
      "roots": [
        {
          "id": "workspace+[TEMP_DIR]/:dev"
        }
      ],
      "inverted": false,
      "members": [
        {
          "name": "package-a",
          "path": "[TEMP_DIR]/package-a",
          "id": "package-a==1.0.0@editable+[TEMP_DIR]/package-a"
        }
      ],
      "resolution": {
        "package-a==1.0.0@editable+[TEMP_DIR]/package-a": {
          "name": "package-a",
          "version": "1.0.0",
          "source": {
            "editable": "[TEMP_DIR]/package-a"
          },
          "kind": "package",
          "dependencies": []
        },
        "workspace+[TEMP_DIR]/": {
          "kind": "workspace",
          "path": "[TEMP_DIR]/",
          "dependencies": [],
          "dependency_groups": [
            {
              "name": "dev",
              "id": "workspace+[TEMP_DIR]/:dev"
            }
          ]
        },
        "workspace+[TEMP_DIR]/:dev": {
          "kind": {
            "group": "dev"
          },
          "path": "[TEMP_DIR]/",
          "dependencies": [
            {
              "id": "package-a==1.0.0@editable+[TEMP_DIR]/package-a"
            }
          ]
        }
      }
    }

    ----- stderr -----
    Resolved 1 package in [TIME]
    "#);

    uv_snapshot!(context.filters(), context.tree()
        .arg("--preview-features")
        .arg("json-output")
        .arg("--format")
        .arg("json")
        .arg("--universal")
        .arg("--only-group")
        .arg("dev")
        .arg("--invert"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "workspace_root": "[TEMP_DIR]/",
      "workspace": {
        "path": "[TEMP_DIR]/",
        "id": "workspace+[TEMP_DIR]/"
      },
      "roots": [
        {
          "id": "package-a==1.0.0@editable+[TEMP_DIR]/package-a"
        }
      ],
      "inverted": true,
      "members": [
        {
          "name": "package-a",
          "path": "[TEMP_DIR]/package-a",
          "id": "package-a==1.0.0@editable+[TEMP_DIR]/package-a"
        }
      ],
      "resolution": {
        "package-a==1.0.0@editable+[TEMP_DIR]/package-a": {
          "name": "package-a",
          "version": "1.0.0",
          "source": {
            "editable": "[TEMP_DIR]/package-a"
          },
          "kind": "package",
          "dependencies": [
            {
              "id": "workspace+[TEMP_DIR]/:dev"
            }
          ]
        },
        "workspace+[TEMP_DIR]/": {
          "kind": "workspace",
          "path": "[TEMP_DIR]/",
          "dependencies": [],
          "dependency_groups": [
            {
              "name": "dev",
              "id": "workspace+[TEMP_DIR]/:dev"
            }
          ]
        },
        "workspace+[TEMP_DIR]/:dev": {
          "kind": {
            "group": "dev"
          },
          "path": "[TEMP_DIR]/",
          "dependencies": []
        }
      }
    }

    ----- stderr -----
    Resolved 1 package in [TIME]
    "#);

    Ok(())
}

#[test]
fn virtual_workspace_members() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context.temp_dir.child("pyproject.toml").write_str(
        r#"
        [tool.uv.workspace]
        members = ["packages/*"]
        "#,
    )?;

    let package_a = context.temp_dir.child("packages/package-a");
    package_a.create_dir_all()?;
    package_a.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "package-a"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["package-b"]

        [tool.uv.sources]
        package-b = { workspace = true }
        "#,
    )?;

    let package_b = context.temp_dir.child("packages/package-b");
    package_b.create_dir_all()?;
    package_b.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "package-b"
        version = "1.0.0"
        requires-python = ">=3.12"
        "#,
    )?;

    uv_snapshot!(context.filters(), context.tree().arg("--universal"), @"
    exit_code: 0 (success)
    ----- stdout -----
    package-b v1.0.0
    package-a v1.0.0
    └── package-b v1.0.0

    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context.tree()
        .arg("--preview-features")
        .arg("json-output")
        .arg("--format")
        .arg("json")
        .arg("--universal"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "workspace_root": "[TEMP_DIR]/",
      "workspace": {
        "path": "[TEMP_DIR]/",
        "id": "workspace+[TEMP_DIR]/"
      },
      "roots": [
        {
          "id": "package-a==1.0.0@virtual+[TEMP_DIR]/packages/package-a"
        },
        {
          "id": "package-b==1.0.0@editable+[TEMP_DIR]/packages/package-b"
        }
      ],
      "inverted": false,
      "members": [
        {
          "name": "package-a",
          "path": "[TEMP_DIR]/packages/package-a",
          "id": "package-a==1.0.0@virtual+[TEMP_DIR]/packages/package-a"
        },
        {
          "name": "package-b",
          "path": "[TEMP_DIR]/packages/package-b",
          "id": "package-b==1.0.0@editable+[TEMP_DIR]/packages/package-b"
        }
      ],
      "resolution": {
        "package-a==1.0.0@virtual+[TEMP_DIR]/packages/package-a": {
          "name": "package-a",
          "version": "1.0.0",
          "source": {
            "virtual": "[TEMP_DIR]/packages/package-a"
          },
          "kind": "package",
          "dependencies": [
            {
              "id": "package-b==1.0.0@editable+[TEMP_DIR]/packages/package-b"
            }
          ]
        },
        "package-b==1.0.0@editable+[TEMP_DIR]/packages/package-b": {
          "name": "package-b",
          "version": "1.0.0",
          "source": {
            "editable": "[TEMP_DIR]/packages/package-b"
          },
          "kind": "package",
          "dependencies": []
        },
        "workspace+[TEMP_DIR]/": {
          "kind": "workspace",
          "path": "[TEMP_DIR]/",
          "dependencies": []
        }
      }
    }

    ----- stderr -----
    Resolved 2 packages in [TIME]
    "#);

    Ok(())
}

#[test]
fn virtual_workspace_dependency_groups_only() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context.temp_dir.child("pyproject.toml").write_str(
        r#"
        [dependency-groups]
        dev = ["group-dependency"]

        [tool.uv.sources]
        group-dependency = { path = "group-dependency" }

        [tool.uv.workspace]
        members = []
        "#,
    )?;

    let dependency = context.temp_dir.child("group-dependency");
    dependency.create_dir_all()?;
    dependency.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "group-dependency"
        version = "1.0.0"
        requires-python = ">=3.12"
        "#,
    )?;

    uv_snapshot!(context.filters(), context.tree()
        .arg("--universal")
        .arg("--only-group")
        .arg("dev"), @"
    exit_code: 0 (success)
    ----- stdout -----
    group-dependency v1.0.0 (group: dev)

    ----- stderr -----
    warning: No `requires-python` value found in the workspace. Defaulting to `>=3.12`.
    Resolved 1 package in [TIME]
    ");

    uv_snapshot!(context.filters(), context.tree()
        .arg("--preview-features")
        .arg("json-output")
        .arg("--format")
        .arg("json")
        .arg("--universal")
        .arg("--only-group")
        .arg("dev"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "workspace_root": "[TEMP_DIR]/",
      "workspace": {
        "path": "[TEMP_DIR]/",
        "id": "workspace+[TEMP_DIR]/"
      },
      "roots": [
        {
          "id": "workspace+[TEMP_DIR]/:dev"
        }
      ],
      "inverted": false,
      "resolution": {
        "group-dependency==1.0.0@directory+[TEMP_DIR]/group-dependency": {
          "name": "group-dependency",
          "version": "1.0.0",
          "source": {
            "directory": "[TEMP_DIR]/group-dependency"
          },
          "kind": "package",
          "dependencies": []
        },
        "workspace+[TEMP_DIR]/": {
          "kind": "workspace",
          "path": "[TEMP_DIR]/",
          "dependencies": [],
          "dependency_groups": [
            {
              "name": "dev",
              "id": "workspace+[TEMP_DIR]/:dev"
            }
          ]
        },
        "workspace+[TEMP_DIR]/:dev": {
          "kind": {
            "group": "dev"
          },
          "path": "[TEMP_DIR]/",
          "dependencies": [
            {
              "id": "group-dependency==1.0.0@directory+[TEMP_DIR]/group-dependency"
            }
          ]
        }
      }
    }

    ----- stderr -----
    warning: No `requires-python` value found in the workspace. Defaulting to `>=3.12`.
    Resolved 1 package in [TIME]
    "#);

    Ok(())
}

#[test]
fn json_output_frozen_missing_members() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context.temp_dir.child("pyproject.toml").write_str(
        r#"
        [tool.uv.workspace]
        members = ["generated/members/*"]
        "#,
    )?;

    let generated = context.temp_dir.child("generated");
    let app = generated.child("members/app");
    app.create_dir_all()?;
    app.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "app"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["bridge"]

        [tool.uv.sources]
        bridge = { path = "../../bridge" }
        "#,
    )?;

    let bridge = generated.child("bridge");
    bridge.create_dir_all()?;
    bridge.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "bridge"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["target"]

        [tool.uv.sources]
        target = { path = "../target" }
        "#,
    )?;

    let target = generated.child("target");
    target.create_dir_all()?;
    target.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "target"
        version = "1.0.0"
        requires-python = ">=3.12"
        "#,
    )?;

    context.lock().assert().success();
    fs_err::remove_dir_all(generated.path())?;

    uv_snapshot!(context.filters(), context.tree()
        .arg("--frozen")
        .arg("--universal")
        .arg("--invert")
        .arg("--package")
        .arg("target")
        .arg("--preview-features")
        .arg("json-output")
        .arg("--format")
        .arg("json"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "workspace_root": "[TEMP_DIR]/",
      "workspace": {
        "path": "[TEMP_DIR]/",
        "id": "workspace+[TEMP_DIR]/"
      },
      "roots": [
        {
          "id": "target==1.0.0@directory+[TEMP_DIR]/generated/target"
        }
      ],
      "inverted": true,
      "members": [
        {
          "name": "app",
          "path": "[TEMP_DIR]/generated/members/app",
          "id": "app==1.0.0@virtual+[TEMP_DIR]/generated/members/app"
        }
      ],
      "resolution": {
        "app==1.0.0@virtual+[TEMP_DIR]/generated/members/app": {
          "name": "app",
          "version": "1.0.0",
          "source": {
            "virtual": "[TEMP_DIR]/generated/members/app"
          },
          "kind": "package",
          "dependencies": []
        },
        "bridge==1.0.0@directory+[TEMP_DIR]/generated/bridge": {
          "name": "bridge",
          "version": "1.0.0",
          "source": {
            "directory": "[TEMP_DIR]/generated/bridge"
          },
          "kind": "package",
          "dependencies": [
            {
              "id": "app==1.0.0@virtual+[TEMP_DIR]/generated/members/app"
            }
          ]
        },
        "target==1.0.0@directory+[TEMP_DIR]/generated/target": {
          "name": "target",
          "version": "1.0.0",
          "source": {
            "directory": "[TEMP_DIR]/generated/target"
          },
          "kind": "package",
          "dependencies": [
            {
              "id": "bridge==1.0.0@directory+[TEMP_DIR]/generated/bridge"
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
    "#);

    Ok(())
}

#[test]
fn json_output_depth_with_extra_context() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context.temp_dir.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["package-a", "package-c"]

        [tool.uv.sources]
        package-a = { path = "packages/package-a" }
        package-c = { path = "packages/package-c" }
        "#,
    )?;

    let package_a = context.temp_dir.child("packages/package-a");
    package_a.create_dir_all()?;
    package_a.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "package-a"
        version = "1.0.0"
        requires-python = ">=3.12"

        [project.optional-dependencies]
        feature = ["package-b"]

        [tool.uv.sources]
        package-b = { path = "../package-b" }
        "#,
    )?;

    let package_c = context.temp_dir.child("packages/package-c");
    package_c.create_dir_all()?;
    package_c.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "package-c"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["package-a[feature]"]

        [tool.uv.sources]
        package-a = { path = "../package-a" }
        "#,
    )?;

    let package_b = context.temp_dir.child("packages/package-b");
    package_b.create_dir_all()?;
    package_b.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "package-b"
        version = "1.0.0"
        requires-python = ">=3.12"
        "#,
    )?;

    let package_names = |depth: u8| -> Result<Vec<String>> {
        let output = context
            .tree()
            .arg("--preview-features")
            .arg("json-output")
            .arg("--format")
            .arg("json")
            .arg("--universal")
            .arg("--depth")
            .arg(depth.to_string())
            .output()?;
        output.clone().assert().success();
        let report: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        report["resolution"]
            .as_object()
            .context("dependency graph resolution should be an object")?
            .values()
            .filter(|node| node["kind"] == "package")
            .map(|node| {
                node["name"]
                    .as_str()
                    .context("dependency graph node should have a name")
                    .map(ToOwned::to_owned)
            })
            .collect()
    };

    assert_json_snapshot!(package_names(2)?, @r#"
    [
      "package-a",
      "package-c",
      "project"
    ]
    "#);
    assert_json_snapshot!(package_names(3)?, @r#"
    [
      "package-a",
      "package-b",
      "package-c",
      "project"
    ]
    "#);

    Ok(())
}

#[test]
fn nested_platform_dependencies() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "platform-parent"
        ]
    "#,
    )?;

    uv_snapshot!(context.filters(), context.tree().arg("--python-platform").arg("linux"), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    └── platform-parent v24.3.0
        ├── platform-leaf v1.0.0
        └── platform-marker v8.1.7

    ----- stderr -----
    Resolved 5 packages in [TIME]
    "
    );

    uv_snapshot!(context.filters(), context.tree().arg("--universal"), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    └── platform-parent v24.3.0
        ├── platform-leaf v1.0.0
        └── platform-marker v8.1.7
            └── platform-windows v0.4.6

    ----- stderr -----
    Resolved 5 packages in [TIME]
    "
    );

    // `uv tree` should update the lockfile
    let lock = context.read("uv.lock");
    assert!(!lock.is_empty());

    Ok(())
}

#[test]
fn invert() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "tree-root"
        ]
    "#,
    )?;

    uv_snapshot!(context.filters(), context.tree().arg("--invert"), @"
    exit_code: 0 (success)
    ----- stdout -----
    tree-branch-a v1.7.0
    └── tree-root v3.0.2
        └── project v0.1.0
    tree-branch-b v8.1.7
    └── tree-root v3.0.2 (*)
    tree-branch-c v2.1.2
    └── tree-root v3.0.2 (*)
    tree-shared-leaf v2.1.5
    ├── tree-branch-d v3.1.3
    │   └── tree-root v3.0.2 (*)
    └── tree-branch-e v3.0.1
        └── tree-root v3.0.2 (*)
    (*) Package tree already displayed

    ----- stderr -----
    Resolved 8 packages in [TIME]
    "
    );

    uv_snapshot!(context.filters(), context.tree().arg("--invert").arg("--no-dedupe"), @"
    exit_code: 0 (success)
    ----- stdout -----
    tree-branch-a v1.7.0
    └── tree-root v3.0.2
        └── project v0.1.0
    tree-branch-b v8.1.7
    └── tree-root v3.0.2
        └── project v0.1.0
    tree-branch-c v2.1.2
    └── tree-root v3.0.2
        └── project v0.1.0
    tree-shared-leaf v2.1.5
    ├── tree-branch-d v3.1.3
    │   └── tree-root v3.0.2
    │       └── project v0.1.0
    └── tree-branch-e v3.0.1
        └── tree-root v3.0.2
            └── project v0.1.0

    ----- stderr -----
    Resolved 8 packages in [TIME]
    "
    );

    Ok(())
}

#[cfg(feature = "test-universal")]
#[test]
fn frozen() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["outdated-package"]
    "#,
    )?;

    uv_snapshot!(context.filters(), context.tree().arg("--universal"), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    └── outdated-package v4.3.0

    ----- stderr -----
    Resolved 2 packages in [TIME]
    "
    );

    // `uv tree` should update the lockfile
    let lock = context.read("uv.lock");
    assert!(!lock.is_empty());

    // Update the project dependencies.
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["simple-package"]
    "#,
    )?;

    // Running with `--frozen` should show the stale tree.
    uv_snapshot!(context.filters(), context.tree().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    └── outdated-package v4.3.0
    "
    );

    Ok(())
}

#[cfg(feature = "test-universal")]
#[test]
fn outdated() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["outdated-package==3.0.0"]
    "#,
    )?;

    uv_snapshot!(context.filters(), context.tree().arg("--outdated").arg("--universal"), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    └── outdated-package v3.0.0 (latest: v4.3.0)

    ----- stderr -----
    Resolved 2 packages in [TIME]
    "
    );

    let output = context
        .tree()
        .arg("--preview-features")
        .arg("json-output")
        .arg("--format")
        .arg("json")
        .arg("--outdated")
        .arg("--universal")
        .output()?;
    output.clone().assert().success();
    let report: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let package = report["resolution"]
        .as_object()
        .and_then(|resolution| {
            resolution
                .values()
                .find(|node| node["name"] == "outdated-package")
        })
        .expect("outdated-package should be included in the dependency graph");
    assert_eq!(package["latest_version"], "4.3.0");

    Ok(())
}

/// Test that `uv tree --outdated` with a relative `exclude-newer` span recomputes the
/// cutoff timestamp relative to the current time, not the time the lock was generated.
///
/// Uses idna which has releases at:
/// - 3.6: 2023-11-25
/// - 3.7: 2024-04-11
#[cfg(feature = "test-universal")]
#[test]
fn outdated_exclude_newer_relative() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["idna"]
    "#,
    )?;

    // Lock at 2024-05-01 with `--exclude-newer "3 weeks"`.
    // Cutoff: 2024-04-10 → resolves idna 3.6 (released 2023-11-25, before cutoff).
    // idna 3.7 (released 2024-04-11) is excluded.
    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, "2024-05-01T00:00:00Z")
        .arg("--exclude-newer")
        .arg("3 weeks"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");

    // Run `--outdated` at a later time (2024-06-01) with the same span.
    // The recomputed cutoff is 2024-05-11, which is after idna 3.7 (2024-04-11),
    // so idna 3.7 should be reported as the latest version.
    uv_snapshot!(context.filters(), context
        .tree()
        .arg("--outdated")
        .arg("--universal")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, "2024-06-01T00:00:00Z")
        .arg("--exclude-newer")
        .arg("3 weeks"), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    └── idna v3.6

    ----- stderr -----
    Resolved 2 packages in [TIME]
    "
    );

    Ok(())
}

/// Exclude a dependency only when it is declared by a matching package version.
#[test]
fn scoped_exclude_dependencies() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["anyio==3.7.0", "sniffio==1.3.1"]

        [tool.uv]
        exclude-dependencies = [
            { package = { name = "anyio" }, dependencies = ["idna"] },
            { package = { name = "anyio", version = "3.7.0" }, dependencies = ["sniffio"] },
        ]
        "#,
    )?;

    context.lock().assert().success();

    // The structured exclusion is persisted in the lockfile manifest.
    uv_snapshot!(context.filters(), context.lock().arg("--locked"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");

    // The exact-version entry takes precedence over the all-versions entry, removing AnyIO's edge
    // to Sniffio without removing the direct requirement while retaining the edge to IDNA.
    uv_snapshot!(context.filters(), context.tree(), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    ├── anyio v3.7.0
    │   └── idna v3.6
    └── sniffio v1.3.1

    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");

    // The version-gated exclusion is ignored when the parent version does not match.
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["anyio==3.7.0", "sniffio==1.3.1"]

        [tool.uv]
        exclude-dependencies = [
            { package = { name = "anyio", version = "3.6.2" }, dependencies = ["sniffio"] },
        ]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.tree(), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    ├── anyio v3.7.0
    │   ├── idna v3.6
    │   └── sniffio v1.3.1
    └── sniffio v1.3.1

    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");

    Ok(())
}

#[cfg(feature = "test-universal")]
#[test]
fn platform_dependencies() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "platform-parent"
        ]
    "#,
    )?;

    // When `--universal` is _not_ provided, `colorama` should _not_ be included.
    #[cfg(not(windows))]
    uv_snapshot!(context.filters(), context.tree(), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    └── platform-parent v24.3.0
        ├── platform-leaf v1.0.0
        └── platform-marker v8.1.7

    ----- stderr -----
    Resolved 5 packages in [TIME]
    ");

    // Unless `--python-platform` is set to `windows`, in which case it should be included.
    uv_snapshot!(context.filters(), context.tree().arg("--python-platform").arg("windows"), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    └── platform-parent v24.3.0
        ├── platform-leaf v1.0.0
        └── platform-marker v8.1.7
            └── platform-windows v0.4.6

    ----- stderr -----
    Resolved 5 packages in [TIME]
    ");

    // When `--universal` is provided, should include `colorama`, even though it's only included on
    // Windows.
    uv_snapshot!(context.filters(), context.tree().arg("--universal"), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    └── platform-parent v24.3.0
        ├── platform-leaf v1.0.0
        └── platform-marker v8.1.7
            └── platform-windows v0.4.6

    ----- stderr -----
    Resolved 5 packages in [TIME]
    "
    );

    // `uv tree` should update the lockfile
    let lock = context.read("uv.lock");
    assert!(!lock.is_empty());

    Ok(())
}

#[test]
fn platform_dependencies_inverted() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "platform-marker"
        ]
    "#,
    )?;

    // When `--universal` is _not_ provided, `colorama` should _not_ be included.
    uv_snapshot!(context.filters(), context.tree().arg("--invert").arg("--python-platform").arg("linux"), @"
    exit_code: 0 (success)
    ----- stdout -----
    platform-marker v8.1.7
    └── project v0.1.0

    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");

    // Unless `--python-platform` is set to `windows`, in which case it should be included.
    uv_snapshot!(context.filters(), context.tree().arg("--invert").arg("--python-platform").arg("windows"), @"
    exit_code: 0 (success)
    ----- stdout -----
    platform-windows v0.4.6
    └── platform-marker v8.1.7
        └── project v0.1.0

    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");

    Ok(())
}

#[cfg(feature = "test-universal")]
#[test]
fn repeated_dependencies() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "outdated-package < 2 ; sys_platform == 'win32'",
            "outdated-package > 2 ; sys_platform == 'linux'",
        ]
    "#,
    )?;

    // Should include both versions of `outdated-package`, which have different dependencies.
    uv_snapshot!(context.filters(), context.tree().arg("--universal"), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    ├── outdated-package v1.4.0
    └── outdated-package v4.3.0

    ----- stderr -----
    Resolved 3 packages in [TIME]
    "
    );

    let mut projected_edges = Vec::new();
    for invert in [false, true] {
        let mut command = context.tree();
        command
            .arg("--preview-features")
            .arg("json-output")
            .arg("--format")
            .arg("json")
            .arg("--universal");
        if invert {
            command.arg("--invert");
        }
        let output = command.output()?;
        output.clone().assert().success();
        let report: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        let resolution = report["resolution"]
            .as_object()
            .context("dependency graph resolution should be an object")?;
        let project_edges = if invert {
            resolution
                .iter()
                .filter(|(_, node)| node["name"] == "outdated-package" && node["kind"] == "package")
                .flat_map(|(package, node)| {
                    node["dependencies"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|dependency| {
                            dependency["id"]
                                .as_str()
                                .is_some_and(|id| id.starts_with("project=="))
                        })
                        .map(move |dependency| {
                            serde_json::json!({
                                "package": package,
                                "marker": dependency["marker"],
                            })
                        })
                })
                .collect::<Vec<_>>()
        } else {
            resolution
                .values()
                .find(|node| node["name"] == "project" && node["kind"] == "package")
                .and_then(|node| node["dependencies"].as_array())
                .context("project should have dependency graph edges")?
                .iter()
                .map(|dependency| {
                    serde_json::json!({
                        "package": dependency["id"],
                        "marker": dependency["marker"],
                    })
                })
                .collect::<Vec<_>>()
        };

        projected_edges.push(serde_json::json!({
            "inverted": invert,
            "edges": project_edges,
        }));
    }
    insta::with_settings!({ filters => context.filters() }, {
    assert_json_snapshot!(projected_edges, @r#"
    [
      {
        "edges": [
          {
            "marker": "sys_platform == 'win32'",
            "package": "outdated-package==1.4.0@registry+http://[LOCALHOST]/simple/"
          },
          {
            "marker": "sys_platform == 'linux'",
            "package": "outdated-package==4.3.0@registry+http://[LOCALHOST]/simple/"
          }
        ],
        "inverted": false
      },
      {
        "edges": [
          {
            "marker": "sys_platform == 'win32'",
            "package": "outdated-package==1.4.0@registry+http://[LOCALHOST]/simple/"
          },
          {
            "marker": "sys_platform == 'linux'",
            "package": "outdated-package==4.3.0@registry+http://[LOCALHOST]/simple/"
          }
        ],
        "inverted": true
      }
    ]
    "#);
    });

    // `uv tree` should update the lockfile
    let lock = context.read("uv.lock");
    assert!(!lock.is_empty());

    Ok(())
}

/// In this case, a package is included twice at the same version, but pointing to different direct
/// URLs.
#[cfg(feature = "test-universal")]
#[test]
fn repeated_version() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let v1 = context.temp_dir.child("v1");
    fs_err::create_dir_all(&v1)?;
    let pyproject_toml = v1.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "dependency"
        version = "0.0.1"
        requires-python = ">=3.12"
        dependencies = ["outdated-package==3.7.0"]
        "#,
    )?;

    let v2 = context.temp_dir.child("v2");
    fs_err::create_dir_all(&v2)?;
    let pyproject_toml = v2.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "dependency"
        version = "0.0.1"
        requires-python = ">=3.12"
        dependencies = ["outdated-package==3.0.0"]
        "#,
    )?;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&formatdoc! {
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
          "dependency @ {} ; sys_platform == 'darwin'",
          "dependency @ {} ; sys_platform != 'darwin'",
        ]
        "#,
        Url::from_file_path(context.temp_dir.join("v1")).unwrap(),
        Url::from_file_path(context.temp_dir.join("v2")).unwrap(),
    })?;

    uv_snapshot!(context.filters(), context.tree().arg("--universal"), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    ├── dependency v0.0.1
    │   └── outdated-package v3.7.0
    └── dependency v0.0.1
        └── outdated-package v3.0.0

    ----- stderr -----
    Resolved 5 packages in [TIME]
    "
    );

    // `uv tree` should update the lockfile
    let lock = context.read("uv.lock");
    assert!(!lock.is_empty());

    Ok(())
}

#[test]
fn dev_dependencies() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["simple-package"]

        [tool.uv]
        dev-dependencies = ["outdated-package"]
    "#,
    )?;

    uv_snapshot!(context.filters(), context.tree(), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    ├── simple-package v2.1.3
    └── outdated-package v4.3.0 (group: dev)

    ----- stderr -----
    warning: The `tool.uv.dev-dependencies` field (used in `pyproject.toml`) is deprecated and will be removed in a future release; use `dependency-groups.dev` instead
    Resolved 3 packages in [TIME]
    "
    );

    uv_snapshot!(context.filters(), context.tree().arg("--no-dev"), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    └── simple-package v2.1.3

    ----- stderr -----
    warning: The `tool.uv.dev-dependencies` field (used in `pyproject.toml`) is deprecated and will be removed in a future release; use `dependency-groups.dev` instead
    Resolved 3 packages in [TIME]
    "
    );

    // `uv tree` should update the lockfile
    let lock = context.read("uv.lock");
    assert!(!lock.is_empty());

    Ok(())
}

#[cfg(feature = "test-universal")]
#[test]
fn dev_dependencies_inverted() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["simple-package"]

        [tool.uv]
        dev-dependencies = ["outdated-package"]
    "#,
    )?;

    uv_snapshot!(context.filters(), context.tree().arg("--universal").arg("--invert"), @"
    exit_code: 0 (success)
    ----- stdout -----
    outdated-package v4.3.0
    └── project v0.1.0 (group: dev)
    simple-package v2.1.3
    └── project v0.1.0

    ----- stderr -----
    warning: The `tool.uv.dev-dependencies` field (used in `pyproject.toml`) is deprecated and will be removed in a future release; use `dependency-groups.dev` instead
    Resolved 3 packages in [TIME]
    "
    );

    uv_snapshot!(context.filters(), context.tree().arg("--universal").arg("--invert").arg("--no-dev"), @"
    exit_code: 0 (success)
    ----- stdout -----
    simple-package v2.1.3
    └── project v0.1.0

    ----- stderr -----
    warning: The `tool.uv.dev-dependencies` field (used in `pyproject.toml`) is deprecated and will be removed in a future release; use `dependency-groups.dev` instead
    Resolved 3 packages in [TIME]
    "
    );

    // `uv tree` should update the lockfile
    let lock = context.read("uv.lock");
    assert!(!lock.is_empty());

    Ok(())
}

#[cfg(feature = "test-universal")]
#[test]
fn optional_dependencies() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["simple-package", "tree-root[dotenv]"]

        [project.optional-dependencies]
        async = ["outdated-package"]
    "#,
    )?;

    uv_snapshot!(context.filters(), context.tree().arg("--universal"), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    ├── simple-package v2.1.3
    ├── tree-root[dotenv] v3.0.2
    │   ├── tree-branch-a v1.7.0
    │   ├── tree-branch-b v8.1.7
    │   ├── tree-branch-c v2.1.2
    │   ├── tree-branch-d v3.1.3
    │   │   └── tree-shared-leaf v2.1.5
    │   ├── tree-branch-e v3.0.1
    │   │   └── tree-shared-leaf v2.1.5
    │   └── tree-extra v1.0.1 (extra: dotenv)
    └── outdated-package v4.3.0 (extra: async)

    ----- stderr -----
    Resolved 11 packages in [TIME]
    "
    );

    // `uv tree` should update the lockfile
    let lock = context.read("uv.lock");
    assert!(!lock.is_empty());

    Ok(())
}

#[cfg(feature = "test-universal")]
#[test]
fn optional_dependencies_inverted() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["simple-package", "tree-root[dotenv]"]

        [project.optional-dependencies]
        async = ["outdated-package"]
    "#,
    )?;

    uv_snapshot!(context.filters(), context.tree().arg("--universal").arg("--invert"), @"
    exit_code: 0 (success)
    ----- stdout -----
    outdated-package v4.3.0
    └── project v0.1.0 (extra: async)
    simple-package v2.1.3
    └── project v0.1.0
    tree-branch-a v1.7.0
    └── tree-root v3.0.2
        └── project[dotenv] v0.1.0
    tree-branch-b v8.1.7
    └── tree-root v3.0.2 (*)
    tree-branch-c v2.1.2
    └── tree-root v3.0.2 (*)
    tree-extra v1.0.1
    └── tree-root v3.0.2 (extra: dotenv)
        └── project[dotenv] v0.1.0
    tree-shared-leaf v2.1.5
    ├── tree-branch-d v3.1.3
    │   └── tree-root v3.0.2 (*)
    └── tree-branch-e v3.0.1
        └── tree-root v3.0.2 (*)
    (*) Package tree already displayed

    ----- stderr -----
    Resolved 11 packages in [TIME]
    "
    );

    // `uv tree` should update the lockfile
    let lock = context.read("uv.lock");
    assert!(!lock.is_empty());

    Ok(())
}

/// Regression test for <https://github.com/astral-sh/uv/issues/19327>.
///
/// When a package is required both as a plain dep and as a dep with extras (e.g., from a
/// dependency group), `uv tree` should not display extra-conditional dependencies for the plain
/// occurrence.
#[cfg(feature = "test-universal")]
#[test]
fn dep_and_group_extras() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["tree-root"]

        [dependency-groups]
        dev = ["tree-root[dotenv]"]
    "#,
    )?;

    // Plain `tree-root` should not show `python-dotenv` (which belongs to the `dotenv` extra),
    // but the `tree-root[dotenv]` occurrence should still be expanded in its own extra context.
    uv_snapshot!(context.filters(), context.tree().arg("--universal"), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    ├── tree-root v3.0.2
    │   ├── tree-branch-a v1.7.0
    │   ├── tree-branch-b v8.1.7
    │   ├── tree-branch-c v2.1.2
    │   ├── tree-branch-d v3.1.3
    │   │   └── tree-shared-leaf v2.1.5
    │   └── tree-branch-e v3.0.1
    │       └── tree-shared-leaf v2.1.5
    └── tree-root[dotenv] v3.0.2 (group: dev)
        ├── tree-branch-a v1.7.0
        ├── tree-branch-b v8.1.7
        ├── tree-branch-c v2.1.2
        ├── tree-branch-d v3.1.3 (*)
        ├── tree-branch-e v3.0.1 (*)
        └── tree-extra v1.0.1 (extra: dotenv)
    (*) Package tree already displayed

    ----- stderr -----
    Resolved 9 packages in [TIME]
    "
    );

    // With `--no-dedupe`, `tree-root[dotenv]` is expanded and shows `python-dotenv` as an extra dep,
    // while plain `tree-root` still does not show it.
    uv_snapshot!(context.filters(), context.tree().arg("--universal").arg("--no-dedupe"), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    ├── tree-root v3.0.2
    │   ├── tree-branch-a v1.7.0
    │   ├── tree-branch-b v8.1.7
    │   ├── tree-branch-c v2.1.2
    │   ├── tree-branch-d v3.1.3
    │   │   └── tree-shared-leaf v2.1.5
    │   └── tree-branch-e v3.0.1
    │       └── tree-shared-leaf v2.1.5
    └── tree-root[dotenv] v3.0.2 (group: dev)
        ├── tree-branch-a v1.7.0
        ├── tree-branch-b v8.1.7
        ├── tree-branch-c v2.1.2
        ├── tree-branch-d v3.1.3
        │   └── tree-shared-leaf v2.1.5
        ├── tree-branch-e v3.0.1
        │   └── tree-shared-leaf v2.1.5
        └── tree-extra v1.0.1 (extra: dotenv)

    ----- stderr -----
    Resolved 9 packages in [TIME]
    "
    );

    Ok(())
}

#[test]
fn dep_and_group_extras_with_extra_only_dependency() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let leaf = context.temp_dir.child("leaf");
    fs_err::create_dir_all(leaf.path())?;
    leaf.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "leaf"
        version = "0.1.0"
        requires-python = ">=3.12"
        "#,
    )?;
    let leaf_url = Url::from_file_path(leaf.path())
        .map_err(|()| anyhow::anyhow!("failed to convert leaf path to URL"))?;

    let child = context.temp_dir.child("child");
    fs_err::create_dir_all(child.path())?;
    child.child("pyproject.toml").write_str(&formatdoc! {
        r#"
        [project]
        name = "child"
        version = "0.1.0"
        requires-python = ">=3.12"

        [project.optional-dependencies]
        extra = ["leaf @ {}"]
        "#,
        leaf_url,
    })?;
    let child_url = Url::from_file_path(child.path())
        .map_err(|()| anyhow::anyhow!("failed to convert child path to URL"))?;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&formatdoc! {
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["child @ {}"]

        [dependency-groups]
        dev = ["child[extra] @ {}"]
        "#,
        child_url,
        child_url,
    })?;

    uv_snapshot!(context.filters(), context.tree(), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    ├── child v0.1.0
    └── child[extra] v0.1.0 (group: dev)
        └── leaf v0.1.0 (extra: extra)

    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");

    Ok(())
}

#[test]
fn dep_and_group_extras_with_different_extras_in_path() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let leaf = context.temp_dir.child("leaf");
    fs_err::create_dir_all(leaf.path())?;
    leaf.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "leaf"
        version = "0.1.0"
        requires-python = ">=3.12"
        "#,
    )?;
    let leaf_url = Url::from_file_path(leaf.path())
        .map_err(|()| anyhow::anyhow!("failed to convert leaf path to URL"))?;

    let a = context.temp_dir.child("a");
    fs_err::create_dir_all(a.path())?;
    let b = context.temp_dir.child("b");
    fs_err::create_dir_all(b.path())?;
    let b_url = Url::from_file_path(b.path())
        .map_err(|()| anyhow::anyhow!("failed to convert b path to URL"))?;
    a.child("pyproject.toml").write_str(&formatdoc! {
        r#"
        [project]
        name = "a"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["b[extra2] @ {}"]
        "#,
        b_url,
    })?;
    let a_url = Url::from_file_path(a.path())
        .map_err(|()| anyhow::anyhow!("failed to convert a path to URL"))?;

    b.child("pyproject.toml").write_str(&formatdoc! {
        r#"
        [project]
        name = "b"
        version = "0.1.0"
        requires-python = ">=3.12"

        [project.optional-dependencies]
        extra1 = ["a @ {}"]
        extra2 = ["leaf @ {}"]
        "#,
        a_url,
        leaf_url,
    })?;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&formatdoc! {
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["b[extra1] @ {}"]
        "#,
        b_url,
    })?;

    uv_snapshot!(context.filters(), context.tree(), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    └── b[extra1] v0.1.0
        └── a v0.1.0 (extra: extra1)
            └── b[extra2] v0.1.0
                └── leaf v0.1.0 (extra: extra2)

    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");

    Ok(())
}

#[test]
fn package() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["tree-root", "shared-root"]
    "#,
    )?;

    uv_snapshot!(context.filters(), context.tree(), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    ├── shared-root v3.0.0
    │   ├── shared-branch v2.14.1
    │   │   └── shared-leaf v2.9.0
    │   │       └── shared-bottom v1.16.0
    │   ├── shared-extra v2024.1
    │   └── shared-leaf v2.9.0 (*)
    └── tree-root v3.0.2
        ├── tree-branch-a v1.7.0
        ├── tree-branch-b v8.1.7
        ├── tree-branch-c v2.1.2
        ├── tree-branch-d v3.1.3
        │   └── tree-shared-leaf v2.1.5
        └── tree-branch-e v3.0.1
            └── tree-shared-leaf v2.1.5
    (*) Package tree already displayed

    ----- stderr -----
    Resolved 13 packages in [TIME]
    "
    );

    uv_snapshot!(context.filters(), context.tree().arg("--package").arg("tree-branch-d"), @"
    exit_code: 0 (success)
    ----- stdout -----
    tree-branch-d v3.1.3
    └── tree-shared-leaf v2.1.5

    ----- stderr -----
    Resolved 13 packages in [TIME]
    "
    );

    uv_snapshot!(context.filters(), context.tree().arg("--package").arg("tree-shared-leaf").arg("--invert"), @"
    exit_code: 0 (success)
    ----- stdout -----
    tree-shared-leaf v2.1.5
    ├── tree-branch-d v3.1.3
    │   └── tree-root v3.0.2
    │       └── project v0.1.0
    └── tree-branch-e v3.0.1
        └── tree-root v3.0.2 (*)
    (*) Package tree already displayed

    ----- stderr -----
    Resolved 13 packages in [TIME]
    "
    );

    // `uv tree` should update the lockfile
    let lock = context.read("uv.lock");
    assert!(!lock.is_empty());

    Ok(())
}

#[test]
fn group() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["tree-leaf-b"]

        [dependency-groups]
        foo = ["outdated-package"]
        bar = ["simple-package"]
        dev = ["tree-leaf-c"]
        "#,
    )?;

    context.lock().assert().success();

    uv_snapshot!(context.filters(), context.tree(), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    ├── tree-leaf-b v3.3.2
    └── tree-leaf-c v3.6 (group: dev)

    ----- stderr -----
    Resolved 5 packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context.tree().arg("--only-group").arg("bar"), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    └── simple-package v2.1.3 (group: bar)

    ----- stderr -----
    Resolved 5 packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context.tree().arg("--group").arg("foo"), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    ├── tree-leaf-b v3.3.2
    ├── tree-leaf-c v3.6 (group: dev)
    └── outdated-package v4.3.0 (group: foo)

    ----- stderr -----
    Resolved 5 packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context.tree().arg("--group").arg("foo").arg("--group").arg("bar"), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    ├── tree-leaf-b v3.3.2
    ├── simple-package v2.1.3 (group: bar)
    ├── tree-leaf-c v3.6 (group: dev)
    └── outdated-package v4.3.0 (group: foo)

    ----- stderr -----
    Resolved 5 packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context.tree().arg("--all-groups"), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    ├── tree-leaf-b v3.3.2
    ├── simple-package v2.1.3 (group: bar)
    ├── tree-leaf-c v3.6 (group: dev)
    └── outdated-package v4.3.0 (group: foo)

    ----- stderr -----
    Resolved 5 packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context.tree().arg("--all-groups").arg("--no-group").arg("bar"), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    ├── tree-leaf-b v3.3.2
    ├── tree-leaf-c v3.6 (group: dev)
    └── outdated-package v4.3.0 (group: foo)

    ----- stderr -----
    Resolved 5 packages in [TIME]
    ");

    Ok(())
}

#[cfg(feature = "test-universal")]
#[test]
fn cycle() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["cycle-root==2.3.0", "cycle-backref==3.0.0"]
    "#,
    )?;

    uv_snapshot!(context.filters(), context.tree().arg("--universal"), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    ├── cycle-backref v3.0.0
    │   ├── cycle-leaf-c v1.16.0
    │   └── cycle-root v2.3.0
    │       ├── cycle-backref v3.0.0 (*)
    │       ├── cycle-leaf-a v1.0.0
    │       ├── cycle-leaf-b v6.0.0
    │       └── cycle-nested v1.1.0
    │           ├── cycle-leaf-c v1.16.0
    │           ├── cycle-leaf-d v1.4.0
    │           └── cycle-trace v1.4.0
    │               └── cycle-leaf-e v1.0.0
    └── cycle-root v2.3.0 (*)
    (*) Package tree already displayed

    ----- stderr -----
    Resolved 10 packages in [TIME]
    "
    );

    uv_snapshot!(context.filters(), context.tree().arg("--package").arg("cycle-trace").arg("--package").arg("cycle-leaf-c"), @"
    exit_code: 0 (success)
    ----- stdout -----
    cycle-leaf-c v1.16.0
    cycle-trace v1.4.0
    └── cycle-leaf-e v1.0.0

    ----- stderr -----
    Resolved 10 packages in [TIME]
    "
    );

    uv_snapshot!(context.filters(), context.tree().arg("--package").arg("cycle-trace").arg("--package").arg("cycle-leaf-c").arg("--invert"), @"
    exit_code: 0 (success)
    ----- stdout -----
    cycle-leaf-c v1.16.0
    ├── cycle-backref v3.0.0
    │   ├── cycle-root v2.3.0
    │   │   ├── cycle-backref v3.0.0 (*)
    │   │   └── project v0.1.0
    │   └── project v0.1.0
    └── cycle-nested v1.1.0
        └── cycle-root v2.3.0 (*)
    cycle-trace v1.4.0
    └── cycle-nested v1.1.0 (*)
    (*) Package tree already displayed

    ----- stderr -----
    Resolved 10 packages in [TIME]
    "
    );

    // `uv tree` should update the lockfile
    let lock = context.read("uv.lock");
    assert!(!lock.is_empty());

    Ok(())
}

#[cfg(feature = "test-universal")]
#[test]
fn cycle_no_orphaned_roots() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["cycle-root==2.3.0", "cycle-backref==3.0.0"]
    "#,
    )?;

    // With --depth 1, only "project" should appear as a root — transitive deps
    // involved in cycles (e.g. cycle-root <-> cycle-backref) must not be promoted to roots.
    uv_snapshot!(context.filters(), context.tree().arg("--universal").arg("--depth").arg("1"), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    ├── cycle-backref v3.0.0
    └── cycle-root v2.3.0

    ----- stderr -----
    Resolved 10 packages in [TIME]
    ");

    Ok(())
}

#[cfg(feature = "test-universal")]
#[test]
fn cycle_no_infinite_loop() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["cycle-root==2.3.0", "cycle-backref==3.0.0"]
    "#,
    )?;

    // This should complete without hanging, and cycles should be marked with (*)
    uv_snapshot!(context.filters(), context.tree().arg("--universal").arg("--depth").arg("2"), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    ├── cycle-backref v3.0.0
    │   ├── cycle-leaf-c v1.16.0
    │   └── cycle-root v2.3.0
    └── cycle-root v2.3.0
        ├── cycle-backref v3.0.0 (*)
        ├── cycle-leaf-a v1.0.0
        ├── cycle-leaf-b v6.0.0
        └── cycle-nested v1.1.0
    (*) Package tree already displayed

    ----- stderr -----
    Resolved 10 packages in [TIME]
    "
    );

    Ok(())
}

#[cfg(feature = "test-universal")]
#[test]
fn cycle_invert() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["cycle-root==2.3.0", "cycle-backref==3.0.0"]
    "#,
    )?;

    // With --invert, leaf packages should be roots and the tree should show
    // reverse dependencies without orphaned roots from cycle-breaking.
    uv_snapshot!(context.filters(), context.tree().arg("--universal").arg("--invert").arg("--depth").arg("1"), @"
    exit_code: 0 (success)
    ----- stdout -----
    cycle-leaf-a v1.0.0
    └── cycle-root v2.3.0
    cycle-leaf-b v6.0.0
    └── cycle-root v2.3.0
    cycle-leaf-c v1.16.0
    ├── cycle-backref v3.0.0
    └── cycle-nested v1.1.0
    cycle-leaf-d v1.4.0
    └── cycle-nested v1.1.0
    cycle-leaf-e v1.0.0
    └── cycle-trace v1.4.0

    ----- stderr -----
    Resolved 10 packages in [TIME]
    ");

    Ok(())
}

#[cfg(feature = "test-universal")]
#[test]
fn cycle_invert_leaf() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    setup_leaf_cycle(&context, false)?;

    uv_snapshot!(context.filters(), context.tree().arg("--frozen").arg("--invert"), @"
    exit_code: 0 (success)
    ----- stdout -----
    alpha v1.0.0
    ├── beta v1.0.0
    │   └── alpha v1.0.0
    │       ├── beta v1.0.0 (*)
    │       └── project v1.0.0
    └── project v1.0.0
    (*) Package tree already displayed
    ");

    assert_json_snapshot!(
        json_tree_package_names(context.tree().arg("--frozen").arg("--invert"))?,
        @r#"
    [
      "alpha",
      "beta",
      "project"
    ]
    "#
    );

    Ok(())
}

#[cfg(feature = "test-universal")]
#[test]
fn cycle_invert_leaf_with_acyclic_leaf() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    setup_leaf_cycle(&context, true)?;

    uv_snapshot!(context.filters(), context.tree().arg("--frozen").arg("--invert"), @"
    exit_code: 0 (success)
    ----- stdout -----
    alpha v1.0.0
    ├── beta v1.0.0
    │   └── alpha v1.0.0
    │       ├── beta v1.0.0 (*)
    │       └── project v1.0.0
    └── project v1.0.0
    leaf v1.0.0
    └── project v1.0.0
    (*) Package tree already displayed
    ");

    assert_json_snapshot!(
        json_tree_package_names(context.tree().arg("--frozen").arg("--invert"))?,
        @r#"
    [
      "alpha",
      "beta",
      "leaf",
      "project"
    ]
    "#
    );

    uv_snapshot!(context.filters(), context.tree().arg("--frozen").arg("--invert").arg("--package").arg("beta"), @"
    exit_code: 0 (success)
    ----- stdout -----
    beta v1.0.0
    └── alpha v1.0.0
        ├── beta v1.0.0
        │   └── alpha v1.0.0 (*)
        └── project v1.0.0
    (*) Package tree already displayed
    ");

    Ok(())
}

#[cfg(feature = "test-universal")]
#[test]
fn cycle_depth_boundary_no_premature_dedupe() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["cycle-root==2.3.0", "cycle-backref==3.0.0"]
    "#,
    )?;

    // With --depth 3, packages at the depth boundary (depth 3) are shown but not
    // marked as visited. Packages below the boundary (e.g., `cycle-backref` at depth 1)
    // are correctly marked visited and show (*) on later appearances. Leaf packages
    // like `pbr` (no children in this graph) appear without (*) even when visited,
    // since there is nothing to deduplicate.
    uv_snapshot!(context.filters(), context.tree().arg("--universal").arg("--depth").arg("3"), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    ├── cycle-backref v3.0.0
    │   ├── cycle-leaf-c v1.16.0
    │   └── cycle-root v2.3.0
    │       ├── cycle-backref v3.0.0 (*)
    │       ├── cycle-leaf-a v1.0.0
    │       ├── cycle-leaf-b v6.0.0
    │       └── cycle-nested v1.1.0
    └── cycle-root v2.3.0 (*)
    (*) Package tree already displayed

    ----- stderr -----
    Resolved 10 packages in [TIME]
    ");

    Ok(())
}

#[cfg(feature = "test-universal")]
#[test]
fn cycle_invert_deep() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["cycle-root==2.3.0", "cycle-backref==3.0.0"]
    "#,
    )?;

    // With --invert and --depth 2, cycles in the reversed graph should be
    // detected and marked with (*) without causing infinite loops.
    uv_snapshot!(context.filters(), context.tree().arg("--universal").arg("--invert").arg("--depth").arg("2"), @"
    exit_code: 0 (success)
    ----- stdout -----
    cycle-leaf-a v1.0.0
    └── cycle-root v2.3.0
        ├── cycle-backref v3.0.0
        └── project v0.1.0
    cycle-leaf-b v6.0.0
    └── cycle-root v2.3.0 (*)
    cycle-leaf-c v1.16.0
    ├── cycle-backref v3.0.0
    │   ├── cycle-root v2.3.0 (*)
    │   └── project v0.1.0
    └── cycle-nested v1.1.0
        └── cycle-root v2.3.0 (*)
    cycle-leaf-d v1.4.0
    └── cycle-nested v1.1.0 (*)
    cycle-leaf-e v1.0.0
    └── cycle-trace v1.4.0
        └── cycle-nested v1.1.0 (*)
    (*) Package tree already displayed

    ----- stderr -----
    Resolved 10 packages in [TIME]
    ");

    Ok(())
}

#[cfg(feature = "test-universal")]
#[test]
fn cycle_depth_no_dedupe() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["cycle-root==2.3.0", "cycle-backref==3.0.0"]
    "#,
    )?;

    // With --no-dedupe and --depth 2, packages should be expanded each time they
    // appear (up to the depth limit), and cycles should still be marked with (*).
    uv_snapshot!(context.filters(), context.tree().arg("--universal").arg("--no-dedupe").arg("--depth").arg("2"), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    ├── cycle-backref v3.0.0
    │   ├── cycle-leaf-c v1.16.0
    │   └── cycle-root v2.3.0
    └── cycle-root v2.3.0
        ├── cycle-backref v3.0.0
        ├── cycle-leaf-a v1.0.0
        ├── cycle-leaf-b v6.0.0
        └── cycle-nested v1.1.0

    ----- stderr -----
    Resolved 10 packages in [TIME]
    ");

    Ok(())
}

#[cfg(feature = "test-universal")]
#[test]
fn workspace_dev() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["outdated-package"]

        [dependency-groups]
        dev = ["child"]

        [tool.uv.workspace]
        members = ["child"]

        [tool.uv.sources]
        child = { workspace = true }
    "#,
    )?;

    let child = context.temp_dir.child("child");
    let pyproject_toml = child.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "child"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["simple-package"]
    "#,
    )?;

    uv_snapshot!(context.filters(), context.tree().arg("--universal"), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    ├── outdated-package v4.3.0
    └── child v0.1.0 (group: dev)
        └── simple-package v2.1.3
    child v0.1.0 (*)
    (*) Package tree already displayed

    ----- stderr -----
    Resolved 4 packages in [TIME]
    "
    );

    // Under `--no-dev`, the member should still be included, since we show the entire workspace.
    // But it shouldn't be considered a dependency of the root.
    uv_snapshot!(context.filters(), context.tree().arg("--universal").arg("--no-dev"), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    └── outdated-package v4.3.0
    child v0.1.0
    └── simple-package v2.1.3

    ----- stderr -----
    Resolved 4 packages in [TIME]
    "
    );

    // `uv tree` should update the lockfile
    let lock = context.read("uv.lock");
    assert!(!lock.is_empty());

    Ok(())
}

/// An inverted tree should only follow consumers that activate the extra required by the path.
#[cfg(feature = "test-universal")]
#[test]
fn invert_preserves_extra_attribution() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context.temp_dir.child("pyproject.toml").write_str(
        r#"
        [tool.uv.workspace]
        members = ["packages/*"]
        "#,
    )?;

    let package_a = context.temp_dir.child("packages").child("package-a");
    package_a.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "package-a"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["package-p[feature]"]

        [tool.uv.sources]
        package-p = { workspace = true }
        "#,
    )?;

    let package_b = context.temp_dir.child("packages").child("package-b");
    package_b.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "package-b"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["package-p"]

        [tool.uv.sources]
        package-p = { workspace = true }
        "#,
    )?;

    let package_p = context.temp_dir.child("packages").child("package-p");
    package_p.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "package-p"
        version = "0.1.0"
        requires-python = ">=3.12"

        [project.optional-dependencies]
        feature = ["package-x"]

        [tool.uv.sources]
        package-x = { workspace = true }
        "#,
    )?;

    let package_x = context.temp_dir.child("packages").child("package-x");
    package_x.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "package-x"
        version = "0.1.0"
        requires-python = ">=3.12"
        "#,
    )?;

    uv_snapshot!(context.filters(), context.tree().arg("--universal").arg("--invert").arg("--package").arg("package-x"), @"
    exit_code: 0 (success)
    ----- stdout -----
    package-x v0.1.0
    └── package-p v0.1.0 (extra: feature)
        └── package-a[feature] v0.1.0

    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");

    assert_json_snapshot!(
        json_tree_package_names(
            context
                .tree()
                .arg("--universal")
                .arg("--invert")
                .arg("--package")
                .arg("package-x")
        )?,
        @r#"
    [
      "package-a",
      "package-p",
      "package-x"
    ]
    "#
    );

    Ok(())
}

/// Member dependency groups describe the root member, not consumers of that member.
#[cfg(feature = "test-universal")]
#[test]
fn invert_preserves_dependency_group_attribution() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context.temp_dir.child("pyproject.toml").write_str(
        r#"
        [tool.uv.workspace]
        members = ["packages/*"]
        "#,
    )?;

    let package_a = context.temp_dir.child("packages").child("package-a");
    package_a.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "package-a"
        version = "0.1.0"
        requires-python = ">=3.12"

        [dependency-groups]
        dev = ["package-x"]

        [tool.uv.sources]
        package-x = { workspace = true }
        "#,
    )?;

    let package_b = context.temp_dir.child("packages").child("package-b");
    package_b.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "package-b"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["package-a"]

        [tool.uv.sources]
        package-a = { workspace = true }
        "#,
    )?;

    let package_x = context.temp_dir.child("packages").child("package-x");
    package_x.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "package-x"
        version = "0.1.0"
        requires-python = ">=3.12"
        "#,
    )?;

    uv_snapshot!(context.filters(), context.tree().arg("--universal").arg("--invert").arg("--package").arg("package-x"), @"
    exit_code: 0 (success)
    ----- stdout -----
    package-x v0.1.0
    └── package-a v0.1.0 (group: dev)

    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");

    assert_json_snapshot!(
        json_tree_package_names(
            context
                .tree()
                .arg("--universal")
                .arg("--invert")
                .arg("--package")
                .arg("package-x")
        )?,
        @r#"
    [
      "package-a",
      "package-x"
    ]
    "#
    );

    Ok(())
}

/// Universal inverted trees should not join edges from mutually exclusive environments.
#[cfg(feature = "test-universal")]
#[test]
fn invert_preserves_marker_attribution() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context.temp_dir.child("pyproject.toml").write_str(
        r#"
        [tool.uv.workspace]
        members = ["packages/*"]
        "#,
    )?;

    let package_a = context.temp_dir.child("packages").child("package-a");
    package_a.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "package-a"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["package-p; sys_platform == 'win32'"]

        [tool.uv.sources]
        package-p = { workspace = true }
        "#,
    )?;

    let package_p = context.temp_dir.child("packages").child("package-p");
    package_p.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "package-p"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["package-x; sys_platform == 'linux'"]

        [tool.uv.sources]
        package-x = { workspace = true }
        "#,
    )?;

    let package_b = context.temp_dir.child("packages").child("package-b");
    package_b.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "package-b"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["package-p; sys_platform == 'linux'"]

        [tool.uv.sources]
        package-p = { workspace = true }
        "#,
    )?;

    let package_x = context.temp_dir.child("packages").child("package-x");
    package_x.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "package-x"
        version = "0.1.0"
        requires-python = ">=3.12"
        "#,
    )?;

    uv_snapshot!(context.filters(), context.tree().arg("--universal").arg("--invert").arg("--package").arg("package-x"), @"
    exit_code: 0 (success)
    ----- stdout -----
    package-x v0.1.0
    └── package-p v0.1.0
        └── package-b v0.1.0

    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");

    assert_json_snapshot!(
        json_tree_package_names(
            context
                .tree()
                .arg("--universal")
                .arg("--invert")
                .arg("--package")
                .arg("package-x")
        )?,
        @r#"
    [
      "package-b",
      "package-p",
      "package-x"
    ]
    "#
    );

    Ok(())
}

/// Universal inverted trees should include every version that directly depends on a package in a
/// satisfiable marker environment.
#[cfg(feature = "test-universal")]
#[test]
fn invert_preserves_marker_split_versions() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context.temp_dir.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = [
            "bar==1.0.0; sys_platform == 'win32'",
            "bar==2.0.0; sys_platform != 'win32'",
        ]
        "#,
    )?;

    context.temp_dir.child("uv.lock").write_str(
        r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        resolution-markers = [
            "sys_platform == 'win32'",
            "sys_platform != 'win32'",
        ]

        [[package]]
        name = "bar"
        version = "1.0.0"
        source = { registry = "https://pypi.org/simple" }
        resolution-markers = [
            "sys_platform == 'win32'",
        ]
        dependencies = [
            { name = "baz", marker = "sys_platform == 'win32'" },
        ]

        [[package]]
        name = "bar"
        version = "2.0.0"
        source = { registry = "https://pypi.org/simple" }
        resolution-markers = [
            "sys_platform != 'win32'",
        ]
        dependencies = [
            { name = "baz", marker = "sys_platform != 'win32'" },
        ]

        [[package]]
        name = "baz"
        version = "1.0.0"
        source = { registry = "https://pypi.org/simple" }

        [[package]]
        name = "foo"
        version = "1.0.0"
        source = { virtual = "." }
        dependencies = [
            { name = "bar", version = "1.0.0", source = { registry = "https://pypi.org/simple" }, marker = "sys_platform == 'win32'" },
            { name = "bar", version = "2.0.0", source = { registry = "https://pypi.org/simple" }, marker = "sys_platform != 'win32'" },
        ]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.tree().arg("--frozen").arg("--universal").arg("--invert").arg("--package").arg("baz"), @"
    exit_code: 0 (success)
    ----- stdout -----
    baz v1.0.0
    ├── bar v1.0.0
    │   └── foo v1.0.0
    └── bar v2.0.0
        └── foo v1.0.0
    ");

    Ok(())
}

/// Declared conflicts are world knowledge for the encoded extra and group markers.
#[cfg(feature = "test-universal")]
#[test]
fn invert_preserves_conflict_marker_attribution() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context.temp_dir.child("pyproject.toml").write_str(
        r#"
        [tool.uv.workspace]
        members = ["packages/*"]

        [tool.uv]
        conflicts = [
          [
            { package = "a", extra = "bar" },
            { package = "p", extra = "foo" },
          ],
        ]
        "#,
    )?;

    let package_a = context.temp_dir.child("packages").child("a");
    package_a.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "a"
        version = "0.1.0"
        requires-python = ">=3.12"

        [project.optional-dependencies]
        bar = ["p[foo]"]

        [tool.uv.sources]
        p = { workspace = true }
        "#,
    )?;

    let package_p = context.temp_dir.child("packages").child("p");
    package_p.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "p"
        version = "0.1.0"
        requires-python = ">=3.12"

        [project.optional-dependencies]
        foo = ["x"]

        [tool.uv.sources]
        x = { workspace = true }
        "#,
    )?;

    let package_x = context.temp_dir.child("packages").child("x");
    package_x.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "x"
        version = "0.1.0"
        requires-python = ">=3.12"
        "#,
    )?;

    // Preserve both encoded conflict conditions in the lock. Resolving this project would simplify
    // the impossible `a[bar] -> p[foo]` edge before `uv tree` can exercise path attribution.
    context.temp_dir.child("uv.lock").write_str(
        r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        conflicts = [[
            { package = "a", extra = "bar" },
            { package = "p", extra = "foo" },
        ]]

        [manifest]
        members = [
            "a",
            "p",
            "x",
        ]

        [[package]]
        name = "a"
        version = "0.1.0"
        source = { virtual = "packages/a" }

        [package.optional-dependencies]
        bar = [
            { name = "p", extra = ["foo"], marker = "extra == 'extra-1-a-bar'" },
        ]

        [[package]]
        name = "p"
        version = "0.1.0"
        source = { editable = "packages/p" }

        [package.optional-dependencies]
        foo = [
            { name = "x", marker = "extra == 'extra-1-p-foo'" },
        ]

        [[package]]
        name = "x"
        version = "0.1.0"
        source = { editable = "packages/x" }
        "#,
    )?;

    uv_snapshot!(context.filters(), context.tree().arg("--frozen").arg("--universal").arg("--invert").arg("--package").arg("x"), @"
    exit_code: 0 (success)
    ----- stdout -----
    x v0.1.0
    └── p v0.1.0 (extra: foo)
    ");

    Ok(())
}

#[cfg(feature = "test-universal")]
#[test]
fn non_project() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [tool.uv.workspace]
        members = []

        [dependency-groups]
        async = ["outdated-package"]
    "#,
    )?;

    uv_snapshot!(context.filters(), context.tree().arg("--universal").arg("--group").arg("async"), @"
    exit_code: 0 (success)
    ----- stdout -----
    outdated-package v4.3.0 (group: async)

    ----- stderr -----
    warning: No `requires-python` value found in the workspace. Defaulting to `>=3.12`.
    Resolved 1 package in [TIME]
    "
    );

    // `uv tree` should update the lockfile
    let lock = context.read("uv.lock");
    assert!(!lock.is_empty());

    Ok(())
}

/// A pyproject.toml with only `[dependency-groups]` (no `[project]`, no `[tool.uv.workspace]`)
/// is valid per PEP 735, and `uv tree` must handle it.
#[cfg(feature = "test-universal")]
#[test]
fn dependency_groups_only() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [dependency-groups]
        async = ["anyio"]
    "#,
    )?;

    uv_snapshot!(context.filters(), context.tree().arg("--universal").arg("--group").arg("async"), @"
    exit_code: 0 (success)
    ----- stdout -----
    anyio v4.3.0 (group: async)
    ├── idna v3.6
    └── sniffio v1.3.1

    ----- stderr -----
    warning: No `requires-python` value found in the workspace. Defaulting to `>=3.12`.
    Resolved 3 packages in [TIME]
    "
    );

    // `uv tree` should update the lockfile
    let lock = context.read("uv.lock");
    assert!(!lock.is_empty());

    Ok(())
}

#[test]
fn non_project_group_selection_with_extras() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let leaf = context.temp_dir.child("leaf");
    leaf.create_dir_all()?;
    leaf.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "leaf"
        version = "0.1.0"
        requires-python = ">=3.12"
        "#,
    )?;
    let leaf_url = Url::from_file_path(leaf.path())
        .map_err(|()| anyhow::anyhow!("failed to convert leaf path to URL"))?;

    let child = context.temp_dir.child("child");
    child.create_dir_all()?;
    child.child("pyproject.toml").write_str(&formatdoc! {
        r#"
        [project]
        name = "child"
        version = "0.1.0"
        requires-python = ">=3.12"

        [project.optional-dependencies]
        feature = ["leaf @ {leaf_url}"]
        "#,
    })?;
    let child_url = Url::from_file_path(child.path())
        .map_err(|()| anyhow::anyhow!("failed to convert child path to URL"))?;

    let test_dependency = context.temp_dir.child("test-dependency");
    test_dependency.create_dir_all()?;
    test_dependency.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "test-dependency"
        version = "0.1.0"
        requires-python = ">=3.12"
        "#,
    )?;
    let test_dependency_url = Url::from_file_path(test_dependency.path())
        .map_err(|()| anyhow::anyhow!("failed to convert test dependency path to URL"))?;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&formatdoc! {
        r#"
        [tool.uv.workspace]
        members = []

        [dependency-groups]
        dev = ["child[feature] @ {child_url}"]
        test = ["test-dependency @ {test_dependency_url}"]
        "#,
    })?;

    uv_snapshot!(context.filters(), context.tree().arg("--only-group").arg("dev"), @"
    exit_code: 0 (success)
    ----- stdout -----
    child[feature] v0.1.0 (group: dev)
    └── leaf v0.1.0 (extra: feature)

    ----- stderr -----
    warning: No `requires-python` value found in the workspace. Defaulting to `>=3.12`.
    Resolved 3 packages in [TIME]
    ");

    let script = context.temp_dir.child("script.py");
    script.write_str(&formatdoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["child[feature] @ {child_url}"]
        # ///
    "#})?;

    uv_snapshot!(context.filters(), context.tree().arg("--script").arg(script.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    child[feature] v0.1.0
    └── leaf v0.1.0 (extra: feature)

    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context.tree()
        .arg("--script")
        .arg(script.path())
        .arg("--preview-features")
        .arg("json-output")
        .arg("--format")
        .arg("json")
        // A script's direct requirements are at depth zero, even though the JSON graph represents
        // the script itself as a root node.
        .arg("--depth")
        .arg("0"), @r#"
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
      "roots": [
        {
          "id": "script+[TEMP_DIR]/script.py"
        }
      ],
      "inverted": false,
      "resolution": {
        "child==0.1.0@directory+[TEMP_DIR]/child": {
          "name": "child",
          "version": "0.1.0",
          "source": {
            "directory": "[TEMP_DIR]/child"
          },
          "kind": "package",
          "dependencies": [],
          "optional_dependencies": [
            {
              "name": "feature",
              "id": "child[feature]==0.1.0@directory+[TEMP_DIR]/child"
            }
          ]
        },
        "child[feature]==0.1.0@directory+[TEMP_DIR]/child": {
          "name": "child",
          "version": "0.1.0",
          "source": {
            "directory": "[TEMP_DIR]/child"
          },
          "kind": {
            "extra": "feature"
          },
          "dependencies": [
            {
              "id": "child==0.1.0@directory+[TEMP_DIR]/child"
            }
          ]
        },
        "script+[TEMP_DIR]/script.py": {
          "kind": "script",
          "path": "[TEMP_DIR]/script.py",
          "dependencies": [
            {
              "id": "child[feature]==0.1.0@directory+[TEMP_DIR]/child"
            }
          ]
        }
      }
    }

    ----- stderr -----
    Resolved 2 packages in [TIME]
    "#);

    Ok(())
}

#[cfg(feature = "test-universal")]
#[test]
fn non_project_member() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [tool.uv.workspace]
        members = ["child"]

        [dependency-groups]
        async = ["outdated-package"]
        "#,
    )?;

    let child = context.temp_dir.child("child");
    child.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "child"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["simple-package", "tree-leaf-c", "outdated-package"]

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
        "#,
    )?;

    uv_snapshot!(context.filters(), context.tree().arg("--universal").arg("--group").arg("async"), @"
    exit_code: 0 (success)
    ----- stdout -----
    outdated-package v4.3.0 (group: async)
    child v0.1.0
    ├── outdated-package v4.3.0
    ├── simple-package v2.1.3
    └── tree-leaf-c v3.6

    ----- stderr -----
    Resolved 4 packages in [TIME]
    "
    );

    uv_snapshot!(context.filters(), context.tree().arg("--universal").arg("--invert").arg("--group").arg("async"), @"
    exit_code: 0 (success)
    ----- stdout -----
    outdated-package v4.3.0
    └── child v0.1.0
    simple-package v2.1.3
    └── child v0.1.0
    tree-leaf-c v3.6
    └── child v0.1.0

    ----- stderr -----
    Resolved 4 packages in [TIME]
    "
    );

    // `uv tree` should update the lockfile
    let lock = context.read("uv.lock");
    assert!(!lock.is_empty());

    Ok(())
}

#[test]
fn script() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let script = context.temp_dir.child("script.py");
    script.write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = [
        #   "tree-parent<3",
        #   "tree-root",
        # ]
        # ///

        import tree_parent
        from tree_root.pretty import pprint

        resp = tree_parent.get("https://peps.python.org/api/peps.json")
        data = resp.json()
        pprint([(k, v["title"]) for k, v in data.items()][:10])
    "#})?;

    uv_snapshot!(context.filters(), context.tree().arg("--script").arg(script.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    tree-root v3.0.2
    ├── tree-branch-a v1.7.0
    ├── tree-branch-b v8.1.7
    ├── tree-branch-c v2.1.2
    ├── tree-branch-d v3.1.3
    │   └── tree-shared-leaf v2.1.5
    └── tree-branch-e v3.0.1
        └── tree-shared-leaf v2.1.5
    tree-parent v2.31.0
    ├── tree-leaf-a v2024.2.2
    ├── tree-leaf-b v3.3.2
    ├── tree-leaf-c v3.6
    └── tree-leaf-d v2.2.1

    ----- stderr -----
    Resolved 12 packages in [TIME]
    ");

    // If the lockfile didn't exist already, it shouldn't be persisted to disk.
    assert!(!context.temp_dir.child("uv.lock").exists());

    // Explicitly lock the script.
    uv_snapshot!(context.filters(), context.lock().arg("--script").arg(script.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 12 packages in [TIME]
    ");

    let lock = context.read("script.py.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [manifest]
        requirements = [
            { name = "tree-parent", specifier = "<3" },
            { name = "tree-root" },
        ]

        [[package]]
        name = "tree-branch-a"
        version = "1.7.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/tree_branch_a-1.7.0.tar.gz", hash = "sha256:0e8b4d8d1cc1c99e663b0cfd72b0719ed54f5e608f93b7dd060bee21c768df6f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/tree_branch_a-1.7.0-py3-none-any.whl", hash = "sha256:accb8b35b9634f68a3511b28d73914e3000352a178792b82d0bb715101d00a05", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "tree-branch-b"
        version = "8.1.7"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/tree_branch_b-8.1.7.tar.gz", hash = "sha256:81fee543af2969cc2841108e4e95ed699d9e4406c41e81b21a7bd2b0f6a57ba7", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/tree_branch_b-8.1.7-py3-none-any.whl", hash = "sha256:04ff0ba82fcb8dbbe389ebcec80b95f2ccc9dd9dc22a3ee4f5d1c03d9a369b54", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "tree-branch-c"
        version = "2.1.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/tree_branch_c-2.1.2.tar.gz", hash = "sha256:60fbd7d5254df5565235d0ef3aba0d40b64ce11895109795666b5df2c12789c7", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/tree_branch_c-2.1.2-py3-none-any.whl", hash = "sha256:04883160b5990d638f357d739e7c8e9fd5ff69ace5912f09a8298b1cbd9c377b", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "tree-branch-d"
        version = "3.1.3"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "tree-shared-leaf" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/tree_branch_d-3.1.3.tar.gz", hash = "sha256:fb4f82511c4c7912c6e270694ff46526e3debdd622f124fcecdb3ce365a152a6", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/tree_branch_d-3.1.3-py3-none-any.whl", hash = "sha256:4a5e468bb4d521fb171724fbb55d955c9ebebb062d24fee88364fd197d3af22a", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "tree-branch-e"
        version = "3.0.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "tree-shared-leaf" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/tree_branch_e-3.0.1.tar.gz", hash = "sha256:eec8f5bea87e562a30f7b77d8469f20a9a4288f3c14519fc44224083e2a3d6cd", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/tree_branch_e-3.0.1-py3-none-any.whl", hash = "sha256:b35bd1854ba606553a75765a64cdc6491ec5247b95e298b691ed7cc74e065384", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "tree-leaf-a"
        version = "2024.2.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/tree_leaf_a-2024.2.2.tar.gz", hash = "sha256:cb4c647683d931656c195fccfdaf8e9d14d120c7a85f997fe737813f59b2b0a6", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/tree_leaf_a-2024.2.2-py3-none-any.whl", hash = "sha256:59564d25467f6680748a6ce50268b92950cafb5e99f670232c34946c4a0db296", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "tree-leaf-b"
        version = "3.3.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/tree_leaf_b-3.3.2.tar.gz", hash = "sha256:87dde9d0913d868ddff2fd9eb437ee782e85638cc03f6b782a258568125f2d55", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/tree_leaf_b-3.3.2-py3-none-any.whl", hash = "sha256:38833589aa79148a32865dba93e2c157529bd3c7e481df6e62d1d16839d6c103", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "tree-leaf-c"
        version = "3.6"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/tree_leaf_c-3.6.tar.gz", hash = "sha256:6dad7aa8f7e7eaffc985524f72a5e374d927adc2b3ecb1175c3629a64ac2dbc6", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/tree_leaf_c-3.6-py3-none-any.whl", hash = "sha256:de387642087aac7657ec7aa72251887d82ba8b9172ad7ef059ced6064ecca693", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "tree-leaf-d"
        version = "2.2.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/tree_leaf_d-2.2.1.tar.gz", hash = "sha256:2b0d10de82c9a5ed23ee0f04f85c7b61bebf4567c6695a532ef8bae394c86d5a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/tree_leaf_d-2.2.1-py3-none-any.whl", hash = "sha256:27ef175f3663d0b013d92c9fa49c66bf0ad154183a168921e4590ea5d7a4c662", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "tree-parent"
        version = "2.31.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "tree-leaf-a" },
            { name = "tree-leaf-b" },
            { name = "tree-leaf-c" },
            { name = "tree-leaf-d" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/tree_parent-2.31.0.tar.gz", hash = "sha256:e9883d43bca69de2c404151f689cdf40debb2cbdc730351d9a2611c826e53187", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/tree_parent-2.31.0-py3-none-any.whl", hash = "sha256:1da847b7426bf23f24cefedb23192c285d36754d87dae6db95a8190f3e98730e", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "tree-root"
        version = "3.0.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "tree-branch-a" },
            { name = "tree-branch-b" },
            { name = "tree-branch-c" },
            { name = "tree-branch-d" },
            { name = "tree-branch-e" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/tree_root-3.0.2.tar.gz", hash = "sha256:751cabcd159b54e2490056472ff5c99a7ae97ba1c5b0aea10d55ce09fb5687ee", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/tree_root-3.0.2-py3-none-any.whl", hash = "sha256:85ea5d6c627e1ffc8c5933ecd28c762d5c886c2352f8840e84866c754f1986f6", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "tree-shared-leaf"
        version = "2.1.5"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/tree_shared_leaf-2.1.5.tar.gz", hash = "sha256:5d58e887adc7cb408c7f9275064623e7b47a1a116f77722ffb07142a7413ac77", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/tree_shared_leaf-2.1.5-py3-none-any.whl", hash = "sha256:c07e3823316e4e58917f3cd8bead1c541a9eb0bb8340fb3cec3020ea9cd4bd1f", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Update the dependencies.
    script.write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = [
        #   "simple-package",
        #   "tree-parent<3",
        #   "tree-root",
        # ]
        # ///

        import tree_parent
        from tree_root.pretty import pprint

        resp = tree_parent.get("https://peps.python.org/api/peps.json")
        data = resp.json()
        pprint([(k, v["title"]) for k, v in data.items()][:10])
    "#})?;

    // `uv tree` should update the lockfile.
    uv_snapshot!(context.filters(), context.tree().arg("--script").arg(script.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    tree-root v3.0.2
    ├── tree-branch-a v1.7.0
    ├── tree-branch-b v8.1.7
    ├── tree-branch-c v2.1.2
    ├── tree-branch-d v3.1.3
    │   └── tree-shared-leaf v2.1.5
    └── tree-branch-e v3.0.1
        └── tree-shared-leaf v2.1.5
    tree-parent v2.31.0
    ├── tree-leaf-a v2024.2.2
    ├── tree-leaf-b v3.3.2
    ├── tree-leaf-c v3.6
    └── tree-leaf-d v2.2.1
    simple-package v2.1.3

    ----- stderr -----
    Resolved 13 packages in [TIME]
    ");

    let lock = context.read("script.py.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [manifest]
        requirements = [
            { name = "simple-package" },
            { name = "tree-parent", specifier = "<3" },
            { name = "tree-root" },
        ]

        [[package]]
        name = "simple-package"
        version = "2.1.3"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/simple_package-2.1.3.tar.gz", hash = "sha256:10d19b0f846b6482adf48cbe1d53470b837bbd77d44777dfa92f5be59cad45e4", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/simple_package-2.1.3-py3-none-any.whl", hash = "sha256:0a27d6da31d01818d02c374ab253af79875bff7e3ad144e6d2b16d545f2f329b", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "tree-branch-a"
        version = "1.7.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/tree_branch_a-1.7.0.tar.gz", hash = "sha256:0e8b4d8d1cc1c99e663b0cfd72b0719ed54f5e608f93b7dd060bee21c768df6f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/tree_branch_a-1.7.0-py3-none-any.whl", hash = "sha256:accb8b35b9634f68a3511b28d73914e3000352a178792b82d0bb715101d00a05", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "tree-branch-b"
        version = "8.1.7"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/tree_branch_b-8.1.7.tar.gz", hash = "sha256:81fee543af2969cc2841108e4e95ed699d9e4406c41e81b21a7bd2b0f6a57ba7", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/tree_branch_b-8.1.7-py3-none-any.whl", hash = "sha256:04ff0ba82fcb8dbbe389ebcec80b95f2ccc9dd9dc22a3ee4f5d1c03d9a369b54", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "tree-branch-c"
        version = "2.1.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/tree_branch_c-2.1.2.tar.gz", hash = "sha256:60fbd7d5254df5565235d0ef3aba0d40b64ce11895109795666b5df2c12789c7", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/tree_branch_c-2.1.2-py3-none-any.whl", hash = "sha256:04883160b5990d638f357d739e7c8e9fd5ff69ace5912f09a8298b1cbd9c377b", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "tree-branch-d"
        version = "3.1.3"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "tree-shared-leaf" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/tree_branch_d-3.1.3.tar.gz", hash = "sha256:fb4f82511c4c7912c6e270694ff46526e3debdd622f124fcecdb3ce365a152a6", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/tree_branch_d-3.1.3-py3-none-any.whl", hash = "sha256:4a5e468bb4d521fb171724fbb55d955c9ebebb062d24fee88364fd197d3af22a", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "tree-branch-e"
        version = "3.0.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "tree-shared-leaf" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/tree_branch_e-3.0.1.tar.gz", hash = "sha256:eec8f5bea87e562a30f7b77d8469f20a9a4288f3c14519fc44224083e2a3d6cd", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/tree_branch_e-3.0.1-py3-none-any.whl", hash = "sha256:b35bd1854ba606553a75765a64cdc6491ec5247b95e298b691ed7cc74e065384", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "tree-leaf-a"
        version = "2024.2.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/tree_leaf_a-2024.2.2.tar.gz", hash = "sha256:cb4c647683d931656c195fccfdaf8e9d14d120c7a85f997fe737813f59b2b0a6", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/tree_leaf_a-2024.2.2-py3-none-any.whl", hash = "sha256:59564d25467f6680748a6ce50268b92950cafb5e99f670232c34946c4a0db296", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "tree-leaf-b"
        version = "3.3.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/tree_leaf_b-3.3.2.tar.gz", hash = "sha256:87dde9d0913d868ddff2fd9eb437ee782e85638cc03f6b782a258568125f2d55", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/tree_leaf_b-3.3.2-py3-none-any.whl", hash = "sha256:38833589aa79148a32865dba93e2c157529bd3c7e481df6e62d1d16839d6c103", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "tree-leaf-c"
        version = "3.6"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/tree_leaf_c-3.6.tar.gz", hash = "sha256:6dad7aa8f7e7eaffc985524f72a5e374d927adc2b3ecb1175c3629a64ac2dbc6", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/tree_leaf_c-3.6-py3-none-any.whl", hash = "sha256:de387642087aac7657ec7aa72251887d82ba8b9172ad7ef059ced6064ecca693", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "tree-leaf-d"
        version = "2.2.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/tree_leaf_d-2.2.1.tar.gz", hash = "sha256:2b0d10de82c9a5ed23ee0f04f85c7b61bebf4567c6695a532ef8bae394c86d5a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/tree_leaf_d-2.2.1-py3-none-any.whl", hash = "sha256:27ef175f3663d0b013d92c9fa49c66bf0ad154183a168921e4590ea5d7a4c662", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "tree-parent"
        version = "2.31.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "tree-leaf-a" },
            { name = "tree-leaf-b" },
            { name = "tree-leaf-c" },
            { name = "tree-leaf-d" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/tree_parent-2.31.0.tar.gz", hash = "sha256:e9883d43bca69de2c404151f689cdf40debb2cbdc730351d9a2611c826e53187", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/tree_parent-2.31.0-py3-none-any.whl", hash = "sha256:1da847b7426bf23f24cefedb23192c285d36754d87dae6db95a8190f3e98730e", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "tree-root"
        version = "3.0.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "tree-branch-a" },
            { name = "tree-branch-b" },
            { name = "tree-branch-c" },
            { name = "tree-branch-d" },
            { name = "tree-branch-e" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/tree_root-3.0.2.tar.gz", hash = "sha256:751cabcd159b54e2490056472ff5c99a7ae97ba1c5b0aea10d55ce09fb5687ee", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/tree_root-3.0.2-py3-none-any.whl", hash = "sha256:85ea5d6c627e1ffc8c5933ecd28c762d5c886c2352f8840e84866c754f1986f6", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "tree-shared-leaf"
        version = "2.1.5"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/tree_shared_leaf-2.1.5.tar.gz", hash = "sha256:5d58e887adc7cb408c7f9275064623e7b47a1a116f77722ffb07142a7413ac77", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/tree_shared_leaf-2.1.5-py3-none-any.whl", hash = "sha256:c07e3823316e4e58917f3cd8bead1c541a9eb0bb8340fb3cec3020ea9cd4bd1f", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    Ok(())
}

#[cfg(feature = "test-universal")]
#[test]
fn only_group() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "simple-package",
            "other-package",
        ]

        [dependency-groups]
        dev = [
            "tree-root",
            "other-package",
        ]
        test = [
            "tree-parent",
        ]
    "#,
    )?;

    uv_snapshot!(context.filters(), context.tree().arg("--universal"), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    ├── other-package v2.0.1
    ├── simple-package v2.1.3
    ├── other-package v2.0.1 (group: dev)
    └── tree-root v3.0.2 (group: dev)
        ├── tree-branch-a v1.7.0
        ├── tree-branch-b v8.1.7
        ├── tree-branch-c v2.1.2
        ├── tree-branch-d v3.1.3
        │   └── tree-shared-leaf v2.1.5
        └── tree-branch-e v3.0.1
            └── tree-shared-leaf v2.1.5

    ----- stderr -----
    Resolved 15 packages in [TIME]
    "
    );

    uv_snapshot!(context.filters(), context.tree().arg("--universal").arg("--only-group").arg("dev"), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    ├── other-package v2.0.1 (group: dev)
    └── tree-root v3.0.2 (group: dev)
        ├── tree-branch-a v1.7.0
        ├── tree-branch-b v8.1.7
        ├── tree-branch-c v2.1.2
        ├── tree-branch-d v3.1.3
        │   └── tree-shared-leaf v2.1.5
        └── tree-branch-e v3.0.1
            └── tree-shared-leaf v2.1.5

    ----- stderr -----
    Resolved 15 packages in [TIME]
    "
    );

    uv_snapshot!(context.filters(), context.tree().arg("--universal").arg("--only-group").arg("test"), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    └── tree-parent v2.31.0 (group: test)
        ├── tree-leaf-a v2024.2.2
        ├── tree-leaf-b v3.3.2
        ├── tree-leaf-c v3.6
        └── tree-leaf-d v2.2.1

    ----- stderr -----
    Resolved 15 packages in [TIME]
    "
    );

    // `uv tree` should update the lockfile
    let lock = context.read("uv.lock");
    assert!(!lock.is_empty());

    Ok(())
}

#[cfg(feature = "test-universal")]
#[test]
fn show_sizes() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12")
        .with_default_index(&_server.index_url())
        .with_filtered_sizes();

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["simple-package"]
    "#,
    )?;

    uv_snapshot!(context.filters(), context.tree().arg("--show-sizes").arg("--universal"), @"
    exit_code: 0 (success)
    ----- stdout -----
    project v0.1.0
    └── simple-package v2.1.3

    ----- stderr -----
    Resolved 2 packages in [TIME]
    "
    );

    uv_snapshot!(context.filters(), context.tree()
        .arg("--preview-features")
        .arg("json-output")
        .arg("--format")
        .arg("json")
        .arg("--universal"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "workspace_root": "[TEMP_DIR]/",
      "workspace": {
        "path": "[TEMP_DIR]/",
        "id": "workspace+[TEMP_DIR]/"
      },
      "roots": [
        {
          "id": "project==0.1.0@virtual+[TEMP_DIR]/"
        }
      ],
      "inverted": false,
      "members": [
        {
          "name": "project",
          "path": "[TEMP_DIR]/",
          "id": "project==0.1.0@virtual+[TEMP_DIR]/"
        }
      ],
      "resolution": {
        "project==0.1.0@virtual+[TEMP_DIR]/": {
          "name": "project",
          "version": "0.1.0",
          "source": {
            "virtual": "[TEMP_DIR]/"
          },
          "kind": "package",
          "dependencies": [
            {
              "id": "simple-package==2.1.3@registry+http://[LOCALHOST]/simple/"
            }
          ]
        },
        "simple-package==2.1.3@registry+http://[LOCALHOST]/simple/": {
          "name": "simple-package",
          "version": "2.1.3",
          "source": {
            "registry": {
              "url": "http://[LOCALHOST]/simple/"
            }
          },
          "kind": "package",
          "dependencies": [],
          "wheels": [
            {
              "url": "http://[LOCALHOST]/files/simple_package-2.1.3-py3-none-any.whl",
              "hashes": {
                "sha256": "0a27d6da31d01818d02c374ab253af79875bff7e3ad144e6d2b16d545f2f329b"
              },
              "upload_time": "2024-03-24T00:00:00Z",
              "filename": "simple_package-2.1.3-py3-none-any.whl"
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
    Resolved 2 packages in [TIME]
    "#);

    Ok(())
}

#[test]
fn workspace_circular_dependencies() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    // Create workspace root
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [tool.uv.workspace]
        members = ["packages/*"]
    "#,
    )?;

    // Create package-a that depends on package-b
    let package_a_dir = context.temp_dir.child("packages").child("package-a");
    package_a_dir.create_dir_all()?;
    let package_a_pyproject = package_a_dir.child("pyproject.toml");
    package_a_pyproject.write_str(
        r#"
        [project]
        name = "package-a"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["package-b"]

        [tool.uv.sources]
        package-b = { workspace = true }
    "#,
    )?;

    // Create package-b that depends on package-a (circular dependency)
    let package_b_dir = context.temp_dir.child("packages").child("package-b");
    package_b_dir.create_dir_all()?;
    let package_b_pyproject = package_b_dir.child("pyproject.toml");
    package_b_pyproject.write_str(
        r#"
        [project]
        name = "package-b"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["package-a"]

        [tool.uv.sources]
        package-a = { workspace = true }
    "#,
    )?;

    // Test that package-a is at the root when requested
    uv_snapshot!(context.filters(), context.tree().arg("--package").arg("package-a"), @"
    exit_code: 0 (success)
    ----- stdout -----
    package-a v0.1.0
    └── package-b v0.1.0
        └── package-a v0.1.0 (*)
    (*) Package tree already displayed

    ----- stderr -----
    Resolved 2 packages in [TIME]
    "
    );

    // Test that package-b is at the root when requested
    uv_snapshot!(context.filters(), context.tree().arg("--package").arg("package-b"), @"
    exit_code: 0 (success)
    ----- stdout -----
    package-b v0.1.0
    └── package-a v0.1.0
        └── package-b v0.1.0 (*)
    (*) Package tree already displayed

    ----- stderr -----
    Resolved 2 packages in [TIME]
    "
    );

    // Test that both packages are shown as roots when both are requested
    uv_snapshot!(context.filters(), context.tree().arg("--package").arg("package-a").arg("--package").arg("package-b"), @"
    exit_code: 0 (success)
    ----- stdout -----
    package-a v0.1.0
    └── package-b v0.1.0
        └── package-a v0.1.0 (*)
    package-b v0.1.0 (*)
    (*) Package tree already displayed

    ----- stderr -----
    Resolved 2 packages in [TIME]
    "
    );

    Ok(())
}

#[cfg(feature = "test-universal")]
fn setup_leaf_cycle(context: &TestContext, with_acyclic_leaf: bool) -> Result<()> {
    let project_dependencies = if with_acyclic_leaf {
        r#""alpha==1.0.0", "leaf==1.0.0""#
    } else {
        r#""alpha==1.0.0""#
    };
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
            [project]
            name = "project"
            version = "1.0.0"
            requires-python = ">=3.12"
            dependencies = [{project_dependencies}]
        "#})?;

    let (leaf, root_dependencies) = if with_acyclic_leaf {
        (
            indoc! {r#"
                [[package]]
                name = "leaf"
                version = "1.0.0"
                source = { registry = "https://pypi.org/simple" }

            "#},
            r#"{ name = "alpha" }, { name = "leaf" }"#,
        )
    } else {
        ("", r#"{ name = "alpha" }"#)
    };
    context
        .temp_dir
        .child("uv.lock")
        .write_str(&formatdoc! {r#"
            version = 1
            revision = 3
            requires-python = ">=3.12"

            [[package]]
            name = "alpha"
            version = "1.0.0"
            source = {{ registry = "https://pypi.org/simple" }}
            dependencies = [{{ name = "beta" }}]

            [[package]]
            name = "beta"
            version = "1.0.0"
            source = {{ registry = "https://pypi.org/simple" }}
            dependencies = [{{ name = "alpha" }}]

            {leaf}[[package]]
            name = "project"
            version = "1.0.0"
            source = {{ virtual = "." }}
            dependencies = [{root_dependencies}]
        "#})?;

    Ok(())
}

#[cfg(feature = "test-universal")]
fn setup_json_output(context: &TestContext) -> Result<()> {
    context.temp_dir.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["package-a[feature]"]

        [dependency-groups]
        dev = ["package-c"]

        [tool.uv.sources]
        package-a = { path = "packages/package-a" }
        package-c = { path = "packages/package-c" }
        "#,
    )?;

    let package_a = context.temp_dir.child("packages/package-a");
    package_a.create_dir_all()?;
    package_a.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "package-a"
        version = "1.0.0"
        requires-python = ">=3.12"

        [project.optional-dependencies]
        feature = ["package-b"]

        [tool.uv.sources]
        package-b = { path = "../package-b" }
        "#,
    )?;

    for package in ["package-b", "package-c"] {
        let directory = context.temp_dir.child(format!("packages/{package}"));
        directory.create_dir_all()?;
        directory
            .child("pyproject.toml")
            .write_str(&formatdoc! {r#"
            [project]
            name = "{package}"
            version = "1.0.0"
            requires-python = ">=3.12"
        "#})?;
    }

    Ok(())
}

#[cfg(feature = "test-universal")]
fn json_tree_package_names(command: &mut Command) -> Result<Vec<String>> {
    let assert = command
        .arg("--preview-features")
        .arg("json-output")
        .arg("--format")
        .arg("json")
        .output()?
        .assert()
        .success();

    let report: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout)?;
    report["resolution"]
        .as_object()
        .context("dependency graph resolution should be an object")?
        .values()
        .filter(|node| node["kind"] == "package")
        .map(|node| {
            node["name"]
                .as_str()
                .context("dependency graph node should have a name")
                .map(ToOwned::to_owned)
        })
        .collect()
}
