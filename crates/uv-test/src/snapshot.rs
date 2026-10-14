//! Command output capture, normalization, and snapshot assertions.

use std::borrow::BorrowMut;
use std::cell::RefCell;
use std::collections::HashMap;
use std::env;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::thread;

use regex::{Regex, regex};

use uv_static::EnvVars;

#[doc(hidden)] // Macro and test context only, don't use directly.
pub const INSTA_FILTERS: &[(&str, &str)] = &[
    (r"--cache-dir [^\s]+", "--cache-dir [CACHE_DIR]"),
    // Operation times
    (r"(\s|\()(\d+m )?(\d+\.)?\d+(ms|s)", "$1[TIME]"),
    // Timestamps
    (r"tv_sec: \d+", "tv_sec: [TIME]"),
    (r"tv_nsec: \d+", "tv_nsec: [TIME]"),
    // Rewrite Windows output to Unix output
    (r"\\([\w\d]|\.)", "/$1"),
    (r"uv\.exe", "uv"),
    // uv version display
    (
        r"uv(-.*)? \d+\.\d+\.\d+(-(alpha|beta|rc)\.\d+)?(\+\d+)?( \([^)]*\))?",
        r"uv [VERSION] ([COMMIT] DATE)",
    ),
    // Trim end-of-line whitespaces, to allow removing them on save.
    (r"([^\s])[ \t]+(\r?\n)", "$1$2"),
    // Certificate overrides and their contents depend on the host environment.
    (
        r"(?ms)^([ \t]*custom_certificates: )(?:None|Some\(\n.*?^[ \t]*\),\n[ \t]*\)),",
        "${1}[CERTIFICATES],",
    ),
];

#[derive(Debug, Copy, Clone)]
pub enum WindowsFilters {
    Platform,
    Universal,
}

thread_local! {
    /// Reuse compiled patterns across snapshots and contexts on the same test thread.
    static SNAPSHOT_FILTER_CACHE: RefCell<HashMap<String, Regex>> = RefCell::new(HashMap::new());
}

/// Helper method to apply filters to a string. Useful when `!uv_snapshot` cannot be used.
pub fn apply_filters<T: AsRef<str>>(mut snapshot: String, filters: impl AsRef<[(T, T)]>) -> String {
    let filters = filters.as_ref();
    // Release the thread-local borrow before converting the caller's filters.
    let mut compiled_filters = SNAPSHOT_FILTER_CACHE.take();
    for (matcher, replacement) in filters {
        let matcher = matcher.as_ref();
        if !compiled_filters.contains_key(matcher) {
            compiled_filters.insert(
                matcher.to_owned(),
                Regex::new(matcher).expect("Do you need to regex::escape your filter?"),
            );
        }
        let re = &compiled_filters[matcher];
        if re.is_match(&snapshot) {
            snapshot = re.replace_all(&snapshot, replacement.as_ref()).to_string();
        }
    }
    SNAPSHOT_FILTER_CACHE.set(compiled_filters);
    snapshot
}

/// Execute the command and format its output status, stdout and stderr into a snapshot string.
///
/// This function is derived from `insta_cmd`s `spawn_with_info`.
#[expect(clippy::print_stderr)]
pub fn run_and_format<T: AsRef<str>>(
    command: impl BorrowMut<Command>,
    filters: impl AsRef<[(T, T)]>,
    function_name: &str,
    windows_filters: Option<WindowsFilters>,
    input: Option<&str>,
) -> (String, Output) {
    let (snapshot, output) =
        run_and_format_silent(command, filters, function_name, windows_filters, input);
    eprintln!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ Unfiltered output ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    eprintln!(
        "----- exit status -----\n{}\n----- stdout -----\n{}\n----- stderr -----\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    eprintln!("────────────────────────────────────────────────────────────────────────────────\n");
    (snapshot, output)
}

/// Execute the command and format its output without printing the unfiltered output.
#[doc(hidden)]
pub fn run_and_format_silent<T: AsRef<str>>(
    mut command: impl BorrowMut<Command>,
    filters: impl AsRef<[(T, T)]>,
    function_name: &str,
    windows_filters: Option<WindowsFilters>,
    input: Option<&str>,
) -> (String, Output) {
    assert_effective_cache_directory(command.borrow_mut());

    let program = command
        .borrow_mut()
        .get_program()
        .to_string_lossy()
        .to_string();

    // Support profiling test run commands with traces.
    if let Some(root) = env::var_os(EnvVars::TRACING_DURATIONS_TEST_ROOT) {
        // We only want to fail if the variable is set at runtime.
        #[expect(clippy::assertions_on_constants)]
        {
            assert!(
                cfg!(feature = "tracing-durations-export"),
                "You need to enable the tracing-durations-export feature to use `TRACING_DURATIONS_TEST_ROOT`"
            );
        }
        command.borrow_mut().env(
            EnvVars::TRACING_DURATIONS_FILE,
            Path::new(&root).join(function_name).with_extension("jsonl"),
        );
    }

    let output = if let Some(input) = input {
        let mut child = command
            .borrow_mut()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|err| panic!("Failed to spawn {program}: {err}"));
        let mut stdin = child.stdin.take().expect("Failed to open stdin");
        thread::scope(|scope| {
            // Drain stdout and stderr while writing stdin so neither side can fill its pipe and
            // block waiting for the other.
            let writer = scope.spawn(move || stdin.write_all(input.as_bytes()));
            let output = child.wait_with_output();
            // A command can exit without consuming all its input. Its status and output still
            // explain the result, so do not replace them with a broken-pipe error from the writer.
            if let Err(err) = writer.join().expect("Stdin writer panicked")
                && err.kind() != std::io::ErrorKind::BrokenPipe
            {
                panic!("Failed to write to {program} stdin: {err}");
            }
            output.unwrap_or_else(|err| panic!("Failed to read output from {program}: {err}"))
        })
    } else {
        command
            .borrow_mut()
            .output()
            .unwrap_or_else(|err| panic!("Failed to spawn {program}: {err}"))
    };

    let mut snapshot = format!(
        "exit_code: {} ({})\n",
        output.status.code().unwrap_or(!0),
        if output.status.success() {
            "success"
        } else {
            "failure"
        },
    );
    if output.status.code().is_none() {
        snapshot.push_str("exit_status: ");
        snapshot.push_str(&output.status.to_string());
        snapshot.push('\n');
    }
    if !output.stdout.is_empty() {
        snapshot.push_str("----- stdout -----\n");
        snapshot.push_str(&String::from_utf8_lossy(&output.stdout));
    }
    if !output.stderr.is_empty() {
        if !output.stdout.is_empty() {
            snapshot.push('\n');
        }
        snapshot.push_str("----- stderr -----\n");
        snapshot.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    let mut snapshot = apply_filters(snapshot, filters);

    // This is a heuristic filter meant to try and make *most* of our tests
    // pass whether it's on Windows or Unix. In particular, there are some very
    // common Windows-only dependencies that, when removed from a resolution,
    // cause the set of dependencies to be the same across platforms.
    if cfg!(windows) {
        if let Some(windows_filters) = windows_filters {
            // The optional leading +/-/~ is for install logs, the optional next line is for lockfiles
            let windows_only_deps = [
                (r"( ?[-+~] ?)?colorama==\d+(\.\d+)+( [\\]\n\s+--hash=.*)?\n(\s+# via .*\n)?"),
                (r"( ?[-+~] ?)?colorama==\d+(\.\d+)+(\s+[-+~]?\s+# via .*)?\n"),
                (r"( ?[-+~] ?)?tzdata==\d+(\.\d+)+( [\\]\n\s+--hash=.*)?\n(\s+# via .*\n)?"),
                (r"( ?[-+~] ?)?tzdata==\d+(\.\d+)+(\s+[-+~]?\s+# via .*)?\n"),
            ];
            let mut removed_packages = 0;
            for windows_only_dep in windows_only_deps {
                // TODO(konstin): Cache regex compilation
                let re = Regex::new(windows_only_dep).unwrap();
                if re.is_match(&snapshot) {
                    snapshot = re.replace(&snapshot, "").to_string();
                    removed_packages += 1;
                }
            }
            if removed_packages > 0 {
                for i in 1..20 {
                    for verb in match windows_filters {
                        WindowsFilters::Platform => [
                            "Resolved",
                            "Prepared",
                            "Installed",
                            "Checked",
                            "Uninstalled",
                        ]
                        .iter(),
                        WindowsFilters::Universal => {
                            ["Prepared", "Installed", "Checked", "Uninstalled"].iter()
                        }
                    } {
                        snapshot = snapshot.replace(
                            &format!("{verb} {} packages", i + removed_packages),
                            &format!("{verb} {} package{}", i, if i > 1 { "s" } else { "" }),
                        );
                    }
                }
            }
        }
    }

    (snapshot, output)
}

/// Reject cache environment overrides hidden by an explicit cache-directory argument.
///
/// Context commands always include `--cache-dir`, so setting `UV_CACHE_DIR` after constructing
/// one cannot change its cache. Check the completed command immediately before execution so
/// snapshots cannot silently pass without exercising their intended cache configuration.
fn assert_effective_cache_directory(command: &Command) {
    let cache_directory_override = command
        .get_envs()
        .find(|(name, value)| *name == EnvVars::UV_CACHE_DIR && value.is_some());

    if cache_directory_override.is_none() {
        return;
    }

    let explicit_cache_directory = command
        .get_args()
        .take_while(|argument| *argument != "--")
        .any(|argument| {
            argument == "--cache-dir"
                || argument
                    .to_str()
                    .is_some_and(|argument| argument.starts_with("--cache-dir="))
        });

    assert!(
        !explicit_cache_directory,
        "`UV_CACHE_DIR` is ignored because this command already supplies `--cache-dir`; configure `TestContext::cache_dir` instead"
    );
}

/// Creates a "unified" diff between the two line-oriented strings suitable
/// for snapshotting.
pub fn diff_snapshot(old: &str, new: &str, context_radius: usize) -> String {
    let diff = similar::TextDiff::from_lines(old, new);
    let unified = diff
        .unified_diff()
        .context_radius(context_radius)
        .header("old", "new")
        .to_string();
    // Not totally clear why, but some lines end up containing only
    // whitespace in the diff, even though they don't appear in the
    // original data. So just strip them here.
    regex!(r"(?m)^\s+$").replace_all(&unified, "").into_owned()
}

/// Assert a snapshot of the diff between `old` and a command's output.
///
/// Returns the command's snapshot, this is useful for chaining diffs.
#[macro_export]
macro_rules! diff_uv_snapshot {
    ($filters:expr, $old:expr, $spawnable:expr, @$snapshot:literal) => {{
        let new = $crate::capture_uv_snapshot!($filters, $spawnable);
        let snapshot = $crate::diff_snapshot($old, &new, 3);
        let mut settings = ::insta::Settings::clone_current();
        // Show the complete diff on failure while avoiding assertions on its unstable metadata.
        let description = match settings.description() {
            Some(description) => format!("{description}\n\nUnfiltered diff:\n{snapshot}"),
            None => format!("Unfiltered diff:\n{snapshot}"),
        };
        settings.set_description(description);
        settings.add_filter(r"^--- old\n\+\+\+ new\n", "");
        settings.add_filter(r"(?m)^@@.*$", "...");
        settings.add_filter(r"\n$", "\n...\n");
        settings.bind(|| {
            ::insta::assert_snapshot!(snapshot, @$snapshot);
        });
        new
    }};
}

/// Capture a command's output, optionally asserting it against a snapshot.
#[macro_export]
macro_rules! capture_uv_snapshot {
    ($filters:expr, $spawnable:expr) => {{
        // Don't echo the output to stderr while capturing without asserting.
        let (snapshot, _) = $crate::run_and_format_silent(
            $spawnable,
            &$filters,
            $crate::function_name!(),
            Some($crate::WindowsFilters::Platform),
            None,
        );
        snapshot
    }};
    ($filters:expr, $spawnable:expr, @$snapshot:literal) => {{
        let (snapshot, _) = $crate::run_and_format(
            $spawnable,
            &$filters,
            $crate::function_name!(),
            Some($crate::WindowsFilters::Platform),
            None,
        );
        ::insta::assert_snapshot!(snapshot, @$snapshot);
        snapshot
    }};
}

/// Utility macro to return the name of the current function.
///
/// https://stackoverflow.com/a/40234666/3549270
#[doc(hidden)]
#[macro_export]
macro_rules! function_name {
    () => {{
        fn f() {}
        fn type_name_of_val<T>(_: T) -> &'static str {
            std::any::type_name::<T>()
        }
        let mut name = type_name_of_val(f).strip_suffix("::f").unwrap_or("");
        while let Some(rest) = name.strip_suffix("::{{closure}}") {
            name = rest;
        }
        name
    }};
}

/// Run [`assert_cmd_snapshot!`], with default filters or with custom filters.
///
/// By default, the filters will search for the generally windows-only deps colorama and tzdata,
/// filter them out and decrease the package counts by one for each match.
#[macro_export]
macro_rules! uv_snapshot {
    ($spawnable:expr, @$snapshot:literal) => {{
        uv_snapshot!($crate::INSTA_FILTERS.to_vec(), $spawnable, @$snapshot)
    }};
    ($filters:expr, $spawnable:expr, @$snapshot:literal) => {{
        // Take a reference for backwards compatibility with the vec-expecting insta filters.
        let (snapshot, output) = $crate::run_and_format($spawnable, &$filters, $crate::function_name!(), Some($crate::WindowsFilters::Platform), None);
        ::insta::assert_snapshot!(snapshot, @$snapshot);
        output
    }};
    ($filters:expr, $spawnable:expr, input=$input:expr, @$snapshot:literal) => {{
        // Take a reference for backwards compatibility with the vec-expecting insta filters.
        let (snapshot, output) = $crate::run_and_format($spawnable, &$filters, $crate::function_name!(), Some($crate::WindowsFilters::Platform), Some($input));
        ::insta::assert_snapshot!(snapshot, @$snapshot);
        output
    }};
    ($filters:expr, windows_filters=false, $spawnable:expr, @$snapshot:literal) => {{
        // Take a reference for backwards compatibility with the vec-expecting insta filters.
        let (snapshot, output) = $crate::run_and_format($spawnable, &$filters, $crate::function_name!(), None, None);
        ::insta::assert_snapshot!(snapshot, @$snapshot);
        output
    }};
    ($filters:expr, universal_windows_filters=true, $spawnable:expr, @$snapshot:literal) => {{
        // Take a reference for backwards compatibility with the vec-expecting insta filters.
        let (snapshot, output) = $crate::run_and_format($spawnable, &$filters, $crate::function_name!(), Some($crate::WindowsFilters::Universal), None);
        ::insta::assert_snapshot!(snapshot, @$snapshot);
        output
    }};
}

#[cfg(all(test, unix))]
mod process_status_tests {
    use std::process::Command;

    use super::run_and_format_silent;

    #[test]
    fn reports_signal() {
        let mut command = Command::new("sh");
        command.args(["-c", "kill -TERM $$"]);
        let filters: &[(&str, &str)] = &[];
        let (snapshot, _) = run_and_format_silent(command, filters, "reports_signal", None, None);

        insta::assert_snapshot!(snapshot, @"
        exit_code: -1 (failure)
        exit_status: signal: 15 (SIGTERM)
        ");
    }

    #[test]
    fn preserves_exit_code() {
        let mut command = Command::new("sh");
        command.args(["-c", "exit 7"]);
        let filters: &[(&str, &str)] = &[];
        let (snapshot, _) =
            run_and_format_silent(command, filters, "preserves_exit_code", None, None);

        insta::assert_snapshot!(snapshot, @"exit_code: 7 (failure)");
    }

    #[test]
    fn drains_output_while_writing_input() {
        let mut command = Command::new("sh");
        command.args([
            "-c",
            "dd if=/dev/zero bs=1024 count=2048 2>/dev/null && cat >/dev/null && printf done",
        ]);
        let input = "x".repeat(2 * 1024 * 1024);
        let filters: &[(&str, &str)] = &[];
        let (_, output) = run_and_format_silent(
            command,
            filters,
            "drains_output_while_writing_input",
            None,
            Some(&input),
        );

        assert!(output.status.success());
        assert_eq!(output.stdout.len(), input.len() + b"done".len());
        assert!(output.stdout.ends_with(b"done"));
    }

    #[test]
    fn captures_output_when_command_closes_stdin_early() {
        let mut command = Command::new("sh");
        command.args(["-c", "exec 0<&-; printf 'input rejected' >&2; exit 2"]);
        let input = "x".repeat(2 * 1024 * 1024);
        let filters: &[(&str, &str)] = &[];

        let (snapshot, _) = run_and_format_silent(
            command,
            filters,
            "captures_output_when_command_closes_stdin_early",
            None,
            Some(&input),
        );

        insta::assert_snapshot!(snapshot, @"
        exit_code: 2 (failure)
        ----- stderr -----
        input rejected
        ");
    }
}

#[cfg(test)]
mod cache_directory_tests {
    use std::process::Command;

    use uv_static::EnvVars;

    use super::assert_effective_cache_directory;

    #[test]
    #[should_panic(expected = "`UV_CACHE_DIR` is ignored")]
    fn rejects_environment_override_with_explicit_cache_argument() {
        let mut command = Command::new("uv");
        command
            .arg("--cache-dir")
            .arg("context-cache")
            .env(EnvVars::UV_CACHE_DIR, "ignored-cache");

        assert_effective_cache_directory(&command);
    }

    #[test]
    #[should_panic(expected = "`UV_CACHE_DIR` is ignored")]
    fn rejects_environment_override_with_inline_cache_argument() {
        let mut command = Command::new("uv");
        command
            .arg("--cache-dir=context-cache")
            .env(EnvVars::UV_CACHE_DIR, "ignored-cache");

        assert_effective_cache_directory(&command);
    }

    #[test]
    fn allows_environment_override_without_explicit_cache_argument() {
        let mut command = Command::new("uv");
        command
            .arg("cache")
            .arg("dir")
            .env(EnvVars::UV_CACHE_DIR, "effective-cache");

        assert_effective_cache_directory(&command);
    }

    #[test]
    fn allows_cache_argument_forwarded_to_child() {
        let mut command = Command::new("uv");
        command
            .args(["run", "--", "child", "--cache-dir=child-cache"])
            .env(EnvVars::UV_CACHE_DIR, "effective-cache");

        assert_effective_cache_directory(&command);
    }

    #[test]
    fn allows_removed_environment_override_with_explicit_cache_argument() {
        let mut command = Command::new("uv");
        command
            .arg("--cache-dir")
            .arg("context-cache")
            .env_remove(EnvVars::UV_CACHE_DIR);

        assert_effective_cache_directory(&command);
    }
}
