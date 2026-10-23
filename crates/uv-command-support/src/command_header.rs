use std::ffi::OsString;

/// Whether an argument belongs in a generated-file command header.
pub enum HeaderArgument {
    Keep,
    Omit,
    OmitWithValue,
}

/// Format the generating command, omitting upgrades, verbosity, and caller-specific options.
///
/// The first argument is the executable name. Values following omitted options are consumed even
/// when they resemble another option. Retained arguments use the CLI's lossy, unquoted spelling.
pub fn format_command_header(
    args: impl IntoIterator<Item = OsString>,
    mut filter: impl FnMut(&str) -> HeaderArgument,
) -> String {
    let mut retained = Vec::new();
    let mut skip_next = false;
    for arg in args.into_iter().skip(1) {
        if skip_next {
            skip_next = false;
            continue;
        }
        let arg = arg.to_string_lossy();
        let action =
            if ["--upgrade", "-U", "--quiet", "-q", "--verbose", "-v"].contains(&arg.as_ref()) {
                HeaderArgument::Omit
            } else if arg == "--upgrade-package" || arg == "-P" {
                HeaderArgument::OmitWithValue
            } else if arg.starts_with("--upgrade-package=") || arg.starts_with("-P") {
                HeaderArgument::Omit
            } else {
                filter(&arg)
            };
        match action {
            HeaderArgument::Keep => retained.push(arg.into_owned()),
            HeaderArgument::Omit => {}
            HeaderArgument::OmitWithValue => skip_next = true,
        }
    }
    format!("uv {}", retained.join(" "))
}
