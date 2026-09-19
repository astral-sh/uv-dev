use anyhow::Result;
use assert_fs::prelude::*;
use indoc::indoc;

use uv_test::uv_snapshot;

/// Directory execution and `-m` must retain Python's distinct import semantics.
#[test]
fn run_directory_and_module() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let package = context.temp_dir.child("mode_case");
    package.child("__init__.py").write_str(indoc! { r#"
        print("package initialized")
    "# })?;
    package.child("sibling.py").write_str(indoc! { r#"
        VALUE = "sibling imported"
    "# })?;
    package.child("__main__.py").write_str(indoc! { r#"
        import sys

        print(f"name: {__name__!r}")
        print(f"package: {__package__!r}")
        print(f"spec: {__spec__.name!r}")
        print(f"arguments: {sys.argv[1:]!r}")
        try:
            from .sibling import VALUE
        except ImportError as error:
            print(f"relative import: {type(error).__name__}")
        else:
            print(f"relative import: {VALUE}")
    "# })?;

    let options = [
        "--no-project",
        "--no-index",
        "--offline",
        "--no-config",
        "--no-build",
    ];

    // A directory runs its `__main__.py` without importing the enclosing package.
    let directory = uv_snapshot!(context.filters(), context.run()
        .args(options)
        .arg("--python").arg(context.interpreter())
        .args(["./mode_case", "argument"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    name: '__main__'
    package: ''
    spec: '__main__'
    arguments: ['argument']
    relative import: ImportError
    ");
    let python_directory = context
        .python_command()
        .args(["./mode_case", "argument"])
        .output()?;
    assert_eq!(directory.status, python_directory.status);
    assert_eq!(directory.stdout, python_directory.stdout);
    assert_eq!(directory.stderr, python_directory.stderr);

    // Explicit module execution imports the package and supports relative imports.
    let module = uv_snapshot!(context.filters(), context.run()
        .args(options)
        .arg("--python").arg(context.interpreter())
        .args(["-m", "mode_case", "argument"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    package initialized
    name: '__main__'
    package: 'mode_case'
    spec: 'mode_case.__main__'
    arguments: ['argument']
    relative import: sibling imported
    ");
    let python_module = context
        .python_command()
        .args(["-m", "mode_case", "argument"])
        .output()?;
    assert_eq!(module.status, python_module.status);
    assert_eq!(module.stdout, python_module.stdout);
    assert_eq!(module.stderr, python_module.stderr);

    Ok(())
}
