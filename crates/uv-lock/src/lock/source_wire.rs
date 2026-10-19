use std::fmt;

use serde::de::{
    DeserializeOwned, IgnoredAny, MapAccess, SeqAccess, Visitor,
    value::{Error, MapAccessDeserializer, StringDeserializer},
};
use serde::{Deserialize, Deserializer};

use super::SourceWire;

/// Read each source field once, retaining the precedence of the source variants.
impl<'de> Deserialize<'de> for SourceWire {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Default, Deserialize)]
        #[serde(default)]
        struct Fields {
            registry: StringField,
            git: StringField,
            url: StringField,
            subdirectory: StringField,
            path: StringField,
            directory: StringField,
            editable: StringField,
            r#virtual: StringField,
        }

        struct SourceVisitor;

        impl<'de> Visitor<'de> for SourceVisitor {
            type Value = Option<SourceWire>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a lockfile source table")
            }

            fn visit_bool<E>(self, _value: bool) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(None)
            }

            fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(None)
            }

            fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(None)
            }

            fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(None)
            }

            fn visit_str<E>(self, _value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(None)
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(None)
            }

            fn visit_seq<A>(self, sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                IgnoredAny.visit_seq(sequence)?;
                Ok(None)
            }

            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let fields = Fields::deserialize(MapAccessDeserializer::new(map))?;
                if let Some(registry) = fields.registry.decode() {
                    return Ok(Some(SourceWire::Registry { registry }));
                }
                if let Some(git) = fields.git.decode() {
                    return Ok(Some(SourceWire::Git { git }));
                }
                if let Some(url) = fields.url.decode() {
                    match fields.subdirectory {
                        StringField::Missing => {
                            return Ok(Some(SourceWire::Direct {
                                url,
                                subdirectory: None,
                            }));
                        }
                        StringField::Value(subdirectory) => {
                            if let Some(subdirectory) = StringField::Value(subdirectory).decode() {
                                return Ok(Some(SourceWire::Direct {
                                    url,
                                    subdirectory: Some(subdirectory),
                                }));
                            }
                        }
                        StringField::Invalid => {}
                    }
                }
                if let Some(path) = fields.path.decode() {
                    return Ok(Some(SourceWire::Path { path }));
                }
                if let Some(directory) = fields.directory.decode() {
                    return Ok(Some(SourceWire::Directory { directory }));
                }
                if let Some(editable) = fields.editable.decode() {
                    return Ok(Some(SourceWire::Editable { editable }));
                }
                if let Some(r#virtual) = fields.r#virtual.decode() {
                    return Ok(Some(SourceWire::Virtual { r#virtual }));
                }
                Ok(None)
            }
        }

        // Report a shape mismatch here so errors from reading the input retain their context.
        deserializer.deserialize_any(SourceVisitor)?.ok_or_else(|| {
            serde::de::Error::custom("data did not match any variant of untagged enum SourceWire")
        })
    }
}

/// A non-string field rules out its source variant, but not later variants.
/// Consume ignored compound values too, without buffering their contents.
#[derive(Default)]
enum StringField {
    #[default]
    Missing,
    Value(String),
    Invalid,
}

impl StringField {
    fn decode<T: DeserializeOwned>(self) -> Option<T> {
        match self {
            Self::Value(value) => T::deserialize(StringDeserializer::<Error>::new(value)).ok(),
            Self::Missing | Self::Invalid => None,
        }
    }
}

impl<'de> Deserialize<'de> for StringField {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct StringFieldVisitor;

        impl<'de> Visitor<'de> for StringFieldVisitor {
            type Value = StringField;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a source field")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(StringField::Value(value.to_owned()))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(StringField::Value(value))
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(StringField::Missing)
            }

            fn visit_bool<E>(self, _value: bool) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(StringField::Invalid)
            }

            fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(StringField::Invalid)
            }

            fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(StringField::Invalid)
            }

            fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(StringField::Invalid)
            }

            fn visit_seq<A>(self, sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                IgnoredAny.visit_seq(sequence)?;
                Ok(StringField::Invalid)
            }

            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                IgnoredAny.visit_map(map)?;
                Ok(StringField::Invalid)
            }
        }

        deserializer.deserialize_any(StringFieldVisitor)
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::SourceWire;

    #[test]
    fn source_variants() -> Result<(), toml::de::Error> {
        #[derive(Debug, Deserialize)]
        struct Sources {
            sources: Vec<SourceWire>,
        }
        let sources: Sources = toml::from_str(
            r#"
            sources = [
                { registry = "https://pypi.org/simple" },
                { registry = "../index" },
                { git = "https://example.org/repo?rev=main#abc" },
                { url = "https://example.org/package.whl", subdirectory = "src" },
                { path = "." },
                { directory = "project" },
                { editable = "../editable" },
                { virtual = "workspace" },
            ]
        "#,
        )?;
        insta::assert_debug_snapshot!(sources.sources, @r#"
        [
            Registry {
                registry: Url(
                    UrlString(
                        "https://pypi.org/simple",
                    ),
                ),
            },
            Registry {
                registry: Path(
                    PortablePathBuf(
                        "../index",
                    ),
                ),
            },
            Git {
                git: "https://example.org/repo?rev=main#abc",
            },
            Direct {
                url: UrlString(
                    "https://example.org/package.whl",
                ),
                subdirectory: Some(
                    PortablePathBuf(
                        "src",
                    ),
                ),
            },
            Path {
                path: PortablePathBuf(
                    "",
                ),
            },
            Directory {
                directory: PortablePathBuf(
                    "project",
                ),
            },
            Editable {
                editable: PortablePathBuf(
                    "../editable",
                ),
            },
            Virtual {
                virtual: PortablePathBuf(
                    "workspace",
                ),
            },
        ]
        "#);
        Ok(())
    }

    #[test]
    fn source_precedence() -> Result<(), toml::de::Error> {
        let source: SourceWire = toml::from_str(
            r#"
            path = "local.whl"
            git = "https://example.org/repo#abc"
            registry = "https://pypi.org/simple"
            unknown = { nested = [true, 42] }
        "#,
        )?;
        insta::assert_debug_snapshot!(source, @r#"
        Registry {
            registry: Url(
                UrlString(
                    "https://pypi.org/simple",
                ),
            ),
        }
        "#);
        Ok(())
    }

    /// A malformed earlier source field must not prevent a later variant matching.
    #[test]
    fn source_invalid_inactive_fields() -> Result<(), toml::de::Error> {
        let source: SourceWire = toml::from_str(
            r#"
            registry = { nested = [true, 42] }
            git = 2026-10-19T12:00:00Z
            url = "https://example.org/package.whl"
            subdirectory = false
            directory = "project"
        "#,
        )?;
        insta::assert_debug_snapshot!(source, @r#"
        Directory {
            directory: PortablePathBuf(
                "project",
            ),
        }
        "#);
        Ok(())
    }

    #[test]
    fn source_invalid() {
        insta::assert_snapshot!(toml::from_str::<SourceWire>("path = 42").unwrap_err(), @"data did not match any variant of untagged enum SourceWire");
    }
}
