/// The format to use when exporting a `uv.lock` file.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum ExportFormat {
    /// Export in `requirements.txt` format.
    #[default]
    #[serde(rename = "requirements.txt", alias = "requirements-txt")]
    #[cfg_attr(
        feature = "clap",
        clap(name = "requirements.txt", alias = "requirements-txt")
    )]
    RequirementsTxt,
    /// Export in `pylock.toml` format.
    #[serde(rename = "pylock.toml", alias = "pylock-toml")]
    #[cfg_attr(feature = "clap", clap(name = "pylock.toml", alias = "pylock-toml"))]
    PylockToml,
    /// Export in `CycloneDX` v1.5 JSON format.
    #[serde(rename = "cyclonedx1.5")]
    #[cfg_attr(
        feature = "clap",
        clap(name = "cyclonedx1.5", alias = "cyclonedx1.5+json")
    )]
    CycloneDX1_5,
}

/// The output format to use in `uv pip compile`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum PipCompileFormat {
    /// Export in `requirements.txt` format.
    #[default]
    #[serde(rename = "requirements.txt", alias = "requirements-txt")]
    #[cfg_attr(
        feature = "clap",
        clap(name = "requirements.txt", alias = "requirements-txt")
    )]
    RequirementsTxt,
    /// Export in `pylock.toml` format.
    #[serde(rename = "pylock.toml", alias = "pylock-toml")]
    #[cfg_attr(feature = "clap", clap(name = "pylock.toml", alias = "pylock-toml"))]
    PylockToml,
}

bitflags::bitflags! {
    /// The optional content to include in exported output.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct OutputFlags: u16 {
        /// Include distribution hashes.
        const HASHES = 1 << 0;
        /// Include extras in requirements.
        const EXTRAS = 1 << 1;
        /// Include environment markers in requirements.
        const MARKERS = 1 << 2;
        /// Include dependency annotations.
        const ANNOTATIONS = 1 << 3;
        /// Include the generated-file header.
        const HEADER = 1 << 4;
        /// Include index URLs.
        const INDEX_URL = 1 << 5;
        /// Include find-links locations.
        const FIND_LINKS = 1 << 6;
        /// Include build options.
        const BUILD_OPTIONS = 1 << 7;
        /// Include the marker expression.
        const MARKER_EXPRESSION = 1 << 8;
        /// Include index annotations.
        const INDEX_ANNOTATION = 1 << 9;
    }
}
