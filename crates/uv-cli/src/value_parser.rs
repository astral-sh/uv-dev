use std::ffi::{OsStr, OsString};

use clap::builder::{PossibleValue, TypedValueParser};
use clap::error::{ContextKind, ContextValue};
use clap::parser::ValueSource;

/// Prevent Clap from repeating a potentially credential-bearing argument in a parse error.
///
/// The underlying parser remains responsible for making its own error message safe to display.
#[derive(Clone, Debug)]
pub(crate) struct RedactedValueParser<P>(pub(crate) P);

impl<P: TypedValueParser> TypedValueParser for RedactedValueParser<P> {
    type Value = P::Value;

    fn parse_ref(
        &self,
        command: &clap::Command,
        arg: Option<&clap::Arg>,
        value: &OsStr,
    ) -> Result<Self::Value, clap::Error> {
        self.0.parse_ref(command, arg, value).map_err(redact)
    }

    fn parse_ref_(
        &self,
        command: &clap::Command,
        arg: Option<&clap::Arg>,
        value: &OsStr,
        source: ValueSource,
    ) -> Result<Self::Value, clap::Error> {
        self.0
            .parse_ref_(command, arg, value, source)
            .map_err(redact)
    }

    fn parse(
        &self,
        command: &clap::Command,
        arg: Option<&clap::Arg>,
        value: OsString,
    ) -> Result<Self::Value, clap::Error> {
        self.0.parse(command, arg, value).map_err(redact)
    }

    fn parse_(
        &self,
        command: &clap::Command,
        arg: Option<&clap::Arg>,
        value: OsString,
        source: ValueSource,
    ) -> Result<Self::Value, clap::Error> {
        self.0.parse_(command, arg, value, source).map_err(redact)
    }

    fn possible_values(&self) -> Option<Box<dyn Iterator<Item = PossibleValue> + '_>> {
        self.0.possible_values()
    }
}

fn redact(mut error: clap::Error) -> clap::Error {
    if error.get(ContextKind::InvalidValue).is_some() {
        error.insert(
            ContextKind::InvalidValue,
            ContextValue::String("****".to_owned()),
        );
    }
    error
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::ffi::{OsStr, OsString};

    use clap::builder::{PossibleValue, TypedValueParser};
    use clap::error::{ContextKind, ContextValue, ErrorKind};
    use clap::parser::ValueSource;
    use uv_distribution_types::{Index, Origin};

    use super::RedactedValueParser;
    use crate::{
        IndexArg, Maybe, parse_default_index, parse_extra_index_url, parse_find_links,
        parse_index_url, parse_indices,
    };

    #[test]
    fn keeps_validation_error_kind_and_cause() {
        let mut command = clap::Command::new("test").arg(clap::Arg::new("number").long("number"));
        command.build();
        let arg = command
            .get_arguments()
            .find(|arg| arg.get_id() == "number")
            .unwrap();
        let value = OsStr::new("secret");
        let parser = str::parse::<u8>;
        let original = parser.parse_ref(&command, Some(arg), value).unwrap_err();
        let redacted = RedactedValueParser(parser)
            .parse_ref(&command, Some(arg), value)
            .unwrap_err();

        assert_eq!(redacted.kind(), ErrorKind::ValueValidation);
        assert_eq!(redacted.kind(), original.kind());
        assert_eq!(
            redacted.get(ContextKind::InvalidValue),
            Some(&ContextValue::String("****".to_owned()))
        );
        assert_eq!(
            redacted.get(ContextKind::InvalidArg),
            original.get(ContextKind::InvalidArg)
        );
        assert_eq!(
            redacted.source().unwrap().to_string(),
            original.source().unwrap().to_string()
        );
        assert_eq!(
            redacted.to_string(),
            original.to_string().replace("'secret'", "'****'")
        );
    }

    #[test]
    fn preserves_other_errors() {
        #[derive(Clone)]
        struct InvalidUtf8;

        impl TypedValueParser for InvalidUtf8 {
            type Value = String;

            fn parse_ref(
                &self,
                command: &clap::Command,
                _arg: Option<&clap::Arg>,
                _value: &OsStr,
            ) -> Result<Self::Value, clap::Error> {
                Err(command
                    .clone()
                    .error(ErrorKind::InvalidUtf8, "invalid UTF-8"))
            }
        }

        let command = clap::Command::new("test");
        let value = OsStr::new("value");
        let original = InvalidUtf8.parse_ref(&command, None, value).unwrap_err();
        let redacted = RedactedValueParser(InvalidUtf8)
            .parse_ref(&command, None, value)
            .unwrap_err();
        assert_eq!(redacted.kind(), original.kind());
        assert_eq!(redacted.get(ContextKind::InvalidValue), None);
        assert_eq!(redacted.to_string(), original.to_string());
    }

    #[test]
    #[cfg(unix)]
    fn preserves_non_utf8_argument_errors() {
        use std::os::unix::ffi::OsStrExt;

        let mut command = clap::Command::new("test");
        command.build();
        let value = OsStr::from_bytes(b"\xff");
        let original = parse_index_url
            .parse_ref(&command, None, value)
            .unwrap_err();
        let redacted = RedactedValueParser(parse_index_url)
            .parse_ref(&command, None, value)
            .unwrap_err();
        assert_eq!(redacted.kind(), ErrorKind::InvalidUtf8);
        assert_eq!(redacted.get(ContextKind::InvalidValue), None);
        assert_eq!(redacted.to_string(), original.to_string());
    }

    #[test]
    fn delegates_value_source_and_completion() {
        #[derive(Clone)]
        struct SourceParser;

        impl TypedValueParser for SourceParser {
            type Value = (OsString, &'static str, Option<ValueSource>);

            fn parse_ref(
                &self,
                _command: &clap::Command,
                _arg: Option<&clap::Arg>,
                value: &OsStr,
            ) -> Result<Self::Value, clap::Error> {
                Ok((value.to_owned(), "borrowed", None))
            }

            fn parse_ref_(
                &self,
                _command: &clap::Command,
                _arg: Option<&clap::Arg>,
                value: &OsStr,
                source: ValueSource,
            ) -> Result<Self::Value, clap::Error> {
                Ok((value.to_owned(), "borrowed-source", Some(source)))
            }

            fn parse(
                &self,
                _command: &clap::Command,
                _arg: Option<&clap::Arg>,
                value: OsString,
            ) -> Result<Self::Value, clap::Error> {
                Ok((value, "owned", None))
            }

            fn parse_(
                &self,
                _command: &clap::Command,
                _arg: Option<&clap::Arg>,
                value: OsString,
                source: ValueSource,
            ) -> Result<Self::Value, clap::Error> {
                Ok((value, "owned-source", Some(source)))
            }

            fn possible_values(&self) -> Option<Box<dyn Iterator<Item = PossibleValue> + '_>> {
                Some(Box::new([PossibleValue::new("example")].into_iter()))
            }
        }

        let command = clap::Command::new("test");
        let parser = RedactedValueParser(SourceParser);
        assert_eq!(
            parser
                .parse_ref(&command, None, OsStr::new("value"))
                .unwrap(),
            (OsString::from("value"), "borrowed", None)
        );
        assert_eq!(
            parser
                .parse(&command, None, OsString::from("value"))
                .unwrap(),
            (OsString::from("value"), "owned", None)
        );
        for source in [
            ValueSource::DefaultValue,
            ValueSource::EnvVariable,
            ValueSource::CommandLine,
        ] {
            assert_eq!(
                parser
                    .parse_ref_(&command, None, OsStr::new("value"), source)
                    .unwrap(),
                (OsString::from("value"), "borrowed-source", Some(source))
            );
            assert_eq!(
                parser
                    .parse_(&command, None, OsString::from("value"), source)
                    .unwrap(),
                (OsString::from("value"), "owned-source", Some(source))
            );
        }
        assert_eq!(
            parser
                .possible_values()
                .unwrap()
                .map(|value| value.get_name().to_owned())
                .collect::<Vec<_>>(),
            ["example"]
        );
    }

    fn assert_optional_unchanged<P, T>(parser: P, value: &str)
    where
        P: TypedValueParser<Value = Maybe<T>>,
        T: std::fmt::Debug + PartialEq,
    {
        let command = clap::Command::new("test");
        let value = OsStr::new(value);
        let expected = parser
            .parse_ref(&command, None, value)
            .unwrap()
            .into_option();
        let actual = RedactedValueParser(parser)
            .parse_ref(&command, None, value)
            .unwrap()
            .into_option();
        assert_eq!(actual, expected);
    }

    #[test]
    fn preserves_index_values() {
        const URL: &str = "https://user:password@example.invalid/simple?sig=signature";
        for value in ["", "./local-index", URL] {
            assert_optional_unchanged(parse_default_index, value);
            assert_optional_unchanged(parse_index_url, value);
            assert_optional_unchanged(parse_extra_index_url, value);
            assert_optional_unchanged(parse_find_links, value);
        }
        for value in ["private", "private=https://example.invalid/simple"] {
            assert_optional_unchanged(parse_default_index, value);
        }

        let command = clap::Command::new("test");
        for value in [
            "",
            " \t\n",
            URL,
            "private",
            "./local-index",
            "private=https://example.invalid/simple",
            "https://example.invalid/one\tprivate\nhttps://example.invalid/two",
        ] {
            let expected = parse_indices(value)
                .unwrap()
                .into_iter()
                .map(Maybe::into_option)
                .collect::<Vec<_>>();
            let actual = RedactedValueParser(parse_indices)
                .parse_ref(&command, None, OsStr::new(value))
                .unwrap()
                .into_iter()
                .map(Maybe::into_option)
                .collect::<Vec<_>>();
            assert_eq!(actual, expected);
            for index in actual.into_iter().flatten() {
                if let IndexArg::Resolved(index) = index {
                    assert_eq!(index.origin, Some(Origin::Cli));
                    assert!(!index.default);
                }
            }
        }

        let index: Index = RedactedValueParser(parse_index_url)
            .parse_ref(&command, None, OsStr::new(URL))
            .unwrap()
            .into_option()
            .unwrap()
            .into();
        assert_eq!(index.origin, Some(Origin::Cli));
        assert!(index.default);
        assert_eq!(index.raw_url().as_str(), URL);
    }
}
