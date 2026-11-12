use std::io::Write;
use std::path::Path;
use std::str::FromStr;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use tracing::Subscriber;
#[cfg(feature = "tracing-durations-export")]
use tracing_durations_export::{
    DurationsLayer, DurationsLayerBuilder, DurationsLayerDropGuard, plot::PlotConfig,
};
use tracing_subscriber::filter::{Directive, LevelFilter, Targets};
use tracing_subscriber::fmt::time::uptime;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, Registry};
use tracing_tree::HierarchicalLayer;
use tracing_tree::time::Uptime;

use uv_configuration::ColorChoice;
use uv_logging::{UvFormat, uv_fields};
use uv_static::EnvVars;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Level {
    #[default]
    Off,
    DebugUv,
    TraceUv,
    TraceAll,
}

/// Configure `tracing` based on the given [`Level`], taking into account the `RUST_LOG` environment
/// variable.
///
/// The [`Level`] is used to dictate the default filters (which can be overridden by the `RUST_LOG`
/// environment variable) along with the formatting of the output. For example, [`Level::Verbose`]
/// includes targets and timestamps, along with all `uv=debug` messages by default.
pub(crate) fn setup_logging(
    level: Level,
    durations_layer: Option<impl Layer<Registry> + Send + Sync>,
    color: ColorChoice,
    detailed_logging: bool,
) -> anyhow::Result<()> {
    // We use directives here to ensure `RUST_LOG` can override them
    let default_directive = match level {
        Level::Off => {
            // Show nothing
            tracing::level_filters::LevelFilter::OFF.into()
        }
        Level::DebugUv => {
            // Show `DEBUG` messages from the CLI crate (and ERROR/WARN/INFO)
            Directive::from_str("uv=debug").unwrap()
        }
        Level::TraceUv => {
            // Show `TRACE` messages from the CLI crate (and ERROR/WARN/INFO/DEBUG)
            Directive::from_str("uv=trace").unwrap()
        }
        Level::TraceAll => {
            // Show all `TRACE` messages (and ERROR/WARN/INFO/DEBUG)
            Directive::from_str("trace").unwrap()
        }
    };

    // Avoid setting the default log level to INFO
    let durations_layer = durations_layer.map(|durations_layer| {
        durations_layer.with_filter(
            // Only record our own spans
            tracing_subscriber::filter::Targets::new()
                .with_target("", tracing::level_filters::LevelFilter::INFO),
        )
    });
    let filter = EnvFilter::builder()
        .with_default_directive(default_directive)
        .from_env()
        .context("Invalid RUST_LOG directives")?;
    let test_log_layer = std::env::var_os(EnvVars::UV_INTERNAL__TEST_LOG_DIR)
        .filter(|value| !value.is_empty())
        .map(|directory| test_log_layer(Path::new(&directory)))
        .transpose()?;

    // Determine our final color settings and create an anstream wrapper based on it.
    //
    // The tracing `with_ansi` function on affects color tracing adds *on top of* the
    // log messages. This means that if we `debug!("{}", "hello".green())`,
    // (a thing we absolutely do throughout uv), then there will still be color
    // in the logs, which is undesirable.
    //
    // So we tell tracing to print to an anstream wrapper around stderr that force-strips ansi.
    // Given we do this, using `with_ansi` at all is arguably pointless, but it feels morally
    // correct to still do it? I don't know what would break if we didn't... but why find out?
    let (ansi, color_choice) =
        match color.and_colorchoice(anstream::Stderr::choice(&std::io::stderr())) {
            ColorChoice::Always => (true, anstream::ColorChoice::Always),
            ColorChoice::Never => (false, anstream::ColorChoice::Never),
            ColorChoice::Auto => unreachable!("anstream can't return auto as choice"),
        };
    let writer = std::sync::Mutex::new(anstream::AutoStream::new(std::io::stderr(), color_choice));

    if detailed_logging {
        // Regardless of the tracing level, include the uptime and target for each message.
        tracing_subscriber::registry()
            .with(durations_layer)
            .with(test_log_layer)
            .with(
                HierarchicalLayer::default()
                    .with_targets(true)
                    .with_timer(Uptime::default())
                    .with_writer(writer)
                    .with_ansi(ansi)
                    .with_filter(filter),
            )
            .init();
    } else {
        tracing_subscriber::registry()
            .with(durations_layer)
            .with(test_log_layer)
            .with(
                tracing_subscriber::fmt::layer()
                    .event_format(UvFormat)
                    .fmt_fields(uv_fields())
                    .with_writer(writer)
                    .with_ansi(ansi)
                    .with_filter(filter),
            )
            .init();
    }

    Ok(())
}

fn test_log_layer<S>(directory: &Path) -> anyhow::Result<impl Layer<S> + Send + Sync + use<S>>
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fs_err::create_dir_all(directory).context("Failed to create test diagnostic directory")?;
    let (file, path) = tempfile::Builder::new()
        .prefix(&format!("uv-{}-", std::process::id()))
        .suffix(".log")
        .tempfile_in(directory)
        .context("Failed to create test diagnostic log")?
        .keep()
        .context("Failed to retain test diagnostic log")?;
    let mut file = fs_err::File::from_parts(file, path);
    writeln!(
        file,
        "pid={} started_at_unix_ns={} cwd={}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos(),
        std::env::current_dir()?.display()
    )
    .context("Failed to write test diagnostic log")?;

    // Direct writes keep completed trace events available if the test runner terminates a stalled
    // subprocess.
    // A separate layer keeps diagnostics out of command output snapshots.
    Ok(tracing_subscriber::fmt::layer()
        .fmt_fields(uv_fields())
        .with_ansi(false)
        .with_timer(uptime())
        .with_writer(Mutex::new(file))
        .log_internal_errors(true)
        .with_filter(
            Targets::new()
                .with_target("uv_python_managed", LevelFilter::TRACE)
                .with_target("uv_python_discovery", LevelFilter::TRACE)
                .with_target("uv_python_interpreter", LevelFilter::TRACE)
                .with_target("uv_python_commands", LevelFilter::TRACE)
                .with_target("uv_fs::locked_file", LevelFilter::TRACE)
                .with_target("uv_client::base_client", LevelFilter::DEBUG)
                .with_target("uv_client::retry", LevelFilter::TRACE),
        ))
}

/// Setup the `TRACING_DURATIONS_FILE` environment variable to enable tracing durations.
#[cfg(feature = "tracing-durations-export")]
pub(crate) fn setup_durations(
    tracing_durations_file: Option<&std::path::PathBuf>,
) -> anyhow::Result<(
    Option<DurationsLayer<Registry>>,
    Option<DurationsLayerDropGuard>,
)> {
    if let Some(location) = tracing_durations_file {
        if let Some(parent) = location.parent() {
            fs_err::create_dir_all(parent)
                .context("Failed to create parent of TRACING_DURATIONS_FILE")?;
        }
        let plot_config = PlotConfig {
            multi_lane: true,
            min_length: None,
            remove: Some(
                ["get_cached_with_callback".to_string()]
                    .into_iter()
                    .collect(),
            ),
            ..PlotConfig::default()
        };
        let (layer, guard) = DurationsLayerBuilder::default()
            .durations_file(location)
            .plot_file(location.with_extension("svg"))
            .plot_config(plot_config)
            .build()
            .context("Couldn't create TRACING_DURATIONS_FILE files")?;
        Ok((Some(layer), Some(guard)))
    } else {
        Ok((None, None))
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Context;
    use tracing_subscriber::Layer;
    use tracing_subscriber::filter::LevelFilter;
    use tracing_subscriber::layer::SubscriberExt;

    use super::test_log_layer;

    /// Timeout diagnostics must reach disk before the child exits, even with stderr logging off.
    #[test]
    fn test_diagnostics_are_unbuffered_and_independent_of_stderr() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let subscriber = tracing_subscriber::registry()
            .with(test_log_layer(directory.path())?)
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(std::io::sink)
                    .with_filter(LevelFilter::OFF),
            );
        let dispatch = tracing::Dispatch::new(subscriber);
        let guard = tracing::dispatcher::set_default(&dispatch);

        tracing::trace!(target: "uv_python_managed::downloads", "archive cache miss");
        tracing::trace!(
            target: "uv_python_interpreter::interpreter",
            executable = %"\x1b[1mpython.exe\x1b[0m",
            "querying {}",
            "\x1b[36minterpreter\x1b[0m"
        );
        tracing::debug!(target: "uv_client::retry", "retrying request");
        tracing::trace!(target: "uv_auth", "authentication details");

        let path = fs_err::read_dir(directory.path())?
            .next()
            .context("Expected a test diagnostic log")??
            .path();
        let contents = fs_err::read_to_string(&path)?;
        insta::with_settings!({
            filters => [
                (r"pid=\d+ started_at_unix_ns=\d+ cwd=[^\n]+", "pid=[PID] started_at_unix_ns=[TIMESTAMP] cwd=[CWD]"),
                (r"[ ]*\d+\.\d+s", "[TIME]"),
            ],
        }, {
            insta::assert_snapshot!(contents, @"
            pid=[PID] started_at_unix_ns=[TIMESTAMP] cwd=[CWD]
            [TIME] TRACE uv_python_managed::downloads: archive cache miss
            [TIME] TRACE uv_python_interpreter::interpreter: querying interpreter executable=python.exe
            [TIME] DEBUG uv_client::retry: retrying request
            ");
        });

        drop(guard);
        drop(dispatch);
        assert!(path.is_file());
        Ok(())
    }

    #[test]
    fn test_diagnostic_directory_errors_are_reported() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("not-a-directory");
        fs_err::write(&path, "")?;

        let error = test_log_layer::<tracing_subscriber::Registry>(&path)
            .err()
            .context("Expected an invalid diagnostic directory to fail")?;
        insta::assert_snapshot!(error.to_string(), @"Failed to create test diagnostic directory");
        Ok(())
    }
}
