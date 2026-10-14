use std::fmt::{self, Display};

use owo_colors::OwoColorize;
use uv_platform_tags::AbiTag;

/// The artifact described by an incompatibility hint.
#[derive(Clone, Copy)]
pub(crate) enum CompatibilitySubject {
    /// A wheel rejected with a user-facing error.
    Wheel,
    /// An installed distribution rejected with a debug message.
    Distribution,
}

impl CompatibilitySubject {
    /// Describe an incompatible tag set and, if known, the active environment's tag.
    pub(crate) fn write_tag_hint<'tag, T: Display + 'tag, P: Display>(
        self,
        writer: &mut impl fmt::Write,
        wheel_tags: impl IntoIterator<Item = &'tag T>,
        current: Option<&T>,
        current_phrase: &str,
        pretty: impl Fn(&T) -> Option<P>,
    ) -> fmt::Result {
        let subject = self.as_str();
        if current.is_some() {
            write!(writer, "The {subject} is compatible with ")?;
        } else {
            write!(writer, "The {subject} requires ")?;
        }
        write_tag_list(writer, wheel_tags, |writer, tag| {
            self.write_pretty_tag(writer, tag, pretty(tag))
        })?;
        if let Some(current) = current {
            write!(writer, ", but you're {current_phrase} ")?;
            self.write_pretty_tag(writer, current, pretty(current))?;
        }
        Ok(())
    }

    /// Describe an ABI that requires a GIL-enabled Python interpreter.
    pub(crate) fn write_freethreaded_hint<'tag>(
        self,
        writer: &mut impl fmt::Write,
        wheel_tags: impl IntoIterator<Item = &'tag AbiTag>,
        current: Option<&AbiTag>,
    ) -> fmt::Result {
        writer.write_str("You're using ")?;
        if let Some(current) = current {
            self.write_pretty_tag(writer, current, current.pretty())?;
        } else {
            writer.write_str("free-threaded Python")?;
        }
        write!(writer, ", but the {} was built for ", self.as_str())?;
        write_tag_list(writer, wheel_tags, |writer, tag| match tag {
            AbiTag::Abi3 => write!(writer, "the stable ABI (`{}`)", self.highlight(tag)),
            _ => {
                if let Some(pretty) = tag.pretty() {
                    write!(
                        writer,
                        "the {} ABI (`{}`)",
                        self.highlight(pretty),
                        self.highlight(tag)
                    )
                } else {
                    write!(writer, "`{}`", self.highlight(tag))
                }
            }
        })?;
        writer.write_str(", which requires a GIL-enabled interpreter")
    }

    fn write_pretty_tag(
        self,
        writer: &mut impl fmt::Write,
        tag: impl Display,
        pretty: Option<impl Display>,
    ) -> fmt::Result {
        if let Some(pretty) = pretty {
            write!(
                writer,
                "{} (`{}`)",
                self.highlight(pretty),
                self.highlight(tag)
            )
        } else {
            write!(writer, "`{}`", self.highlight(tag))
        }
    }

    fn highlight(self, value: impl Display) -> impl Display {
        fmt::from_fn(move |f| match self {
            Self::Wheel => write!(f, "{}", value.cyan()),
            Self::Distribution => write!(f, "{value}"),
        })
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Wheel => "wheel",
            Self::Distribution => "distribution",
        }
    }
}

fn write_tag_list<W: fmt::Write, T>(
    writer: &mut W,
    tags: impl IntoIterator<Item = T>,
    mut write_tag: impl FnMut(&mut W, T) -> fmt::Result,
) -> fmt::Result {
    for (index, tag) in tags.into_iter().enumerate() {
        if index > 0 {
            writer.write_str(", ")?;
        }
        write_tag(writer, tag)?;
    }
    Ok(())
}
