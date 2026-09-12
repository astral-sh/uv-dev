//! Deterministic Python modules and entry-point metadata for scenario packages.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use anyhow::{Result, ensure};

use uv_normalize::PackageName;
use uv_pep440::Version;

use super::scenario::{ScriptTarget, validate_script_name};

/// Generated content shared by a package's wheels and source distribution.
#[derive(Default)]
pub(super) struct GeneratedScripts {
    files: BTreeMap<String, String>,
    entry_points: String,
    pyproject: String,
}

impl GeneratedScripts {
    pub(super) fn new(
        name: &PackageName,
        version: &Version,
        scripts: &BTreeMap<String, ScriptTarget>,
    ) -> Result<Self> {
        if scripts.is_empty() {
            return Ok(Self::default());
        }

        let normalized = name.as_dist_info_name();
        let mut modules: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
        let mut entry_points = String::from("[console_scripts]\n");
        let mut script_values = BTreeMap::new();
        for (script, target) in scripts {
            validate_script_name(script).map_err(anyhow::Error::msg)?;
            ensure!(
                target.module().split('.').next() == Some(normalized.as_ref()),
                "console-script `{script}` for package `{name}` must target `{normalized}` or one of its submodules, got `{target}`"
            );

            let mut module = String::new();
            for component in target.module().split('.') {
                if !module.is_empty() {
                    module.push('.');
                }
                module.push_str(component);
                modules.entry(module.clone()).or_default();
            }
            modules.entry(module).or_default().insert(target.function());
            writeln!(entry_points, "{script} = {target}")?;
            script_values.insert(script, target.to_string());
        }

        let files = modules
            .into_iter()
            .map(|(module, functions)| {
                let mut contents = format!("__version__ = \"{version}\"\n");
                for function in functions {
                    writeln!(
                        contents,
                        "\ndef {function}():\n    print(\"{name} {version}\")"
                    )
                    .expect("writing generated Python into a string should succeed");
                }
                (
                    format!("{}/__init__.py", module.replace('.', "/")),
                    contents,
                )
            })
            .collect();

        Ok(Self {
            files,
            entry_points,
            pyproject: format!("\n[project.scripts]\n{}", toml::to_string(&script_values)?),
        })
    }

    pub(super) fn source(&self, path: &str) -> Option<&str> {
        self.files.get(path).map(String::as_str)
    }

    pub(super) fn files(&self) -> impl Iterator<Item = (&str, &str)> {
        self.files
            .iter()
            .map(|(path, contents)| (path.as_str(), contents.as_str()))
    }

    pub(super) fn entry_points(&self) -> Option<&str> {
        (!self.entry_points.is_empty()).then_some(self.entry_points.as_str())
    }

    pub(super) fn pyproject(&self) -> &str {
        &self.pyproject
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    #[test]
    fn nested_modules_share_callables() -> Result<()> {
        let name = PackageName::from_str("my-package")?;
        let version = Version::from_str("1.2.3")?;
        let scripts = BTreeMap::from([
            (
                "alias".to_string(),
                ScriptTarget::from_str("my_package.commands.cli:main")
                    .expect("valid script target"),
            ),
            (
                "example".to_string(),
                ScriptTarget::from_str("my_package.commands.cli:main")
                    .expect("valid script target"),
            ),
            (
                "root".to_string(),
                ScriptTarget::from_str("my_package:run").expect("valid script target"),
            ),
        ]);
        let generated = GeneratedScripts::new(&name, &version, &scripts)?;

        assert_eq!(
            generated.files().map(|(path, _)| path).collect::<Vec<_>>(),
            [
                "my_package/__init__.py",
                "my_package/commands/__init__.py",
                "my_package/commands/cli/__init__.py",
            ]
        );
        assert_eq!(
            generated.source("my_package/commands/cli/__init__.py"),
            Some("__version__ = \"1.2.3\"\n\ndef main():\n    print(\"my-package 1.2.3\")\n")
        );
        assert_eq!(
            generated.entry_points(),
            Some(
                "[console_scripts]\nalias = my_package.commands.cli:main\nexample = my_package.commands.cli:main\nroot = my_package:run\n"
            )
        );
        let pyproject: toml::Value = toml::from_str(generated.pyproject())?;
        assert_eq!(
            pyproject["project"]["scripts"]["alias"].as_str(),
            Some("my_package.commands.cli:main")
        );
        Ok(())
    }

    #[test]
    fn reject_modules_outside_the_package() -> Result<()> {
        let scripts = BTreeMap::from([(
            "example".to_string(),
            ScriptTarget::from_str("other:main").expect("valid script target"),
        )]);
        let error = GeneratedScripts::new(
            &PackageName::from_str("my-package")?,
            &Version::from_str("1.2.3")?,
            &scripts,
        )
        .err()
        .expect("a module outside the package should fail");
        assert_eq!(
            error.to_string(),
            "console-script `example` for package `my-package` must target `my_package` or one of its submodules, got `other:main`"
        );
        Ok(())
    }

    #[test]
    fn reject_programmatic_invalid_script_names() -> Result<()> {
        let scripts = BTreeMap::from([(
            "../example".to_string(),
            ScriptTarget::from_str("my_package:main").expect("valid script target"),
        )]);
        let error = GeneratedScripts::new(
            &PackageName::from_str("my-package")?,
            &Version::from_str("1.2.3")?,
            &scripts,
        )
        .err()
        .expect("an invalid script name should fail");
        assert_eq!(
            error.to_string(),
            "invalid console-script name `../example`"
        );
        Ok(())
    }

    #[test]
    fn empty_scripts_add_no_files() -> Result<()> {
        let generated = GeneratedScripts::new(
            &PackageName::from_str("my-package")?,
            &Version::from_str("1.2.3")?,
            &BTreeMap::new(),
        )?;
        assert_eq!(generated.files().count(), 0);
        assert_eq!(generated.entry_points(), None);
        assert_eq!(generated.pyproject(), "");
        Ok(())
    }
}
