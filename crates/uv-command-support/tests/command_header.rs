use std::ffi::OsString;

use uv_command_support::command_header::{HeaderArgument, format_command_header};

#[test]
fn omit_transient_flags_and_upgrade_values() {
    let args = [
        "uv",
        "pip",
        "compile",
        "requirements.in",
        "--upgrade",
        "-U",
        "-P",
        "foo",
        "-Pbar",
        "--upgrade-package=baz",
        "--upgrade-package",
        "qux",
        "--quiet",
        "-q",
        "--verbose",
        "-v",
        "--no-progress",
        "--native-tls",
    ];
    assert_eq!(
        format_command_header(args.map(OsString::from), |_| HeaderArgument::Keep),
        "uv pip compile requirements.in --no-progress --native-tls"
    );
}

#[test]
fn caller_options_consume_exactly_one_value() {
    let args = [
        "uv",
        "export",
        "--custom",
        "--quiet",
        "keep",
        "--custom=attached",
        "tail",
    ];
    let command = format_command_header(args.map(OsString::from), |arg| {
        if arg == "--custom" {
            HeaderArgument::OmitWithValue
        } else if arg.starts_with("--custom=") {
            HeaderArgument::Omit
        } else {
            HeaderArgument::Keep
        }
    });
    assert_eq!(command, "uv export keep tail");
}

#[test]
fn retain_spacing_and_empty_arguments() {
    assert_eq!(
        format_command_header(
            ["uv", "export", "two words", ""].map(OsString::from),
            |_| HeaderArgument::Keep
        ),
        "uv export two words "
    );
    assert_eq!(
        format_command_header(["uv", "-P"].map(OsString::from), |_| HeaderArgument::Keep),
        "uv "
    );
}
