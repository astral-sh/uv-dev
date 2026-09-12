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
}

impl Target {
    fn filename(self) -> &'static str {
        match self {
            Self::Configuration => "uv.schema.json",
            Self::WorkspaceMetadata => "docs/reference/internals/metadata.schema.json",
        }
    }

    fn command(self) -> &'static str {
        match self {
            Self::Configuration => "cargo dev generate-json-schema",
            Self::WorkspaceMetadata => "cargo dev generate-json-schema --target workspace-metadata",
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
}
