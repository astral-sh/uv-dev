use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use uv_shell::Shell;
use uv_static::EnvVars;

use crate::printer::Printer;

/// Quote an installation path while retaining the standalone installer's relocatable HOME form.
fn quoted_path(path: &Path, home: &Path) -> Result<String> {
    let (prefix, path) = match path.strip_prefix(home) {
        Ok(relative) => ("$HOME/", relative),
        Err(_) => ("", path),
    };
    let path = path
        .to_str()
        .context("Shell configuration requires a UTF-8 installation path")?;
    anyhow::ensure!(
        !path.contains(['\n', '\r']),
        "Shell configuration does not support newlines in installation paths"
    );
    let mut quoted = format!("\"{prefix}");
    for character in path.chars() {
        if matches!(character, '\\' | '"' | '$' | '`') {
            quoted.push('\\');
        }
        quoted.push(character);
    }
    quoted.push('"');
    Ok(quoted)
}

fn create_env(path: &Path, contents: &str) -> Result<()> {
    match fs_err::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut file) => file.write_all(contents.as_bytes())?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn append_source(path: &Path, source: &str, alternate: &str) -> Result<()> {
    let contents = match fs_err::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    if contents.lines().any(|line| {
        let line = line.trim();
        line == source || line == alternate
    }) {
        return Ok(());
    }
    fs_err::create_dir_all(
        path.parent()
            .context("Shell profile has no parent directory")?,
    )?;
    let mut file = fs_err::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)?;
    writeln!(file, "\n{source}")?;
    Ok(())
}

/// Configure the profile set and env files used by the standalone shell installer.
pub(super) fn update_shell(directory: &Path, printer: Printer) -> Result<()> {
    if Shell::contains_path(directory) {
        return Ok(());
    }
    let home = etcetera::home_dir()?;
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".cargo"));
    let env_directory = if directory == cargo_home.join("bin") {
        cargo_home
    } else {
        directory.to_path_buf()
    };
    let env = env_directory.join("env");
    let fish_env = env_directory.join("env.fish");
    let directory_expression = quoted_path(directory, &home)?;
    let env_expression = quoted_path(&env, &home)?;
    let fish_expression = quoted_path(&fish_env, &home)?;
    create_env(
        &env,
        &format!(
            "#!/bin/sh\ncase \":${{PATH-}}:\" in\n    *:{directory_expression}:*) ;;\n    *) export PATH={directory_expression}:\"${{PATH-}}\" ;;\nesac\n"
        ),
    )?;
    create_env(
        &fish_env,
        &format!(
            "if not contains -- {directory_expression} $PATH\n    set -gx PATH {directory_expression} $PATH\nend\n"
        ),
    )?;
    let source = format!(". {env_expression}");
    let alternate = format!("source {env_expression}");
    append_source(&home.join(".profile"), &source, &alternate)?;
    for profile in [".bashrc", ".bash_profile", ".bash_login"] {
        let profile = home.join(profile);
        if profile.is_file() {
            append_source(&profile, &source, &alternate)?;
        }
    }
    let zsh_home = std::env::var_os(EnvVars::ZDOTDIR)
        .filter(|path| !path.is_empty())
        .map_or_else(|| home.clone(), PathBuf::from);
    let zsh_profile = [".zshrc", ".zshenv"]
        .map(|name| zsh_home.join(name))
        .into_iter()
        .find(|path| path.is_file())
        .unwrap_or_else(|| zsh_home.join(".zshrc"));
    append_source(&zsh_profile, &source, &alternate)?;
    append_source(
        &home.join(".config/fish/conf.d/uv.env.fish"),
        &format!("source {fish_expression}"),
        &format!(". {fish_expression}"),
    )?;
    writeln!(
        printer.stderr(),
        "Restart your shell or run `source {env_expression}` to use uv"
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_paths_without_expanding_shell_syntax() {
        assert_eq!(
            quoted_path(Path::new("/home/user/.local/bin"), Path::new("/home/user")).unwrap(),
            "\"$HOME/.local/bin\""
        );
        assert_eq!(
            quoted_path(Path::new("/opt/a$`\"\\b"), Path::new("/home/user")).unwrap(),
            "\"/opt/a\\$\\`\\\"\\\\b\""
        );
        assert!(quoted_path(Path::new("/opt/a\nb"), Path::new("/home/user")).is_err());
    }
}
