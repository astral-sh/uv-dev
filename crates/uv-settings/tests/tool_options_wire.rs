use std::error::Error;

use uv_settings::{ToolOptions, ToolOptionsWire};
use uv_static::EnvVars;

#[test]
fn borrowed_options_match_owned_wire() -> Result<(), Box<dyn Error>> {
    temp_env::with_var(
        EnvVars::UV_INTERNAL__TEST_CURRENT_TIMESTAMP,
        Some("2024-03-25T00:00:00Z"),
        || {
            for source in [
                "",
                r#"
                index = [{ name = "private", url = "https://user:password@example.com/simple", explicit = true }]
                index-url = "https://example.org/simple"
                extra-index-url = ["https://extra.example.org/simple"]
                no-index = false
                find-links = ["https://files.example.org/wheels"]
                index-strategy = "unsafe-best-match"
                keyring-provider = "subprocess"
                resolution = "lowest-direct"
                prerelease = "if-necessary-or-explicit"
                prerelease-package = { example = "allow" }
                fork-strategy = "requires-python"
                dependency-metadata = [{ name = "example", version = "1.0", requires-dist = ["other>=1"], provides-extra = ["feature"] }]
                config-settings = { key = ["a", "b"] }
                config-settings-package = { example = { key = "value" } }
                build-isolation = { shared-package = ["example"] }
                extra-build-dependencies = { example = ["other>=1"] }
                extra-build-variables = { example = { FLAG = "value" } }
                exclude-newer = false
                exclude-newer-package = { example = "7 days", other = false }
                link-mode = "copy"
                compile-bytecode = false
                no-sources = false
                no-sources-package = ["example"]
                no-build = false
                no-build-package = ["example"]
                no-binary = false
                no-binary-package = ["example"]
                torch-backend = "cpu"
                "#,
            ] {
                let options: ToolOptions = toml::from_str(source)?;
                let borrowed = toml::to_string(&options.as_wire())?;
                let owned = toml::to_string(&ToolOptionsWire::from(options.clone()))?;
                assert_eq!(borrowed, owned);

                let restored = ToolOptions::from(toml::from_str::<ToolOptionsWire>(&borrowed)?);
                assert_eq!(toml::to_string(&restored.as_wire())?, borrowed);
            }
            Ok(())
        },
    )
}

#[test]
fn borrowed_options_materialize_cutoffs_with_spans() -> Result<(), Box<dyn Error>> {
    temp_env::with_var(
        EnvVars::UV_INTERNAL__TEST_CURRENT_TIMESTAMP,
        Some("2024-03-25T00:00:00Z"),
        || {
            for (source, expected) in [
                ("", ""),
                ("exclude-newer = false", "exclude-newer = false"),
                (
                    "exclude-newer = '2024-03-01T00:00:00Z'",
                    "exclude-newer = '2024-03-01T00:00:00Z'",
                ),
                (
                    "exclude-newer = '7 days'",
                    "exclude-newer = '2024-03-18T00:00:00Z'\nexclude-newer-span = 'P7D'",
                ),
                (
                    "exclude-newer-package = { absolute = '2024-03-01T00:00:00Z', relative = '7 days', disabled = false }",
                    "exclude-newer-package = { absolute = '2024-03-01T00:00:00Z', relative = { timestamp = '2024-03-18T00:00:00Z', span = 'P7D' }, disabled = false }",
                ),
            ] {
                let options: ToolOptions = toml::from_str(source)?;
                let borrowed = toml::to_string(&options.as_wire())?;
                assert_eq!(
                    toml::from_str::<toml::Value>(&borrowed)?,
                    toml::from_str::<toml::Value>(expected)?
                );
                assert_eq!(borrowed, toml::to_string(&ToolOptionsWire::from(options))?);

                let restored = ToolOptions::from(toml::from_str::<ToolOptionsWire>(&borrowed)?);
                assert_eq!(toml::to_string(&restored.as_wire())?, borrowed);
            }
            Ok(())
        },
    )
}
