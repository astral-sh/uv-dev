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
    use super::{Target, schema};

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
