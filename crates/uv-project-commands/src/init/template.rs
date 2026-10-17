//! Render project manifests and source templates without filesystem access.

use std::iter;
use std::path::PathBuf;
use std::str::FromStr;

use toml_edit::{InlineTable, Value};

use uv_configuration::ProjectBuildBackend;
use uv_distribution_types::RequiresPython;
use uv_normalize::PackageName;
use uv_pep440::Version;

/// A generated file relative to the initialized project.
pub(super) struct TemplateFile {
    pub(super) path: PathBuf,
    pub(super) contents: String,
}

impl TemplateFile {
    fn new(path: PathBuf, contents: impl Into<String>) -> Self {
        Self {
            path,
            contents: contents.into(),
        }
    }
}

/// Source files and the package directory created before they are written.
pub(super) struct PackageTemplate {
    pub(super) package_dir: PathBuf,
    pub(super) files: Vec<TemplateFile>,
}

#[derive(Debug)]
pub(super) enum Author {
    Name(String),
    Email(String),
    NameEmail { name: String, email: String },
}

impl Author {
    fn to_toml_string(&self) -> String {
        let mut inline = InlineTable::new();

        match self {
            Self::NameEmail { name, email } => {
                inline.insert("name", Value::from(name));
                inline.insert("email", Value::from(email));
            }
            Self::Name(name) => {
                inline.insert("name", Value::from(name));
            }
            Self::Email(email) => {
                inline.insert("email", Value::from(email));
            }
        }

        inline.to_string()
    }
}

/// Generate the `[project]` section of a `pyproject.toml`.
pub(super) fn pyproject_project(
    name: &PackageName,
    requires_python: &RequiresPython,
    author: Option<&Author>,
    description: Option<&str>,
    no_description: bool,
    no_readme: bool,
) -> String {
    indoc::formatdoc! {r#"
        [project]
        name = "{name}"
        version = "0.1.0"{description}{readme}{authors}
        requires-python = "{requires_python}"
        dependencies = []
    "#,
        readme = if no_readme { "" } else { "\nreadme = \"README.md\"" },
        description = if no_description {
            String::new()
        } else {
            format!("\ndescription = \"{description}\"", description = description.unwrap_or("Add your description here"))
        },
        authors = author.map_or_else(String::new, |author| format!("\nauthors = [\n    {}\n]", author.to_toml_string())),
        requires_python = requires_python.specifiers(),
    }
}

/// Generate the `[build-system]` section of a `pyproject.toml`.
/// Generate the `[tool.]` section of a `pyproject.toml` where applicable.
pub(super) fn pyproject_build_system(
    package: &PackageName,
    build_backend: ProjectBuildBackend,
) -> String {
    let module_name = package.as_dist_info_name();
    match build_backend {
        ProjectBuildBackend::Uv => {
            // Limit to the stable version range.
            let min_version = Version::from_str(uv_version::version()).unwrap();
            debug_assert!(
                min_version.release()[0] == 0,
                "migrate to major version bumps"
            );
            let max_version = Version::new(
                [0, min_version.release()[1] + 1]
                    .into_iter()
                    // Add trailing zeroes to match the version length, to use the same style
                    // as `--bounds`.
                    .chain(iter::repeat_n(0, min_version.release().len() - 2)),
            );
            indoc::formatdoc! {r#"
                [build-system]
                requires = ["uv_build>={min_version},<{max_version}"]
                build-backend = "uv_build"
            "#}
        },
        // Pure-python backends
        ProjectBuildBackend::Hatch => indoc::indoc! {r#"
                [build-system]
                requires = ["hatchling"]
                build-backend = "hatchling.build"
            "#}
        .to_string(),
        ProjectBuildBackend::Flit => indoc::indoc! {r#"
                [build-system]
                requires = ["flit_core>=3.2,<4"]
                build-backend = "flit_core.buildapi"
            "#}
        .to_string(),
        ProjectBuildBackend::PDM => indoc::indoc! {r#"
                [build-system]
                requires = ["pdm-backend"]
                build-backend = "pdm.backend"
            "#}
        .to_string(),
        ProjectBuildBackend::Setuptools => indoc::indoc! {r#"
                [build-system]
                requires = ["setuptools>=61"]
                build-backend = "setuptools.build_meta"
            "#}
        .to_string(),
        ProjectBuildBackend::Poetry => indoc::indoc! {r#"
                [build-system]
                requires = ["poetry-core>=2,<3"]
                build-backend = "poetry.core.masonry.api"
            "#}
        .to_string(),
        // Binary build backends
        ProjectBuildBackend::Maturin => indoc::formatdoc! {r#"
                [tool.maturin]
                module-name = "{module_name}._core"
                python-packages = ["{module_name}"]
                python-source = "src"

                [tool.uv]
                cache-keys = [{{ file = "pyproject.toml" }}, {{ file = "src/**/*.rs" }}, {{ file = "Cargo.toml" }}, {{ file = "Cargo.lock" }}]

                [build-system]
                requires = ["maturin>=1.0,<2.0"]
                build-backend = "maturin"
            "#},
        ProjectBuildBackend::Scikit => indoc::indoc! {r#"
                [tool.scikit-build]
                minimum-version = "build-system.requires"
                build-dir = "build/{wheel_tag}"

                [tool.uv]
                cache-keys = [{ file = "pyproject.toml" }, { file = "src/**/*.{h,c,hpp,cpp}" }, { file = "CMakeLists.txt" }]

                [build-system]
                requires = ["scikit-build-core>=0.12", "pybind11>=3"]
                build-backend = "scikit_build_core.build"
            "#}
        .to_string(),
    }
}

/// Generate the `[project.scripts]` section of a `pyproject.toml`.
pub(super) fn pyproject_project_scripts(
    package: &PackageName,
    executable_name: &str,
    target: &str,
) -> String {
    let module_name = package.as_dist_info_name();
    indoc::formatdoc! {r#"
        [project.scripts]
        {executable_name} = "{module_name}:{target}"
    "#}
}

/// Generate additional files as needed for specific build backends.
pub(super) fn build_backend_prerequisites(
    package: &PackageName,
    build_backend: ProjectBuildBackend,
) -> Vec<TemplateFile> {
    let mut files = Vec::new();
    let module_name = package.as_dist_info_name();
    match build_backend {
        ProjectBuildBackend::Maturin => {
            // Generate Cargo.toml
            let build_file = PathBuf::from("Cargo.toml");
            files.push(TemplateFile::new(
                build_file,
                indoc::formatdoc! {r#"
                [package]
                name = "{module_name}"
                version = "0.1.0"
                edition = "2024"

                [lib]
                name = "_core"
                # "cdylib" is necessary to produce a shared library for Python to import from.
                crate-type = ["cdylib"]

                [dependencies]
                # "extension-module" tells pyo3 we want to build an extension module (skips linking against libpython.so)
                # "abi3-py39" tells pyo3 (and maturin) to build using the stable ABI with minimum Python version 3.9
                pyo3 = {{ version = "0.28.2", features = ["extension-module", "abi3-py39"] }}
            "#},
            ));
        }
        ProjectBuildBackend::Scikit => {
            // Generate CMakeLists.txt
            let build_file = PathBuf::from("CMakeLists.txt");
            files.push(TemplateFile::new(
                build_file,
                indoc::formatdoc! {r"
                cmake_minimum_required(VERSION 3.15...4.0)
                project(${{SKBUILD_PROJECT_NAME}} LANGUAGES CXX)

                find_package(pybind11 CONFIG REQUIRED)

                pybind11_add_module(_core MODULE src/main.cpp)
                install(TARGETS _core DESTINATION ${{SKBUILD_PROJECT_NAME}})
            "},
            ));
        }
        _ => {}
    }
    files
}

/// Generate startup scripts for a package-based application or library.
pub(super) fn package_scripts(
    package: &PackageName,
    build_backend: ProjectBuildBackend,
    is_lib: bool,
) -> PackageTemplate {
    let mut files = Vec::new();
    let module_name = package.as_dist_info_name();

    let src_dir = PathBuf::from("src");
    let pkg_dir = src_dir.join(&*module_name);

    let pure_python_script = if is_lib {
        indoc::formatdoc! {r#"
        def hello() -> str:
            return "Hello from {package}!"
        "#}
    } else {
        indoc::formatdoc! {r#"
        def main() -> None:
            print("Hello from {package}!")
        "#}
    };

    // Python script for binary-based packaged apps or libs
    let binary_call_script = if is_lib {
        indoc::formatdoc! {r"
        from {module_name}._core import hello_from_bin


        def hello() -> str:
            return hello_from_bin()
        "}
    } else {
        indoc::formatdoc! {r"
        from {module_name}._core import hello_from_bin


        def main() -> None:
            print(hello_from_bin())
        "}
    };

    // .pyi file for binary script
    let pyi_contents = indoc::indoc! {r"
        def hello_from_bin() -> str: ...
    "};

    let package_script = match build_backend {
        ProjectBuildBackend::Maturin => {
            // Generate lib.rs
            let native_src = src_dir.join("lib.rs");
            files.push(TemplateFile::new(
                native_src,
                indoc::formatdoc! {r#"
                use pyo3::prelude::*;

                /// A Python module implemented in Rust. The name of this module must match
                /// the `lib.name` setting in the `Cargo.toml`, else Python will not be able to
                /// import the module.
                #[pymodule]
                mod _core {{
                    use pyo3::prelude::*;

                    #[pyfunction]
                    fn hello_from_bin() -> String {{
                        "Hello from {package}!".to_string()
                    }}
                }}
            "#},
            ));
            // Generate .pyi file
            let pyi_file = pkg_dir.join("_core.pyi");
            files.push(TemplateFile::new(pyi_file, pyi_contents));
            // Return python script calling binary
            binary_call_script
        }
        ProjectBuildBackend::Scikit => {
            // Generate main.cpp
            let native_src = src_dir.join("main.cpp");
            files.push(TemplateFile::new(
                native_src,
                indoc::formatdoc! {r#"
                #include <pybind11/pybind11.h>

                std::string hello_from_bin() {{ return "Hello from {package}!"; }}

                namespace py = pybind11;

                PYBIND11_MODULE(_core, m) {{
                  m.doc() = "pybind11 hello module";

                  m.def("hello_from_bin", &hello_from_bin, R"pbdoc(
                      A function that returns a Hello string.
                  )pbdoc");
                }}
            "#},
            ));
            // Generate .pyi file
            let pyi_file = pkg_dir.join("_core.pyi");
            files.push(TemplateFile::new(pyi_file, pyi_contents));
            // Return python script calling binary
            binary_call_script
        }
        _ => pure_python_script,
    };

    // Generate `src/{name}/__init__.py`.
    let init_py = pkg_dir.join("__init__.py");
    files.push(TemplateFile::new(init_py, package_script));

    // Generate `src/{name}/py.typed` for libraries.
    if is_lib {
        let py_typed = pkg_dir.join("py.typed");
        files.push(TemplateFile::new(py_typed, ""));
    }

    PackageTemplate {
        package_dir: pkg_dir,
        files,
    }
}

/// Render the entrypoint for an unpackaged application.
pub(super) fn application_script(name: &PackageName) -> String {
    indoc::formatdoc! {r#"
                    def main():
                        print("Hello from {name}!")


                    if __name__ == "__main__":
                        main()
                "#}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn author_to_toml_string_handles_inline_quotes() {
        let author = Author::NameEmail {
            name: "Tony \"Iron Man\" Stark".to_string(),
            email: "ironman@example.com".to_string(),
        };

        assert_eq!(
            author.to_toml_string(),
            "{ name = 'Tony \"Iron Man\" Stark', email = \"ironman@example.com\" }"
        );
    }
}
