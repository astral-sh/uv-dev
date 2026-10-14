use std::path::PathBuf;
use std::str::FromStr;
use std::{fmt::Display, fmt::Write};

use anstream::{ColorChoice, stream::IsTerminal};
use anyhow::{Result, anyhow};
use clap::CommandFactory;
use itertools::Itertools;
use owo_colors::OwoColorize;
use which::which;

use uv_cli::Cli;
use uv_command_support::{ExitStatus, Printer};
use uv_static::EnvVars;

// hidden subcommands to show in the help command
const SHOW_HIDDEN_COMMANDS: &[&str] = &["generate-shell-completion"];

pub(crate) fn help(query: &[String], printer: Printer, no_pager: bool) -> Result<ExitStatus> {
    let mut uv: clap::Command = SHOW_HIDDEN_COMMANDS
        .iter()
        .fold(Cli::command(), |uv, &name| {
            uv.mut_subcommand(name, |cmd| cmd.hide(false))
        });

    // It is very important to build the command before beginning inspection or subcommands
    // will be missing all of the propagated options.
    uv.build();

    let command = find_command(query, &uv).map_err(|(unmatched, nearest)| {
        let missing = if unmatched.len() == query.len() {
            format!("`{}` for `uv`", query.join(" "))
        } else {
            format!("`{}` for `uv {}`", unmatched.join(" "), nearest.get_name())
        };
        anyhow!(
            "There is no command {}. Did you mean one of:\n    {}",
            missing,
            nearest
                .get_subcommands()
                .filter(|cmd| !cmd.is_hide_set())
                .map(clap::Command::get_name)
                .filter(|name| *name != "help")
                .join("\n    "),
        )
    })?;

    let name = command.get_name();
    let is_root = name == uv.get_name();
    let mut command = command.clone();

    let help = if is_root {
        command
            .after_help(format!(
                "Use `{}` for more information on a specific command.",
                "uv help <command>".bold()
            ))
            .render_help()
    } else {
        if command.has_subcommands() {
            command.after_long_help(format!(
                "Use `{}` for more information on a specific command.",
                format!("uv help {name} <command>").bold()
            ))
        } else {
            command
        }
        .render_long_help()
    };

    let want_color = match anstream::Stdout::choice(&std::io::stdout()) {
        ColorChoice::Always | ColorChoice::AlwaysAnsi => true,
        ColorChoice::Never => false,
        // We just asked anstream for a choice, that can't be auto
        ColorChoice::Auto => unreachable!(),
    };

    let pager = if !no_pager && !is_root && std::io::stdout().is_terminal() {
        Pager::try_from_env()
    } else {
        None
    };
    let use_color = want_color && pager.as_ref().is_none_or(Pager::supports_colors);

    let help = if use_color {
        help.ansi().to_string()
    } else {
        help.to_string()
    };
    // Reformat inline [env: VAR=] annotations to their own line.
    let help = if is_root {
        help
    } else {
        reformat_env_annotations(&help)
    };
    // Trim trailing whitespace from each line (https://github.com/clap-rs/clap/issues/5074).
    let help = help.lines().map(str::trim_end).join("\n");

    if let Some(pager) = pager {
        let query = query.join(" ");
        let heading = if use_color {
            format!("{}: {query}", "uv help".bold())
        } else {
            format!("uv help: {query}")
        };
        pager.spawn(heading, help)?;
    } else {
        writeln!(printer.stdout(), "{help}")?;
    }

    Ok(ExitStatus::Success)
}

/// Get the first non-ANSI character.
///
/// Returns `None` if the string is empty or only contains ANSI sequences.
fn first_non_ansi_char(s: &str) -> Option<char> {
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip ANSI escape sequences.
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            return Some(c);
        }
    }
    None
}

/// Reformat `[env: VAR=]` annotations in long help output.
///
/// Moves inline `[env: VAR=]` annotations to their own line at the end of each
/// argument's description, matching clap's native formatting for environment vars.
fn reformat_env_annotations(help: &str) -> String {
    let mut result = String::with_capacity(help.len());
    let mut pending_env = None;
    for line in help.lines() {
        // Classify the line type based on clap's help formatting:
        // - Argument lines: 6 spaces + `-` or `<` (e.g., "      --offline", "      <PACKAGE>")
        // - Description lines: 10 spaces + text (e.g., "          Disable network access")
        // - Section headers: no leading spaces, ends with `:` (e.g., "Options:")
        //
        // Leading spaces never contain ANSI codes, but argument names may be colored,
        // so we skip ANSI sequences when checking the first content character.
        let content = line.trim_start();
        let indent = line.len() - content.len();
        let first_char = first_non_ansi_char(content);
        let is_arg_line = indent == 6 && matches!(first_char, Some('-' | '<'));
        let is_section_header = indent == 0 && line.ends_with(':');
        let is_description_line = indent == 10;

        // Flush pending env before starting a new argument or section.
        if (is_arg_line || is_section_header)
            && let Some(env) = pending_env.take()
        {
            append_env_annotation(&mut result, env);
            result.push('\n');
        }

        // Check for inline environment annotations on description lines.
        if is_description_line
            && let Some((env_annotation, new_line)) = extract_env_annotation(line)
        {
            pending_env = Some(env_annotation);
            result.push_str(&new_line);
            // Add a period, if the line doesn't end with punctuation.
            if !new_line.ends_with('.') && !new_line.ends_with(':') {
                result.push('.');
            }
            result.push('\n');
            continue;
        }

        result.push_str(line);
        result.push('\n');
    }

    // Flush any remaining pending environment variables at the end of the help.
    if let Some(env) = pending_env {
        append_env_annotation(&mut result, env);
    }

    if result.ends_with('\n') {
        result.pop();
    }

    result
}

/// Append an environment annotation after exactly one blank line.
fn append_env_annotation(help: &mut String, annotation: &str) {
    while help.ends_with("\n\n") {
        help.pop();
    }
    if !help.ends_with('\n') {
        help.push('\n');
    }
    help.push_str("\n          ");
    help.push_str(annotation);
    help.push('\n');
}

/// Extract an inline `[env: VAR=]` annotation from a line.
///
/// Returns the annotation and the line with the annotation removed, or `None` if no
/// annotation is found.
fn extract_env_annotation(line: &str) -> Option<(&str, String)> {
    // Look for the pattern: " [env: SOMETHING=]"
    let start = line.find(" [env: ")?;
    let rest = &line[start + " [env: ".len()..];
    let end_offset = rest.find("=]")?;

    // Validate that the environment variable name contains only uppercase letters and underscores.
    let env_name = &rest[..end_offset];
    if !env_name.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
        return None;
    }

    let annotation_end = start + " [env: ".len() + end_offset + "=]".len();
    let annotation = &line[start + " ".len()..annotation_end];
    let prefix = &line[..start];
    let suffix = &line[annotation_end..];

    // Only extract if there's actual text remaining (not just whitespace).
    // If the line is just the annotation (clap-generated), leave it alone.
    if prefix.trim().is_empty() && suffix.trim().is_empty() {
        return None;
    }

    Some((annotation, format!("{prefix}{suffix}")))
}

/// Find the command corresponding to a set of arguments, e.g., `["uv", "pip", "install"]`.
///
/// If the command cannot be found, the nearest command is returned.
fn find_command<'a>(
    query: &'a [String],
    cmd: &'a clap::Command,
) -> Result<&'a clap::Command, (&'a [String], &'a clap::Command)> {
    let Some((next, remaining)) = query.split_first() else {
        return Ok(cmd);
    };

    let subcommand = cmd.find_subcommand(next).ok_or((query, cmd))?;
    find_command(remaining, subcommand)
}

#[derive(Debug)]
enum PagerKind {
    Less,
    More,
    Other(String),
}

#[derive(Debug)]
struct Pager {
    kind: PagerKind,
    args: Vec<String>,
    path: Option<PathBuf>,
}

impl PagerKind {
    fn default_args(&self) -> Vec<String> {
        match self {
            Self::Less => vec!["-R".to_string()],
            Self::More | Self::Other(_) => vec![],
        }
    }
}

impl Display for PagerKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Less => write!(f, "less"),
            Self::More => write!(f, "more"),
            Self::Other(name) => write!(f, "{name}"),
        }
    }
}

impl FromStr for Pager {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut split = s.split_ascii_whitespace();

        // Empty string
        let Some(first) = split.next() else {
            return Err(());
        };

        let kind = match first {
            "less" => PagerKind::Less,
            "more" => PagerKind::More,
            _ => PagerKind::Other(first.to_string()),
        };
        Ok(Self {
            kind,
            args: split.map(str::to_string).collect(),
            path: None,
        })
    }
}

impl Pager {
    /// Display `contents` using the pager.
    fn spawn(self, heading: String, contents: String) -> Result<()> {
        use std::io::Write;

        let command = self
            .path
            .map(PathBuf::into_os_string)
            .unwrap_or_else(|| self.kind.to_string().into());

        let args = if self.args.is_empty() {
            self.kind.default_args()
        } else {
            self.args
        };

        let mut child = std::process::Command::new(command)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .spawn()?;

        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("Failed to take child process stdin"))?;

        let writer = std::thread::spawn(move || {
            let _ = write!(stdin, "{heading}\n\n");
            let _ = stdin.write_all(contents.as_bytes());
        });

        drop(child.wait());
        drop(writer.join());

        Ok(())
    }

    /// Get a pager to use and its path, if available.
    ///
    /// Supports the `PAGER` environment variable, otherwise checks for `less` and `more` in the
    /// search path.
    fn try_from_env() -> Option<Self> {
        if let Some(pager) = std::env::var_os(EnvVars::PAGER)
            && !pager.is_empty()
        {
            return Self::from_str(&pager.to_string_lossy()).ok();
        }

        if let Ok(less) = which("less") {
            Some(Self {
                kind: PagerKind::Less,
                args: vec![],
                path: Some(less),
            })
        } else if let Ok(more) = which("more") {
            Some(Self {
                kind: PagerKind::More,
                args: vec![],
                path: Some(more),
            })
        } else {
            None
        }
    }

    fn supports_colors(&self) -> bool {
        match self.kind {
            // The `-R` flag is required for color support. We will provide it by default.
            PagerKind::Less => self.args.is_empty() || self.args.iter().any(|arg| arg == "-R"),
            PagerKind::More => false,
            PagerKind::Other(_) => false,
        }
    }
}
