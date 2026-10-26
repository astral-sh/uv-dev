use std::path::Path;
use std::process::Command;

use indoc::indoc;
use uv_test::TestContext;

/// Create a stdlib virtual environment that follows a managed minor-version installation.
pub(super) fn create_stdlib_venv(context: &TestContext, minor_version: &str) -> Command {
    // CPython records the interpreter's directory as the environment's home, so create it
    // through the managed minor-version interpreter instead of the user-bin launcher.
    let mut command = context.run();
    command
        .args(["--python", minor_version, "python", "-m", "venv"])
        .arg(context.venv.as_os_str())
        .arg("--without-pip");
    command
}

/// Run the interpreter to verify it initializes as a virtual environment and print its version.
pub(super) fn venv_version(context: &TestContext, interpreter: impl AsRef<Path>) -> Command {
    let mut command = context.external_command(interpreter);
    command.args([
        "-I",
        "-c",
        indoc! {"
            import sys
            if sys.prefix == sys.base_prefix:
                raise RuntimeError('the interpreter is not in a virtual environment')
            print('Python ' + sys.version.split()[0])
        "},
    ]);
    command
}
