use std::fmt;

use owo_colors::OwoColorize;

use uv_distribution_filename::{ExpandedTags, WheelFilename};
use uv_platform_tags::{AbiTag, IncompatibleTag, LanguageTag, PlatformTag, TagCompatibility, Tags};

/// A hint describing why a wheel or installed distribution is incompatible.
#[derive(Debug)]
pub(crate) struct CompatibilityHint {
    source: HintSource,
    incompatibility: Incompatibility,
}

/// Wheel errors are user-facing and colored, while installed distribution hints are debug logs.
#[derive(Debug, Clone, Copy)]
enum HintSource {
    Wheel,
    Distribution,
}

impl HintSource {
    fn name(self) -> &'static str {
        match self {
            Self::Wheel => "wheel",
            Self::Distribution => "distribution",
        }
    }

    fn format_value(self, value: impl fmt::Display) -> String {
        match self {
            Self::Wheel => value.cyan().to_string(),
            Self::Distribution => value.to_string(),
        }
    }

    fn format_tag(self, tag: impl fmt::Display, pretty: Option<impl fmt::Display>) -> String {
        if let Some(pretty) = pretty {
            format!(
                "{} (`{}`)",
                self.format_value(pretty),
                self.format_value(tag)
            )
        } else {
            format!("`{}`", self.format_value(tag))
        }
    }

    fn language_tag(self, tag: LanguageTag) -> String {
        self.format_tag(tag, tag.pretty())
    }

    fn language_tags(self, tags: &[LanguageTag]) -> String {
        tags.iter()
            .map(|tag| self.language_tag(*tag))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn abi_tag(self, tag: AbiTag) -> String {
        self.format_tag(tag, tag.pretty())
    }

    fn abi_tags(self, tags: &[AbiTag]) -> String {
        tags.iter()
            .map(|tag| self.abi_tag(*tag))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn platform_tag(self, tag: &PlatformTag) -> String {
        self.format_tag(tag, tag.pretty())
    }

    fn platform_tags(self, tags: &[PlatformTag]) -> String {
        tags.iter()
            .map(|tag| self.platform_tag(tag))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[derive(Debug)]
enum Incompatibility {
    Python {
        wheel_tags: Vec<LanguageTag>,
        current: Option<LanguageTag>,
    },
    Abi {
        wheel_tags: Vec<AbiTag>,
        current: Option<AbiTag>,
    },
    FreethreadedAbi {
        wheel_tags: Vec<AbiTag>,
        current: Option<AbiTag>,
    },
    Platform {
        wheel_tags: Vec<PlatformTag>,
        current: Option<PlatformTag>,
    },
}

impl CompatibilityHint {
    pub(crate) fn from_wheel(filename: &WheelFilename, tags: &Tags) -> Option<Self> {
        Self::new(
            HintSource::Wheel,
            filename.compatibility(tags),
            filename.python_tags().iter(),
            filename.abi_tags().iter(),
            filename.platform_tags().iter(),
            tags,
        )
    }

    pub(crate) fn from_distribution(wheel_tags: &ExpandedTags, tags: &Tags) -> Option<Self> {
        Self::new(
            HintSource::Distribution,
            wheel_tags.compatibility(tags),
            wheel_tags.python_tags(),
            wheel_tags.abi_tags(),
            wheel_tags.platform_tags(),
            tags,
        )
    }

    fn new<'a>(
        source: HintSource,
        compatibility: TagCompatibility,
        python_tags: impl Iterator<Item = &'a LanguageTag>,
        abi_tags: impl Iterator<Item = &'a AbiTag>,
        platform_tags: impl Iterator<Item = &'a PlatformTag>,
        tags: &Tags,
    ) -> Option<Self> {
        let TagCompatibility::Incompatible(incompatible_tag) = compatibility else {
            return None;
        };

        let incompatibility = match incompatible_tag {
            IncompatibleTag::Python => Incompatibility::Python {
                wheel_tags: python_tags.copied().collect(),
                current: tags.python_tag(),
            },
            IncompatibleTag::Abi => Incompatibility::Abi {
                wheel_tags: abi_tags.copied().collect(),
                current: tags.abi_tag(),
            },
            IncompatibleTag::FreethreadedAbi => Incompatibility::FreethreadedAbi {
                wheel_tags: abi_tags.copied().collect(),
                current: tags.abi_tag(),
            },
            IncompatibleTag::Platform => Incompatibility::Platform {
                wheel_tags: platform_tags.cloned().collect(),
                current: tags.platform_tag().cloned(),
            },
            IncompatibleTag::Invalid | IncompatibleTag::AbiPythonVersion => return None,
        };
        Some(Self {
            source,
            incompatibility,
        })
    }
}

impl fmt::Display for CompatibilityHint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let source = self.source;
        let name = source.name();
        match &self.incompatibility {
            Incompatibility::Python {
                wheel_tags,
                current,
            } => {
                if let Some(current) = current {
                    write!(
                        f,
                        "The {name} is compatible with {}, but you're using {}",
                        source.language_tags(wheel_tags),
                        source.language_tag(*current),
                    )
                } else {
                    write!(
                        f,
                        "The {name} requires {}",
                        source.language_tags(wheel_tags)
                    )
                }
            }
            Incompatibility::Abi {
                wheel_tags,
                current,
            } => {
                if let Some(current) = current {
                    write!(
                        f,
                        "The {name} is compatible with {}, but you're using {}",
                        source.abi_tags(wheel_tags),
                        source.abi_tag(*current),
                    )
                } else {
                    write!(f, "The {name} requires {}", source.abi_tags(wheel_tags))
                }
            }
            Incompatibility::FreethreadedAbi {
                wheel_tags,
                current,
            } => {
                let current_display = if let Some(current) = current {
                    source.abi_tag(*current)
                } else {
                    "free-threaded Python".to_string()
                };
                let wheel_display = wheel_tags
                    .iter()
                    .map(|tag| match tag {
                        AbiTag::Abi3 => format!("the stable ABI (`{}`)", source.format_value(tag)),
                        _ => {
                            if let Some(pretty) = tag.pretty() {
                                format!(
                                    "the {} ABI (`{}`)",
                                    source.format_value(pretty),
                                    source.format_value(tag)
                                )
                            } else {
                                format!("`{}`", source.format_value(tag))
                            }
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(
                    f,
                    "You're using {current_display}, but the {name} was built for {wheel_display}, which requires a GIL-enabled interpreter"
                )
            }
            Incompatibility::Platform {
                wheel_tags,
                current,
            } => {
                if let Some(current) = current {
                    write!(
                        f,
                        "The {name} is compatible with {}, but you're on {}",
                        source.platform_tags(wheel_tags),
                        source.platform_tag(current),
                    )
                } else {
                    write!(
                        f,
                        "The {name} requires {}",
                        source.platform_tags(wheel_tags)
                    )
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use uv_platform_tags::{Arch, Os, Platform, TagsOptions};

    use super::*;

    #[test]
    fn distribution_compatibility_hints() -> Result<()> {
        let hints = [
            Incompatibility::Python {
                wheel_tags: vec!["cp311".parse()?, "py3".parse()?],
                current: Some("cp314".parse()?),
            },
            Incompatibility::Python {
                wheel_tags: vec!["none".parse()?],
                current: None,
            },
            Incompatibility::Abi {
                wheel_tags: vec!["cp311".parse()?, "abi3".parse()?],
                current: Some("cp314".parse()?),
            },
            Incompatibility::Abi {
                wheel_tags: vec!["none".parse()?],
                current: None,
            },
            Incompatibility::FreethreadedAbi {
                wheel_tags: vec!["abi3".parse()?, "cp314".parse()?, "none".parse()?],
                current: Some("cp314t".parse()?),
            },
            Incompatibility::FreethreadedAbi {
                wheel_tags: vec!["abi3".parse()?],
                current: None,
            },
            Incompatibility::Platform {
                wheel_tags: vec!["win_amd64".parse()?, "any".parse()?],
                current: Some("linux_x86_64".parse()?),
            },
            Incompatibility::Platform {
                wheel_tags: vec!["any".parse()?],
                current: None,
            },
        ];
        let hints = hints
            .into_iter()
            .map(|incompatibility| {
                let mut hint = CompatibilityHint {
                    source: HintSource::Distribution,
                    incompatibility,
                };
                let distribution = hint.to_string();
                hint.source = HintSource::Wheel;
                let wheel = hint.to_string();
                // The user-facing hint retains colors, but otherwise uses the same wording.
                assert_eq!(
                    anstream::adapter::strip_str(&wheel).to_string(),
                    distribution.replace("distribution", "wheel")
                );
                distribution
            })
            .collect::<Vec<_>>()
            .join("\n");

        insta::assert_snapshot!(hints, @"
        The distribution is compatible with CPython 3.11 (`cp311`), Python 3 (`py3`), but you're using CPython 3.14 (`cp314`)
        The distribution requires `none`
        The distribution is compatible with CPython 3.11 (`cp311`), `abi3`, but you're using CPython 3.14 (`cp314`)
        The distribution requires `none`
        You're using free-threaded CPython 3.14 (`cp314t`), but the distribution was built for the stable ABI (`abi3`), the CPython 3.14 ABI (`cp314`), `none`, which requires a GIL-enabled interpreter
        You're using free-threaded Python, but the distribution was built for the stable ABI (`abi3`), which requires a GIL-enabled interpreter
        The distribution is compatible with Windows (`win_amd64`), `any`, but you're on Linux (`linux_x86_64`)
        The distribution requires `any`
        ");
        Ok(())
    }

    #[test]
    fn expanded_tags_preserve_order_and_duplicates() -> Result<()> {
        let tags = Tags::from_env(
            Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 28,
                },
                Arch::X86_64,
            ),
            (3, 14),
            "cpython",
            (3, 14),
            TagsOptions {
                manylinux_compatible: true,
                gil_disabled: false,
                debug_enabled: false,
                is_cross: false,
            },
        )?;
        let expanded = ExpandedTags::parse([
            "cp311-cp311-win_amd64",
            "cp310-cp310-win_amd64",
            "cp311-cp311-linux_x86_64",
        ])?;
        let hint = CompatibilityHint::from_distribution(&expanded, &tags)
            .expect("The expanded tags are incompatible");
        insta::assert_snapshot!(hint, @"The distribution is compatible with CPython 3.11 (`cp311`), CPython 3.10 (`cp310`), CPython 3.11 (`cp311`), but you're using CPython 3.14 (`cp314`)");
        Ok(())
    }
}
