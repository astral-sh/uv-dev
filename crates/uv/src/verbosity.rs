use std::ffi::OsStr;

use clap::error::ErrorKind;
use clap::{CommandFactory, Error};
use uv_cli::{Cli, GlobalArgs};
use uv_static::{EnvVars, InvalidEnvironmentVariable};

/// The resolved output counts for one uv invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Verbosity {
    pub(crate) quiet: u8,
    pub(crate) verbose: u8,
}

impl Verbosity {
    /// Resolve the command-line counts before reading either environment variable.
    pub(crate) fn from_args(args: &GlobalArgs) -> Result<Self, Error> {
        if let Some(verbosity) = Self::from_cli(args.quiet, args.verbose) {
            return Ok(verbosity);
        }

        let quiet = std::env::var_os(EnvVars::UV_QUIET);
        let verbose = std::env::var_os(EnvVars::UV_VERBOSE);
        Self::from_environment(quiet.as_deref(), verbose.as_deref())
    }

    fn from_cli(quiet: u8, verbose: u8) -> Option<Self> {
        (quiet != 0 || verbose != 0).then_some(Self { quiet, verbose })
    }

    fn from_environment(quiet: Option<&OsStr>, verbose: Option<&OsStr>) -> Result<Self, Error> {
        let quiet = parse_count(EnvVars::UV_QUIET, quiet)?;
        let verbose = parse_count(EnvVars::UV_VERBOSE, verbose)?;
        if quiet != 0 && verbose != 0 {
            return Err(Cli::command().error(
                ErrorKind::ArgumentConflict,
                "the environment variables `UV_QUIET` and `UV_VERBOSE` cannot both be nonzero",
            ));
        }
        Ok(Self { quiet, verbose })
    }
}

fn parse_count(name: &str, value: Option<&OsStr>) -> Result<u8, Error> {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        return Ok(0);
    };
    let Some(value) = value.to_str() else {
        return Err(Cli::command().error(
            ErrorKind::ValueValidation,
            InvalidEnvironmentVariable {
                name: name.to_owned(),
                value: value.to_string_lossy().into_owned(),
                err: "expected a valid UTF-8 string".to_owned(),
            },
        ));
    };
    value.parse::<u8>().map_err(|error| {
        Cli::command().error(
            ErrorKind::ValueValidation,
            InvalidEnvironmentVariable {
                name: name.to_owned(),
                value: value.to_owned(),
                err: format!("{error}; expected an integer in 0..=255"),
            },
        )
    })
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    #[cfg(unix)]
    use std::os::unix::ffi::OsStrExt;

    use clap::error::ErrorKind;
    use insta::assert_snapshot;

    use super::Verbosity;

    #[test]
    fn command_line_counts_take_precedence() {
        assert_eq!(Verbosity::from_cli(0, 0), None);
        for (quiet, verbose) in [(1, 0), (2, 0), (255, 0), (0, 1), (0, 3), (0, 255), (1, 1)] {
            assert_eq!(
                Verbosity::from_cli(quiet, verbose),
                Some(Verbosity { quiet, verbose })
            );
        }
    }

    #[test]
    fn environment_counts() -> anyhow::Result<()> {
        for (quiet, verbose, expected) in [
            (None, None, (0, 0)),
            (Some(""), Some(""), (0, 0)),
            (Some("0"), Some("0"), (0, 0)),
            (Some("1"), None, (1, 0)),
            (Some("2"), Some("0"), (2, 0)),
            (Some("255"), None, (255, 0)),
            (None, Some("1"), (0, 1)),
            (Some("0"), Some("3"), (0, 3)),
            (None, Some("255"), (0, 255)),
            (None, Some("+3"), (0, 3)),
            (None, Some("003"), (0, 3)),
        ] {
            let verbosity =
                Verbosity::from_environment(quiet.map(OsStr::new), verbose.map(OsStr::new))?;
            assert_eq!((verbosity.quiet, verbosity.verbose), expected);
        }
        Ok(())
    }

    #[test]
    fn invalid_environment_counts() {
        for value in ["-1", "256", "1.5", "true", " 3 "] {
            for (quiet, verbose) in [(Some(value), None), (None, Some(value))] {
                let error =
                    Verbosity::from_environment(quiet.map(OsStr::new), verbose.map(OsStr::new))
                        .expect_err("invalid count should be rejected");
                assert_eq!(error.kind(), ErrorKind::ValueValidation);
                assert_eq!(error.exit_code(), 2);
            }
        }
    }

    #[test]
    fn environment_count_errors() {
        let invalid = Verbosity::from_environment(Some(OsStr::new("bad")), Some(OsStr::new("bad")))
            .expect_err("quiet is validated first");
        assert_snapshot!(invalid, @"
        error: Failed to parse environment variable `UV_QUIET` with invalid value `bad`: invalid digit found in string; expected an integer in 0..=255

        Usage: uv [OPTIONS] <COMMAND>

        For more information, try '--help'.
        ");

        let conflict = Verbosity::from_environment(Some(OsStr::new("1")), Some(OsStr::new("3")))
            .expect_err("nonzero counts should conflict");
        assert_eq!(conflict.kind(), ErrorKind::ArgumentConflict);
        assert_eq!(conflict.exit_code(), 2);
        assert_snapshot!(conflict, @"
        error: the environment variables `UV_QUIET` and `UV_VERBOSE` cannot both be nonzero

        Usage: uv [OPTIONS] <COMMAND>

        For more information, try '--help'.
        ");
    }

    #[cfg(unix)]
    #[test]
    fn non_unicode_environment_count() {
        let error = Verbosity::from_environment(None, Some(OsStr::from_bytes(b"\xff")))
            .expect_err("non-Unicode count should be rejected");
        assert_eq!(error.kind(), ErrorKind::ValueValidation);
        assert_eq!(error.exit_code(), 2);
        assert_snapshot!(error, @"
        error: Failed to parse environment variable `UV_VERBOSE` with invalid value `�`: expected a valid UTF-8 string

        Usage: uv [OPTIONS] <COMMAND>

        For more information, try '--help'.
        ");
    }
}
