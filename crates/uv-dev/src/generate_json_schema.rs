use std::path::PathBuf;
use std::process::Command;

use anstream::println;
use anyhow::{Context, Result, bail};
use pretty_assertions::StrComparison;
use schemars::JsonSchema;
use serde::Deserialize;

use uv_settings::Options as SettingsOptions;
use uv_workspace::pyproject::ToolUv as WorkspaceOptions;

use crate::ROOT_DIR;
use crate::generate_all::Mode;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
// The names and docstrings of this struct and the types it contains are used as `title` and
// `description` in uv.schema.json, see https://github.com/SchemaStore/schemastore/blob/master/editor-features.md#title-as-an-expected-object-type
/// Metadata and configuration for uv.
struct CombinedOptions {
    #[serde(flatten)]
    options: SettingsOptions,
    #[serde(flatten)]
    workspace: WorkspaceOptions,
}

#[derive(clap::Args)]
pub(crate) struct Args {
    #[arg(long, default_value_t, value_enum)]
    pub(crate) mode: Mode,
    #[arg(long, default_value_t, value_enum)]
    pub(crate) target: Target,
}

#[derive(Copy, Clone, PartialEq, Eq, clap::ValueEnum, Default)]
pub(crate) enum Target {
    /// Configuration in `uv.toml` and `pyproject.toml`.
    #[default]
    Configuration,
    /// The preview `uv workspace metadata` output format.
    WorkspaceMetadata,
    /// The preview `uv tool list` JSON output format.
    ToolList,
    /// The preview `uv lock` JSON output format.
    Lock,
    /// The preview `uv sync` JSON output format.
    Sync,
    /// The preview `uv pip check` JSON output format.
    PipCheck,
    /// The `uv version` JSON output format.
    Version,
    /// The `uv self version` JSON output format.
    SelfVersion,
    /// The `uv python list` JSON output format.
    PythonList,
    /// Shared progress records in the preview JSONL output format.
    JsonlProgress,
    /// Records in the preview `uv workspace metadata` JSONL output format.
    WorkspaceMetadataJsonl,
    /// Records in the preview `uv tool list` JSONL output format.
    ToolListJsonl,
    /// Records in the preview `uv lock` JSONL output format.
    LockJsonl,
    /// Records in the preview `uv sync` JSONL output format.
    SyncJsonl,
    /// Records in the preview `uv pip check` JSONL output format.
    PipCheckJsonl,
    /// Records in the preview `uv version` JSONL output format.
    VersionJsonl,
    /// Records in the preview `uv self version` JSONL output format.
    SelfVersionJsonl,
    /// Records in the preview `uv python list` JSONL output format.
    PythonListJsonl,
}

impl Target {
    fn filename(self) -> &'static str {
        match self {
            Self::Configuration => "uv.schema.json",
            Self::WorkspaceMetadata => "docs/reference/internals/metadata.schema.json",
            Self::ToolList => "docs/reference/internals/tool-list.schema.json",
            Self::Lock => "docs/reference/internals/lock.schema.json",
            Self::Sync => "docs/reference/internals/sync.schema.json",
            Self::PipCheck => "docs/reference/internals/pip-check.schema.json",
            Self::Version => "docs/reference/internals/version.schema.json",
            Self::SelfVersion => "docs/reference/internals/self-version.schema.json",
            Self::PythonList => "docs/reference/internals/python-list.schema.json",
            Self::JsonlProgress => "docs/reference/internals/jsonl-progress.schema.json",
            Self::WorkspaceMetadataJsonl => "docs/reference/internals/metadata-jsonl.schema.json",
            Self::ToolListJsonl => "docs/reference/internals/tool-list-jsonl.schema.json",
            Self::LockJsonl => "docs/reference/internals/lock-jsonl.schema.json",
            Self::SyncJsonl => "docs/reference/internals/sync-jsonl.schema.json",
            Self::PipCheckJsonl => "docs/reference/internals/pip-check-jsonl.schema.json",
            Self::VersionJsonl => "docs/reference/internals/version-jsonl.schema.json",
            Self::SelfVersionJsonl => "docs/reference/internals/self-version-jsonl.schema.json",
            Self::PythonListJsonl => "docs/reference/internals/python-list-jsonl.schema.json",
        }
    }

    fn command(self) -> &'static str {
        match self {
            Self::Configuration => "cargo dev generate-json-schema",
            Self::WorkspaceMetadata => "cargo dev generate-json-schema --target workspace-metadata",
            Self::ToolList => "cargo dev generate-json-schema --target tool-list",
            Self::Lock => "cargo dev generate-json-schema --target lock",
            Self::Sync => "cargo dev generate-json-schema --target sync",
            Self::PipCheck => "cargo dev generate-json-schema --target pip-check",
            Self::Version => "cargo dev generate-json-schema --target version",
            Self::SelfVersion => "cargo dev generate-json-schema --target self-version",
            Self::PythonList => "cargo dev generate-json-schema --target python-list",
            Self::JsonlProgress => "cargo dev generate-json-schema --target jsonl-progress",
            Self::WorkspaceMetadataJsonl => {
                "cargo dev generate-json-schema --target workspace-metadata-jsonl"
            }
            Self::ToolListJsonl => "cargo dev generate-json-schema --target tool-list-jsonl",
            Self::LockJsonl => "cargo dev generate-json-schema --target lock-jsonl",
            Self::SyncJsonl => "cargo dev generate-json-schema --target sync-jsonl",
            Self::PipCheckJsonl => "cargo dev generate-json-schema --target pip-check-jsonl",
            Self::VersionJsonl => "cargo dev generate-json-schema --target version-jsonl",
            Self::SelfVersionJsonl => "cargo dev generate-json-schema --target self-version-jsonl",
            Self::PythonListJsonl => "cargo dev generate-json-schema --target python-list-jsonl",
        }
    }
}

pub(crate) fn main(args: &Args) -> Result<()> {
    // Generate the schema.
    let schema_string = generate(args.target)?;
    let filename = args.target.filename();
    let command = args.target.command();
    let schema_path = PathBuf::from(ROOT_DIR).join(filename);

    match args.mode {
        Mode::DryRun => {
            println!("{schema_string}");
        }
        Mode::Check => match fs_err::read_to_string(schema_path) {
            Ok(current) => {
                if current == schema_string {
                    println!("Up-to-date: {filename}");
                } else {
                    let comparison = StrComparison::new(&current, &schema_string);
                    bail!("{filename} changed, please run `{command}`:\n{comparison}");
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                bail!("{filename} not found, please run `{command}`");
            }
            Err(err) => {
                bail!("{filename} changed, please run `{command}`:\n{err}");
            }
        },
        Mode::Write => match fs_err::read_to_string(&schema_path) {
            Ok(current) => {
                if current == schema_string {
                    println!("Up-to-date: {filename}");
                } else {
                    println!("Updating: {filename}");
                    fs_err::write(schema_path, schema_string.as_bytes())?;
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                println!("Updating: {filename}");
                fs_err::write(schema_path, schema_string.as_bytes())?;
            }
            Err(err) => {
                bail!("{filename} changed, please run `{command}`:\n{err}");
            }
        },
    }

    Ok(())
}

const REPLACEMENTS: &[(&str, &str)] = &[
    // Use the fully-resolved URL rather than the relative Markdown path.
    (
        "(../concepts/projects/dependencies.md)",
        "(https://docs.astral.sh/uv/concepts/projects/dependencies/)",
    ),
];

fn schema(target: Target) -> schemars::Schema {
    let settings = schemars::generate::SchemaSettings::draft07();
    match target {
        Target::Configuration => settings
            .into_generator()
            .into_root_schema_for::<CombinedOptions>(),
        Target::WorkspaceMetadata => settings
            .for_serialize()
            .into_generator()
            .into_root_schema_for::<uv_resolver::Metadata>(),
        Target::ToolList => uv::commands::tool_list_json_schema(),
        Target::Lock => uv::commands::lock_json_schema(),
        Target::Sync => uv::commands::sync_json_schema(),
        Target::PipCheck => uv::commands::pip_check_json_schema(),
        Target::Version => uv::commands::version_json_schema(),
        Target::SelfVersion => uv::commands::self_version_json_schema(),
        Target::PythonList => uv::commands::python_list_json_schema(),
        Target::JsonlProgress => uv::commands::jsonl_progress_json_schema(),
        Target::WorkspaceMetadataJsonl => uv::commands::workspace_metadata_jsonl_schema(),
        Target::ToolListJsonl => uv::commands::tool_list_jsonl_schema(),
        Target::LockJsonl => uv::commands::lock_jsonl_schema(),
        Target::SyncJsonl => uv::commands::sync_jsonl_schema(),
        Target::PipCheckJsonl => uv::commands::pip_check_jsonl_schema(),
        Target::VersionJsonl => uv::commands::version_jsonl_schema(),
        Target::SelfVersionJsonl => uv::commands::self_version_jsonl_schema(),
        Target::PythonListJsonl => uv::commands::python_list_jsonl_schema(),
    }
}

/// Generate a JSON schema as a formatted string.
fn generate(target: Target) -> Result<String> {
    let json = serde_json::to_string_pretty(&schema(target))?;

    // Format with prettier
    let mut output = Command::new("npx")
        .args(["prettier@3.9.0", "--stdin-filepath", target.filename()])
        .current_dir(ROOT_DIR)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("Failed to spawn prettier")?;

    let mut stdin = output.stdin.take().context("Missing prettier stdin")?;
    std::io::Write::write_all(&mut stdin, json.as_bytes())
        .context("Failed to write to prettier stdin")?;
    drop(stdin);

    let output = output
        .wait_with_output()
        .context("Failed to run prettier")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("prettier failed: {stderr}");
    }

    let mut output = String::from_utf8(output.stdout).context("prettier output is not UTF-8")?;

    if target == Target::Configuration {
        for (value, replacement) in REPLACEMENTS {
            assert_ne!(
                value, replacement,
                "`value` and `replacement` must be different, but both are `{value}`"
            );
            let before = &output;
            let after = output.replace(value, replacement);
            assert_ne!(*before, after, "Could not find `{value}` in the output");
            output = after;
        }
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use anyhow::Context;
    use serde_json::{Value, json};
    use uv_cli::version::{ProjectVersionInfo, uv_self_version};
    use uv_test::json_schema::JsonSchema;

    use super::{Target, schema};

    #[test]
    fn jsonl_record_schemas_validate_both_variants() -> anyhow::Result<()> {
        let environment = json!({
            "path": "/project/.venv",
            "python": {
                "path": "/project/.venv/bin/python",
                "version": "3.12.0",
                "implementation": "cpython",
                "key": "cpython-3.12.0-linux-x86_64-gnu"
            }
        });
        let cases = [
            (
                Target::WorkspaceMetadata,
                Target::WorkspaceMetadataJsonl,
                "uv workspace metadata JSONL (preview)",
                json!({
                    "schema": {"version": "preview"},
                    "workspace_root": "/project",
                    "requires_python": ">=3.12",
                    "conflicts": {"sets": []}
                }),
            ),
            (
                Target::ToolList,
                Target::ToolListJsonl,
                "uv tool list JSONL (preview)",
                json!({"schema": {"version": "preview"}, "tools": []}),
            ),
            (
                Target::Lock,
                Target::LockJsonl,
                "uv lock JSONL (preview)",
                json!({
                    "schema": {"version": "preview"},
                    "status": "not_checked",
                    "dry_run": false
                }),
            ),
            (
                Target::Sync,
                Target::SyncJsonl,
                "uv sync JSONL (preview)",
                json!({
                    "schema": {"version": "preview"},
                    "target": "project",
                    "sync": {"environment": environment, "action": "check", "changes": []},
                    "lock": null,
                    "dry_run": false
                }),
            ),
            (
                Target::PipCheck,
                Target::PipCheckJsonl,
                "uv pip check JSONL (preview)",
                json!({
                    "schema": {"version": "preview"},
                    "environment": environment,
                    "target": {"python_version": "3.12.0", "python_platform": null},
                    "packages_checked": 0,
                    "diagnostics": []
                }),
            ),
        ];

        for (object_target, record_target, title, mut result) in cases {
            JsonSchema::new(&serde_json::to_string(&schema(object_target))?)?
                .parse(&serde_json::to_vec(&result)?)?;
            let document = serde_json::to_value(schema(record_target))?;
            assert_eq!(document["title"], title);
            assert_eq!(document["anyOf"].as_array().map(Vec::len), Some(2));
            assert_eq!(
                document["definitions"]["ResultType"]["enum"],
                json!(["result"])
            );
            let validator = JsonSchema::new(&serde_json::to_string(&document)?)?;
            validator.parse(br#"{"type":"progress","phase":"resolve","status":"started"}"#)?;
            result["type"] = json!("result");
            validator.parse(&serde_json::to_vec(&result)?)?;

            for discriminator in [Value::Null, json!("progress"), json!("unknown")] {
                result["type"] = discriminator;
                assert!(validator.parse(&serde_json::to_vec(&result)?).is_err());
            }
            result["type"] = json!("result");
            result["schema"]["version"] = json!(1);
            assert!(validator.parse(&serde_json::to_vec(&result)?).is_err());
            assert!(validator.parse(br#"{"type":"result"}"#).is_err());
            assert!(
                validator
                    .parse(br#"{"type":"progress","phase":"resolve"}"#)
                    .is_err()
            );
        }
        Ok(())
    }

    #[test]
    fn version_schemas_describe_serialized_values() -> anyhow::Result<()> {
        let name = "project".parse()?;
        let version = "1.2.3".parse()?;
        let project = serde_json::to_value(ProjectVersionInfo::new(Some(&name), &version))?;
        let anonymous = serde_json::to_value(ProjectVersionInfo::new(None, &version))?;
        let self_version = serde_json::to_value(uv_self_version())?;

        let project_schema = serde_json::to_value(schema(Target::Version))?;
        assert_eq!(project_schema["title"], "uv version");
        assert_eq!(
            project_schema["required"],
            json!(["package_name", "version", "commit_info"])
        );
        assert_eq!(project_schema["properties"]["commit_info"]["type"], "null");
        assert_eq!(
            project_schema["properties"]["package_name"]["type"],
            json!(["string", "null"])
        );
        let project_validator = JsonSchema::new(&serde_json::to_string(&project_schema)?)?;
        project_validator.parse(&serde_json::to_vec(&project)?)?;
        project_validator.parse(&serde_json::to_vec(&anonymous)?)?;
        for invalid_commit_info in [json!({}), json!(false), json!("unknown")] {
            let mut invalid = project.clone();
            invalid["commit_info"] = invalid_commit_info;
            assert!(
                project_validator
                    .parse(&serde_json::to_vec(&invalid)?)
                    .is_err()
            );
        }

        let self_schema = serde_json::to_value(schema(Target::SelfVersion))?;
        assert_eq!(self_schema["title"], "uv self version");
        assert_eq!(
            self_schema["required"],
            json!(["package_name", "version", "commit_info", "target_triple"])
        );
        let commits = &self_schema["definitions"]["CommitInfo"]["properties"];
        assert_eq!(commits["last_tag"]["type"], json!(["string", "null"]));
        assert_eq!(commits["commits_since_last_tag"]["type"], "integer");
        assert_eq!(commits["commits_since_last_tag"]["minimum"], 0);
        assert_eq!(commits["commits_since_last_tag"]["maximum"], u32::MAX);
        let self_validator = JsonSchema::new(&serde_json::to_string(&self_schema)?)?;
        self_validator.parse(&serde_json::to_vec(&self_version)?)?;
        let mut from_tarball = self_version.clone();
        from_tarball["commit_info"] = Value::Null;
        self_validator.parse(&serde_json::to_vec(&from_tarball)?)?;
        let mut from_git = self_version.clone();
        from_git["commit_info"] = json!({
            "short_commit_hash": "53b0f5d92",
            "commit_hash": "53b0f5d924110e5b26fbf09f6fd3a03d67b475b7",
            "commit_date": "2023-10-19",
            "last_tag": null,
            "commits_since_last_tag": 0
        });
        self_validator.parse(&serde_json::to_vec(&from_git)?)?;
        for invalid_distance in [json!(-1), json!(u64::from(u32::MAX) + 1), json!(0.5)] {
            let mut invalid = from_git.clone();
            invalid["commit_info"]["commits_since_last_tag"] = invalid_distance;
            assert!(
                self_validator
                    .parse(&serde_json::to_vec(&invalid)?)
                    .is_err()
            );
        }

        for (validator, report, fields) in [
            (
                &project_validator,
                &project,
                ["package_name", "version", "commit_info"].as_slice(),
            ),
            (
                &self_validator,
                &self_version,
                ["package_name", "version", "commit_info", "target_triple"].as_slice(),
            ),
        ] {
            for field in fields {
                let mut missing = report.clone();
                missing
                    .as_object_mut()
                    .context("version report is not an object")?
                    .remove(*field);
                assert!(validator.parse(&serde_json::to_vec(&missing)?).is_err());
            }
            let mut invalid = report.clone();
            invalid["version"] = Value::Null;
            assert!(validator.parse(&serde_json::to_vec(&invalid)?).is_err());
        }
        Ok(())
    }

    #[test]
    fn version_jsonl_record_schemas_validate_both_variants() -> anyhow::Result<()> {
        let name = "project".parse()?;
        let version = "1.2.3".parse()?;
        let cases = [
            (
                Target::Version,
                Target::VersionJsonl,
                "uv version JSONL (preview)",
                serde_json::to_value(ProjectVersionInfo::new(Some(&name), &version))?,
            ),
            (
                Target::SelfVersion,
                Target::SelfVersionJsonl,
                "uv self version JSONL (preview)",
                serde_json::to_value(uv_self_version())?,
            ),
        ];

        for (object_target, record_target, title, mut result) in cases {
            JsonSchema::new(&serde_json::to_string(&schema(object_target))?)?
                .parse(&serde_json::to_vec(&result)?)?;
            assert!(result.get("schema").is_none());
            let document = serde_json::to_value(schema(record_target))?;
            assert_eq!(document["title"], title);
            assert_eq!(document["anyOf"].as_array().map(Vec::len), Some(2));
            let validator = JsonSchema::new(&serde_json::to_string(&document)?)?;
            validator.parse(br#"{"type":"progress","phase":"resolve","status":"started"}"#)?;
            result["type"] = json!("result");
            validator.parse(&serde_json::to_vec(&result)?)?;

            for discriminator in [Value::Null, json!("progress"), json!("unknown")] {
                result["type"] = discriminator;
                assert!(validator.parse(&serde_json::to_vec(&result)?).is_err());
            }
            result["type"] = json!("result");
            result["version"] = json!(1);
            assert!(validator.parse(&serde_json::to_vec(&result)?).is_err());
            assert!(validator.parse(br#"{"type":"result"}"#).is_err());
            assert!(
                validator
                    .parse(br#"{"type":"progress","phase":"resolve"}"#)
                    .is_err()
            );
        }
        Ok(())
    }

    #[test]
    fn python_list_record_schema_uses_an_array_envelope() -> anyhow::Result<()> {
        let document = serde_json::to_value(schema(Target::PythonList))?;
        assert_eq!(document["title"], "uv python list");
        assert_eq!(document["type"], "array");
        let validator = JsonSchema::new(&serde_json::to_string(&document)?)?;
        validator.parse(b"[]")?;
        assert!(validator.parse(b"{}").is_err());

        let document = serde_json::to_value(schema(Target::PythonListJsonl))?;
        assert_eq!(document["title"], "uv python list JSONL (preview)");
        assert_eq!(document["anyOf"].as_array().map(Vec::len), Some(2));
        let validator = JsonSchema::new(&serde_json::to_string(&document)?)?;
        validator.parse(br#"{"type":"result","data":[]}"#)?;
        validator.parse(br#"{"type":"progress","phase":"resolve","status":"started"}"#)?;
        for invalid in [
            br#"{"type":"result"}"#.as_slice(),
            br#"{"type":"result","data":{}}"#,
            br#"{"type":"result","data":[{}]}"#,
            br#"{"type":"unknown","data":[]}"#,
        ] {
            assert!(validator.parse(invalid).is_err());
        }
        Ok(())
    }

    #[test]
    fn jsonl_progress_schema_describes_serialized_values() -> anyhow::Result<()> {
        let schema = serde_json::to_value(schema(Target::JsonlProgress))?;
        let definitions = &schema["definitions"];
        assert_eq!(schema["title"], "uv JSONL progress (preview)");
        assert_eq!(
            schema["required"],
            serde_json::json!(["type", "phase", "status"])
        );
        assert_eq!(
            definitions["ProgressType"]["enum"],
            serde_json::json!(["progress"])
        );
        assert_eq!(
            definitions["ProgressStatus"]["enum"],
            serde_json::json!(["started", "updated", "completed"])
        );
        assert_eq!(
            definitions["ProgressPhase"]["enum"],
            serde_json::json!([
                "audit",
                "build",
                "checkout",
                "download",
                "extract",
                "hash",
                "install",
                "latest_version",
                "prepare",
                "resolve",
                "upload"
            ])
        );
        for field in ["name", "version", "url", "revision"] {
            assert_eq!(schema["properties"][field]["type"], "string");
        }
        for field in ["id", "completed", "total"] {
            assert_eq!(schema["properties"][field]["type"], "integer");
            assert_eq!(schema["properties"][field]["minimum"], 0);
        }
        Ok(())
    }

    #[test]
    fn workspace_metadata_schema_describes_serialized_values() -> anyhow::Result<()> {
        let schema = serde_json::to_value(schema(Target::WorkspaceMetadata))?;
        let definitions = &schema["definitions"];

        assert_eq!(schema["title"], "uv workspace metadata (preview)");
        assert_eq!(definitions["SchemaVersion"]["oneOf"][0]["const"], "preview");
        assert_eq!(
            definitions["MetadataInstalledPackage"]["properties"]["version"]["type"],
            "string"
        );
        assert!(
            definitions["MetadataInstalledPackage"]["required"]
                .as_array()
                .is_some_and(|fields| !fields.iter().any(|field| field == "direct_url"))
        );
        assert_eq!(
            definitions["MetadataInstalledPackage"]["properties"]["direct_url"]["allOf"][0]["$ref"],
            "#/definitions/MetadataDirectUrl"
        );
        for variant in definitions["DirectUrl"]["anyOf"]
            .as_array()
            .expect("direct URL variants")
        {
            assert_eq!(variant["properties"]["url"]["format"], "uri");
            assert_eq!(variant["properties"]["subdirectory"]["type"], "string");
        }
        assert_eq!(
            definitions["VcsInfo"]["properties"]["commit_id"]["type"],
            "string"
        );
        assert_eq!(
            definitions["DirInfo"]["properties"]["editable"]["type"],
            "boolean"
        );
        for field in ["requires_dist", "provides_extra"] {
            assert!(
                definitions["MetadataInstalledPackage"]["required"]
                    .as_array()
                    .is_some_and(|fields| fields.iter().any(|required| required == field))
            );
            assert_eq!(
                definitions["MetadataInstalledPackage"]["properties"][field]["type"],
                "array"
            );
        }
        assert_eq!(
            definitions["MetadataInstalledPackage"]["properties"]["requires_dist"]["items"]["$ref"],
            "#/definitions/Requirement"
        );
        assert_eq!(definitions["Requirement"]["type"], "string");
        assert_eq!(
            definitions["MetadataInstalledPackage"]["properties"]["requires_python"]["type"],
            "string"
        );
        assert_eq!(
            definitions["PythonReport"]["properties"]["version"]["type"],
            "string"
        );
        assert_eq!(
            definitions["PythonReport"]["properties"]["key"]["type"],
            "string"
        );

        // Module names can contain non-ASCII identifiers, including combining characters.
        // Keep their object keys unrestricted rather than approximating Python's identifier rules.
        for owners in [
            &schema["properties"]["module_owners"],
            &definitions["MetadataEnvironment"]["properties"]["module_owners"],
        ] {
            assert!(owners.get("patternProperties").is_none());
            assert_eq!(owners["additionalProperties"]["type"], "array");
        }

        Ok(())
    }

    #[test]
    fn tool_list_schema_describes_serialized_values() -> anyhow::Result<()> {
        let schema = serde_json::to_value(schema(Target::ToolList))?;
        let definitions = &schema["definitions"];
        let tool = &definitions["ToolReport"];

        assert_eq!(schema["title"], "uv tool list (preview)");
        assert_eq!(definitions["SchemaVersion"]["oneOf"][0]["const"], "preview");
        assert_eq!(tool["properties"]["version"]["type"], "string");
        assert_eq!(
            tool["properties"]["latest_version"]["type"],
            serde_json::json!(["string", "null"])
        );
        assert!(
            tool["required"]
                .as_array()
                .is_some_and(|fields| fields.iter().any(|field| field == "latest_version"))
        );
        assert_eq!(
            definitions["PythonReport"]["properties"]["key"]["type"],
            "string"
        );
        assert_eq!(
            definitions["CommandReport"]["properties"]["name"]["type"],
            "string"
        );

        Ok(())
    }

    #[test]
    fn lock_schema_describes_serialized_values() -> anyhow::Result<()> {
        let schema = serde_json::to_value(schema(Target::Lock))?;
        let definitions = &schema["definitions"];
        let reason = &definitions["LockReason"];
        let error = &definitions["ErrorReport"];
        let required = schema["required"].as_array().expect("required fields");

        assert_eq!(schema["title"], "uv lock (preview)");
        assert_eq!(definitions["SchemaVersion"]["oneOf"][0]["const"], "preview");
        assert_eq!(
            schema["properties"]["path"]["allOf"][0]["$ref"],
            "#/definitions/PortablePathBuf"
        );
        assert_eq!(definitions["PortablePathBuf"]["type"], "string");
        assert_eq!(schema["properties"]["dry_run"]["type"], "boolean");
        for field in ["schema", "status", "dry_run"] {
            assert!(required.iter().any(|required| required == field));
        }
        for field in ["path", "action", "reason", "validation_error", "error"] {
            assert!(!required.iter().any(|required| required == field));
        }
        for field in ["completed", "had_existing_lockfile"] {
            assert!(schema["properties"].get(field).is_none());
        }
        assert_eq!(
            definitions["Action"]["enum"],
            serde_json::json!(["use", "check", "update", "create"])
        );
        let mut statuses = Vec::new();
        for variant in definitions["Status"]["oneOf"]
            .as_array()
            .expect("status variants")
        {
            if let Some(value) = variant.get("const") {
                statuses.push(value.as_str().expect("status value"));
            } else {
                statuses.extend(
                    variant["enum"]
                        .as_array()
                        .expect("status values")
                        .iter()
                        .map(|value| value.as_str().expect("status value")),
                );
            }
        }
        statuses.sort_unstable();
        assert_eq!(
            statuses,
            vec!["fresh", "indeterminate", "not_checked", "stale"]
        );
        assert_eq!(
            reason["properties"]["package"]["allOf"][0]["$ref"],
            "#/definitions/PackageName"
        );
        assert_eq!(definitions["PackageName"]["type"], "string");
        for field in ["expected", "actual"] {
            assert_eq!(reason["properties"][field]["type"], "array");
            assert_eq!(reason["properties"][field]["items"]["type"], "string");
        }
        assert_eq!(error["properties"]["message"]["type"], "string");
        assert_eq!(error["properties"]["http_status"]["type"], "integer");
        assert_eq!(error["properties"]["http_status"]["minimum"], 0);
        assert_eq!(error["properties"]["http_status"]["maximum"], 65535);
        assert!(error["properties"].get("causes").is_none());
        assert_eq!(
            definitions["ErrorCode"]["enum"],
            serde_json::json!([
                "evaluation_failed",
                "metadata_unavailable",
                "offline_cache_miss",
                "authentication",
                "access_denied",
                "http",
                "network"
            ])
        );
        Ok(())
    }

    #[test]
    fn pip_check_schema_describes_serialized_values() -> anyhow::Result<()> {
        let schema = serde_json::to_value(schema(Target::PipCheck))?;
        let definitions = &schema["definitions"];
        let target = &definitions["CheckTargetReport"];
        let diagnostic = &definitions["DiagnosticReport"];

        assert_eq!(schema["title"], "uv pip check (preview)");
        assert_eq!(definitions["SchemaVersion"]["oneOf"][0]["const"], "preview");
        assert_eq!(schema["properties"]["packages_checked"]["type"], "integer");
        assert_eq!(schema["properties"]["packages_checked"]["minimum"], 0);
        assert_eq!(target["properties"]["python_version"]["type"], "string");
        assert_eq!(
            target["properties"]["python_platform"]["type"],
            serde_json::json!(["string", "null"])
        );
        assert!(
            target["required"]
                .as_array()
                .is_some_and(|fields| { fields.iter().any(|field| field == "python_platform") })
        );
        assert!(
            diagnostic["required"]
                .as_array()
                .is_some_and(|fields| { fields.iter().any(|field| field == "package") })
        );
        let variants = diagnostic["oneOf"].as_array().expect("diagnostic variants");
        assert_eq!(
            variants
                .iter()
                .map(|variant| {
                    variant["properties"]["kind"]["const"]
                        .as_str()
                        .expect("diagnostic kind")
                })
                .collect::<Vec<_>>(),
            vec![
                "duplicate_package",
                "incompatible_dependency",
                "incompatible_platform",
                "incompatible_python_version",
                "metadata_unavailable",
                "missing_dependency",
                "tags_unavailable",
            ]
        );
        for (kind, field) in [
            ("incompatible_dependency", "requirement"),
            ("incompatible_dependency", "installed_version"),
            ("incompatible_python_version", "requires_python"),
            ("incompatible_python_version", "installed_version"),
            ("metadata_unavailable", "path"),
            ("missing_dependency", "requirement"),
            ("tags_unavailable", "path"),
        ] {
            let variant = variants
                .iter()
                .find(|variant| variant["properties"]["kind"]["const"] == kind)
                .expect("named diagnostic variant");
            assert_eq!(variant["properties"][field]["type"], "string");
            assert!(
                variant["required"]
                    .as_array()
                    .is_some_and(|fields| { fields.iter().any(|required| required == field) })
            );
        }
        Ok(())
    }

    #[test]
    fn sync_schema_describes_serialized_values() -> anyhow::Result<()> {
        let schema = serde_json::to_value(schema(Target::Sync))?;
        let definitions = &schema["definitions"];
        let sync = &definitions["SyncReport"];
        let package = &definitions["PackageChangeReport"];

        assert_eq!(schema["title"], "uv sync (preview)");
        assert_eq!(definitions["SchemaVersion"]["oneOf"][0]["const"], "preview");
        assert_eq!(
            definitions["PythonReport"]["properties"]["key"]["type"],
            "string"
        );
        assert_eq!(package["properties"]["version"]["type"], "string");
        assert!(
            package["required"]
                .as_array()
                .is_some_and(|fields| fields.iter().all(|field| field != "version"))
        );
        assert_eq!(definitions["PackageChangesReport"]["type"], "array");
        assert_eq!(
            schema["properties"]["project"]["allOf"][0]["$ref"],
            "#/definitions/ProjectReport"
        );
        assert_eq!(
            schema["properties"]["script"]["allOf"][0]["$ref"],
            "#/definitions/ScriptReport"
        );
        assert_eq!(schema["properties"]["lock"]["anyOf"][1]["type"], "null");
        assert!(sync["properties"].get("dry_run").is_none());
        assert!(sync["properties"].get("target").is_none());
        assert!(
            definitions["LockReport"]["properties"]
                .get("dry_run")
                .is_none()
        );
        assert!(
            schema["required"]
                .as_array()
                .is_some_and(|fields| fields.iter().any(|field| field == "lock"))
        );
        assert!(schema["required"].as_array().is_some_and(|fields| {
            fields
                .iter()
                .all(|field| field != "project" && field != "script")
        }));

        Ok(())
    }
}
