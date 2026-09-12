use std::env;

use anyhow::Result;
use clap::Parser;
use tracing::instrument;

use uv_settings::EnvironmentOptions;

use crate::check_scenarios::Args as CheckScenariosArgs;
use crate::clear_compile::ClearCompileArgs;
use crate::compile::CompileArgs;
use crate::generate_all::Args as GenerateAllArgs;
use crate::generate_cli_reference::Args as GenerateCliReferenceArgs;
use crate::generate_dirhash_test_vectors::Args as GenerateDirhashTestVectorsArgs;
use crate::generate_env_vars_reference::Args as GenerateEnvVarsReferenceArgs;
use crate::generate_json_schema::Args as GenerateJsonSchemaArgs;
use crate::generate_options_reference::Args as GenerateOptionsReferenceArgs;
use crate::generate_preview_features_reference::Args as GeneratePreviewFeaturesReferenceArgs;
use crate::generate_scenarios::Args as GenerateScenarioTestsArgs;
use crate::generate_sysconfig_mappings::Args as GenerateSysconfigMetadataArgs;
use crate::list_packages::ListPackagesArgs;
use crate::minimize_scenario::Args as MinimizeScenarioArgs;
#[cfg(feature = "render")]
use crate::render_benchmarks::RenderBenchmarksArgs;
use crate::validate_zip::ValidateZipArgs;
use crate::wheel_metadata::WheelMetadataArgs;

mod check_scenarios;
mod clear_compile;
mod compile;
mod generate_all;
mod generate_cli_reference;
mod generate_dirhash_test_vectors;
mod generate_env_vars_reference;
mod generate_json_schema;
mod generate_options_reference;
mod generate_preview_features_reference;
mod generate_scenarios;
mod generate_sysconfig_mappings;
mod list_packages;
mod minimize_scenario;
mod render_benchmarks;
mod validate_zip;
mod wheel_metadata;

const ROOT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../");

#[derive(Parser)]
enum Cli {
    /// Check small Packse scenarios against an exhaustive resolver oracle.
    CheckScenarios(CheckScenariosArgs),
    /// Reduce a fixed-environment resolver counterexample to a replayable Packse fixture.
    MinimizeScenario(MinimizeScenarioArgs),
    /// Display the metadata for a `.whl` at a given URL.
    WheelMetadata(WheelMetadataArgs),
    /// Validate that a `.whl` or `.zip` file at a given URL is a valid ZIP file.
    ValidateZip(ValidateZipArgs),
    /// Compile all `.py` to `.pyc` files in the tree.
    Compile(CompileArgs),
    /// Remove all `.pyc` in the tree.
    ClearCompile(ClearCompileArgs),
    /// List all packages from a Simple API index.
    ListPackages(ListPackagesArgs),
    /// Run all code and documentation generation steps.
    GenerateAll(GenerateAllArgs),
    /// Generate JSON schema for the TOML configuration file.
    GenerateJSONSchema(GenerateJsonSchemaArgs),
    /// Generate the options reference for the documentation.
    GenerateOptionsReference(GenerateOptionsReferenceArgs),
    /// Generate the CLI reference for the documentation.
    GenerateCliReference(GenerateCliReferenceArgs),
    /// Generate the uv-extract dirhash test vectors.
    GenerateDirhashTestVectors(GenerateDirhashTestVectorsArgs),
    /// Generate the environment variables reference for the documentation.
    GenerateEnvVarsReference(GenerateEnvVarsReferenceArgs),
    /// Generate the available preview features reference for the documentation.
    GeneratePreviewFeaturesReference(GeneratePreviewFeaturesReferenceArgs),
    /// Generate the Packse scenario integration tests.
    GenerateScenarioTests(GenerateScenarioTestsArgs),
    /// Generate the sysconfig metadata from derived targets.
    GenerateSysconfigMetadata(GenerateSysconfigMetadataArgs),
    #[cfg(feature = "render")]
    /// Render the benchmarks.
    RenderBenchmarks(RenderBenchmarksArgs),
}

#[instrument] // Anchor span to check for overhead
pub async fn run() -> Result<()> {
    let cli = Cli::parse();
    // Scenario checks reuse the test harness, which scopes preview state while discovering its
    // Python interpreters. That state cannot be installed after normal process initialization.
    if !matches!(&cli, Cli::CheckScenarios(_) | Cli::MinimizeScenario(_)) {
        uv_preview::set(uv_preview::Preview::default())?;
        uv_preview::finalize()?;
    }

    let environment = EnvironmentOptions::new()?;
    match cli {
        Cli::CheckScenarios(args) => check_scenarios::main(&args)?,
        Cli::MinimizeScenario(args) => minimize_scenario::main(&args)?,
        Cli::WheelMetadata(args) => wheel_metadata::wheel_metadata(args, environment).await?,
        Cli::ValidateZip(args) => validate_zip::validate_zip(args, environment).await?,
        Cli::Compile(args) => compile::compile(args).await?,
        Cli::ClearCompile(args) => clear_compile::clear_compile(&args)?,
        Cli::ListPackages(args) => list_packages::list_packages(args, environment).await?,
        Cli::GenerateAll(args) => generate_all::main(&args).await?,
        Cli::GenerateJSONSchema(args) => generate_json_schema::main(&args)?,
        Cli::GenerateOptionsReference(args) => generate_options_reference::main(&args)?,
        Cli::GenerateCliReference(args) => generate_cli_reference::main(&args)?,
        Cli::GenerateDirhashTestVectors(args) => generate_dirhash_test_vectors::main(&args)?,
        Cli::GenerateEnvVarsReference(args) => generate_env_vars_reference::main(&args)?,
        Cli::GeneratePreviewFeaturesReference(args) => {
            generate_preview_features_reference::main(&args)?;
        }
        Cli::GenerateScenarioTests(args) => generate_scenarios::main(&args)?,
        Cli::GenerateSysconfigMetadata(args) => generate_sysconfig_mappings::main(&args).await?,
        #[cfg(feature = "render")]
        Cli::RenderBenchmarks(args) => render_benchmarks::render_benchmarks(&args)?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser};

    use super::Cli;

    #[test]
    fn scenario_tests_command_uses_explicit_name() {
        let command = Cli::command();

        assert!(command.find_subcommand("generate-scenario-tests").is_some());
        assert!(command.find_subcommand("generate-scenarios").is_none());
    }

    #[test]
    fn scenario_checker_command_is_registered() {
        let command = Cli::command();
        assert!(command.find_subcommand("check-scenarios").is_some());
    }

    #[test]
    fn scenario_checker_accepts_files_or_saved_generated_inputs() {
        assert!(
            Cli::try_parse_from(["uv-dev", "check-scenarios", "--uv", "uv", "scenario.toml"])
                .is_ok()
        );
        assert!(
            Cli::try_parse_from(["uv-dev", "check-scenarios", "--uv", "uv", "--seed", "0"])
                .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "uv-dev",
                "check-scenarios",
                "--uv",
                "uv",
                "--seed",
                "0",
                "--output-dir",
                "generated",
            ])
            .is_ok()
        );
    }

    #[test]
    fn scenario_checker_captures_resolver_failures() {
        let arguments = [
            "uv-dev",
            "check-scenarios",
            "--uv",
            "uv",
            "--failure-dir",
            "failure",
            "scenario.toml",
        ];
        assert!(Cli::try_parse_from(arguments).is_ok());
        assert!(Cli::try_parse_from(arguments.into_iter().chain(["--lock"])).is_ok());
    }

    #[test]
    fn scenario_checker_requires_a_lock_for_project_selections() {
        let arguments = [
            "uv-dev",
            "check-scenarios",
            "--uv",
            "uv",
            "--project-selections",
            "scenario.toml",
        ];
        assert!(Cli::try_parse_from(arguments).is_err());
        assert!(Cli::try_parse_from(arguments.into_iter().chain(["--lock"])).is_ok());
    }

    #[test]
    fn scenario_checker_accepts_target_matrices() {
        let arguments = [
            "uv-dev",
            "check-scenarios",
            "--uv",
            "uv",
            "--python-version",
            "3.12,3.13",
            "--python-platform",
            "linux,windows",
            "scenario.toml",
        ];
        assert!(Cli::try_parse_from(arguments).is_ok());
        assert!(Cli::try_parse_from(arguments.into_iter().chain(["--lock"])).is_ok());
    }

    #[test]
    fn marker_graphs_require_a_saved_seed() {
        let arguments = ["uv-dev", "check-scenarios", "--uv", "uv", "--markers"];
        assert!(Cli::try_parse_from(arguments).is_err());
        assert!(
            Cli::try_parse_from(arguments.into_iter().chain([
                "--seed",
                "0",
                "--output-dir",
                "generated",
                "--python-version",
                "3.12,3.13,3.14",
                "--python-platform",
                "linux,macos,windows",
            ]))
            .is_ok()
        );
    }

    #[test]
    fn scenario_reducer_requires_a_replay_destination() {
        let arguments = ["uv-dev", "minimize-scenario", "--uv", "uv", "scenario.toml"];
        assert!(Cli::try_parse_from(arguments).is_err());
        assert!(
            Cli::try_parse_from(arguments.into_iter().chain(["--output", "reduced.toml"])).is_ok()
        );
    }

    #[test]
    fn scenario_reducer_accepts_lock_projection_matrices() {
        let arguments = [
            "uv-dev",
            "minimize-scenario",
            "--uv",
            "uv",
            "--output",
            "reduced.toml",
            "--python-version",
            "3.12,3.13",
            "--python-platform",
            "linux,windows",
            "--project-selections",
            "scenario.toml",
        ];
        assert!(Cli::try_parse_from(arguments).is_err());
        assert!(Cli::try_parse_from(arguments.into_iter().chain(["--lock"])).is_ok());
    }
}
