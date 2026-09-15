use std::fmt;

use jiff::Timestamp;
use owo_colors::OwoColorize;
use tracing::{Event, Subscriber, field::Field};
use tracing_subscriber::field::MakeExt;
use tracing_subscriber::fmt::format::{self, Writer};
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;

/// The style of a uv logging line.
pub struct UvFormat {
    display_timestamp: bool,
    display_level: bool,
    show_spans: bool,
}

impl Default for UvFormat {
    /// Regardless of the tracing level, show messages without any adornment.
    fn default() -> Self {
        Self {
            display_timestamp: false,
            display_level: true,
            show_spans: false,
        }
    }
}

/// See <https://docs.rs/tracing-subscriber/0.3.18/src/tracing_subscriber/fmt/format/mod.rs.html#1026-1156>
impl<S, N> FormatEvent<S, N> for UvFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let meta = event.metadata();
        let ansi = writer.has_ansi_escapes();

        if self.display_timestamp {
            if ansi {
                write!(writer, "{} ", Timestamp::now().dimmed())?;
            } else {
                write!(writer, "{} ", Timestamp::now())?;
            }
        }

        if self.display_level {
            let level = meta.level();
            // Same colors as tracing
            if ansi {
                match *level {
                    tracing::Level::TRACE => write!(writer, "{} ", level.purple())?,
                    tracing::Level::DEBUG => write!(writer, "{} ", level.blue())?,
                    tracing::Level::INFO => write!(writer, "{} ", level.green())?,
                    tracing::Level::WARN => write!(writer, "{} ", level.yellow())?,
                    tracing::Level::ERROR => write!(writer, "{} ", level.red())?,
                }
            } else {
                write!(writer, "{level} ")?;
            }
        }

        if self.show_spans {
            let span = event.parent();
            let mut seen = false;

            let span = span
                .and_then(|id| ctx.span(id))
                .or_else(|| ctx.lookup_current());

            let scope = span.into_iter().flat_map(|span| span.scope().from_root());

            for span in scope {
                seen = true;
                if ansi {
                    write!(writer, "{}:", span.metadata().name().bold())?;
                } else {
                    write!(writer, "{}:", span.metadata().name())?;
                }
            }

            if seen {
                writer.write_char(' ')?;
            }
        }

        ctx.field_format().format_fields(writer.by_ref(), event)?;

        writeln!(writer)
    }
}

/// Return the field formatter for uv logging.
///
/// The event formatter is responsible for uv's own log colors, such as the level prefix. Field
/// values can come from arbitrary `Display` or `Debug` implementations, so strip any ANSI escape
/// sequences there before writing them to the log line.
pub fn uv_fields() -> impl for<'writer> FormatFields<'writer> {
    format::debug_fn(format_field)
        .display_messages()
        .delimited(" ")
}

fn format_field(writer: &mut Writer<'_>, field: &Field, value: &dyn fmt::Debug) -> fmt::Result {
    // NOTE: The various cases in this function match tracing-subscriber's default field formatting.
    // See: https://docs.rs/tracing-subscriber/0.3.23/src/tracing_subscriber/fmt/format/mod.rs.html#1303-1338

    let field = field.name();
    if field.starts_with("log.") {
        return Ok(());
    }

    let value = format!("{value:?}");
    let value = anstream::adapter::strip_str(&value);

    if field == "message" {
        write!(writer, "{value}")
    } else {
        write!(
            writer,
            "{}={value}",
            // Render `type=...` instead of `r#type=...`.
            field.strip_prefix("r#").unwrap_or(field)
        )
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::fmt;

    use tracing::{Callsite, Event, Level, field::Value, metadata::Kind};
    use tracing_subscriber::fmt::FormatFields;
    use tracing_subscriber::fmt::format::Writer;

    use super::uv_fields;

    fn format_fields(event: Event<'_>) -> String {
        let mut output = String::new();
        uv_fields()
            .format_fields(Writer::new(&mut output), event)
            .expect("field formatting should succeed");
        output
    }

    /// Unlike a string's `Debug` implementation, this emits control characters verbatim.
    struct RawDebug<'a>(&'a str);

    impl fmt::Debug for RawDebug<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(self.0)
        }
    }

    #[test]
    fn strips_ansi_from_message_fields() {
        let callsite = tracing::callsite! {
            name: "event",
            kind: Kind::EVENT,
            level: Level::TRACE,
            fields: message
        };
        let metadata = callsite.metadata();
        let message = format_args!("Error trace: {}", "\x1b[36m\x1b[1mhint\x1b[0m");
        let values = [Some(&message as &dyn Value)];
        let fields = metadata.fields().value_set_all(&values);
        let event = Event::new(metadata, &fields);

        assert_eq!(format_fields(event), "Error trace: hint");
    }

    #[test]
    fn strips_terminal_sequences_from_display_and_debug_fields() {
        let callsite = tracing::callsite! {
            name: "event",
            kind: Kind::EVENT,
            level: Level::TRACE,
            fields: message, display_value, debug_value
        };
        let metadata = callsite.metadata();

        for (input, expected) in [
            ("plain Unicode: λ", "plain Unicode: λ"),
            ("\x1b[31mred\x1b[0m", "red"),
            ("\x1b[2J\x1b[Hscreen", "screen"),
            (
                "\x1b]8;;https://example.invalid/\x07link\x1b]8;;\x07",
                "link",
            ),
            (
                "\x1b]8;;https://example.invalid/\x1b\\link\x1b]8;;\x1b\\",
                "link",
            ),
            ("\x1b]52;c;c2FmZQ==\x07visible", "visible"),
            ("\x1bP1;2|hidden\x1b\\visible", "visible"),
        ] {
            let display = tracing::field::display(input);
            let debug = tracing::field::debug(RawDebug(input));
            let values = [
                Some(&display as &dyn Value),
                Some(&display as &dyn Value),
                Some(&debug as &dyn Value),
            ];
            let fields = metadata.fields().value_set_all(&values);
            let event = Event::new(metadata, &fields);

            assert_eq!(
                format_fields(event),
                format!("{expected} display_value={expected} debug_value={expected}"),
                "input: {input:?}",
            );
        }
    }

    #[test]
    fn preserves_raw_identifier_field_formatting() {
        let callsite = tracing::callsite! {
            name: "event",
            kind: Kind::EVENT,
            level: Level::TRACE,
            fields: "r#type" = ""
        };
        let metadata = callsite.metadata();
        let value = tracing::field::display("\x1b[36mexample\x1b[0m");
        let values = [Some(&value as &dyn Value)];
        let fields = metadata.fields().value_set_all(&values);
        let event = Event::new(metadata, &fields);

        assert_eq!(format_fields(event), "type=example");
    }

    #[test]
    fn ignores_log_metadata_before_formatting_values() {
        struct ObservedDebug<'a>(&'a Cell<usize>);

        impl fmt::Debug for ObservedDebug<'_> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.set(self.0.get() + 1);
                f.write_str("should not be formatted")
            }
        }

        let callsite = tracing::callsite! {
            name: "event",
            kind: Kind::EVENT,
            level: Level::TRACE,
            fields: log.target
        };
        let metadata = callsite.metadata();
        let calls = Cell::new(0);
        let value = tracing::field::debug(ObservedDebug(&calls));
        let values = [Some(&value as &dyn Value)];
        let fields = metadata.fields().value_set_all(&values);
        let event = Event::new(metadata, &fields);

        assert_eq!(format_fields(event), "");
        assert_eq!(calls.get(), 0);
    }
}
