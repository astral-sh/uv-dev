use std::borrow::Cow;
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::LazyLock;

use memchr::memmem::Finder;
use serde::Deserialize;
use thiserror::Error;
use tracing::instrument;
use url::Url;

use uv_configuration::NoSources;
use uv_normalize::PackageName;
use uv_pep440::VersionSpecifiers;
use uv_pypi_types::VerbatimParsedUrl;
use uv_redacted::DisplaySafeUrl;
use uv_settings::{GlobalOptions, ResolverInstallerSchema};
use uv_warnings::warn_user;
use uv_workspace::pyproject::{ExtraBuildDependency, Sources};

pub use uv_configuration::ExcludeDependency;
pub use uv_workspace::pyproject::OverrideDependency;

static FINDER: LazyLock<Finder> = LazyLock::new(|| Finder::new(b"# /// script"));

/// A PEP 723 item, either read from a script on disk or provided via `stdin`.
#[derive(Debug)]
pub enum Pep723Item {
    /// A PEP 723 script read from disk.
    Script(Pep723Script),
    /// A PEP 723 script provided via `stdin`.
    Stdin(Pep723Metadata),
    /// A PEP 723 script provided via a remote URL.
    Remote(Pep723Metadata, DisplaySafeUrl),
}

impl Pep723Item {
    /// Return the [`Pep723Metadata`] associated with the item.
    pub fn metadata(&self) -> &Pep723Metadata {
        match self {
            Self::Script(script) => &script.metadata,
            Self::Stdin(metadata) => metadata,
            Self::Remote(metadata, ..) => metadata,
        }
    }

    /// Return the PEP 723 script, if any.
    pub fn as_script(&self) -> Option<&Pep723Script> {
        match self {
            Self::Script(script) => Some(script),
            _ => None,
        }
    }
}

/// A reference to a PEP 723 item.
#[derive(Debug, Copy, Clone)]
pub enum Pep723ItemRef<'item> {
    /// A PEP 723 script read from disk.
    Script(&'item Pep723Script),
    /// A PEP 723 script provided via `stdin`.
    Stdin(&'item Pep723Metadata),
    /// A PEP 723 script provided via a remote URL.
    Remote(&'item Pep723Metadata, &'item Url),
}

impl Pep723ItemRef<'_> {
    /// Return the [`Pep723Metadata`] associated with the item.
    pub fn metadata(&self) -> &Pep723Metadata {
        match self {
            Self::Script(script) => &script.metadata,
            Self::Stdin(metadata) => metadata,
            Self::Remote(metadata, ..) => metadata,
        }
    }

    /// Return the path of the PEP 723 item, if any.
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::Script(script) => Some(&script.path),
            Self::Stdin(..) => None,
            Self::Remote(..) => None,
        }
    }

    /// Determine the working directory for the script.
    pub fn directory(&self) -> Result<PathBuf, io::Error> {
        match self {
            Self::Script(script) => Ok(std::path::absolute(&script.path)?
                .parent()
                .expect("script path has no parent")
                .to_owned()),
            Self::Stdin(..) | Self::Remote(..) => std::env::current_dir(),
        }
    }

    /// Collect any `tool.uv.index` from the script.
    pub fn indexes(&self, source_strategy: &NoSources) -> &[uv_distribution_types::Index] {
        match source_strategy {
            NoSources::None | NoSources::Packages(_) => self
                .metadata()
                .tool
                .as_ref()
                .and_then(|tool| tool.uv.as_ref())
                .and_then(|uv| uv.top_level.index.as_deref())
                .unwrap_or(&[]),
            NoSources::All => &[],
        }
    }

    /// Collect any `tool.uv.sources` from the script.
    pub fn sources(&self, source_strategy: &NoSources) -> Cow<'_, BTreeMap<PackageName, Sources>> {
        static EMPTY: BTreeMap<PackageName, Sources> = BTreeMap::new();
        let sources = self
            .metadata()
            .tool
            .as_ref()
            .and_then(|tool| tool.uv.as_ref())
            .and_then(|uv| uv.sources.as_ref())
            .unwrap_or(&EMPTY);

        match source_strategy {
            NoSources::None => Cow::Borrowed(sources),
            NoSources::All => Cow::Borrowed(&EMPTY),
            NoSources::Packages(packages) => Cow::Owned(
                sources
                    .iter()
                    .filter(|(name, _)| !packages.contains(name))
                    .map(|(name, sources)| (name.clone(), sources.clone()))
                    .collect(),
            ),
        }
    }
}

impl<'item> From<&'item Pep723Item> for Pep723ItemRef<'item> {
    fn from(item: &'item Pep723Item) -> Self {
        match item {
            Pep723Item::Script(script) => Self::Script(script),
            Pep723Item::Stdin(metadata) => Self::Stdin(metadata),
            Pep723Item::Remote(metadata, url) => Self::Remote(metadata, url),
        }
    }
}

impl<'item> From<&'item Pep723Script> for Pep723ItemRef<'item> {
    fn from(script: &'item Pep723Script) -> Self {
        Self::Script(script)
    }
}

/// A PEP 723 script, including its [`Pep723Metadata`].
#[derive(Debug, Clone)]
pub struct Pep723Script {
    /// The path to the Python script.
    pub path: PathBuf,
    /// The parsed [`Pep723Metadata`] table from the script.
    pub metadata: Pep723Metadata,
    /// The content of the script before the metadata table.
    pub prelude: String,
    /// The content of the script after the metadata table.
    pub postlude: String,
}

impl Pep723Script {
    /// Read the PEP 723 `script` metadata from a Python file, if it exists.
    ///
    /// Returns `None` if the file is missing a PEP 723 metadata block.
    ///
    /// See: <https://peps.python.org/pep-0723/>
    pub async fn read(file: impl AsRef<Path>) -> Result<Option<Self>, Pep723Error> {
        let contents = fs_err::tokio::read(&file).await?;

        // Extract the `script` tag.
        let ScriptTag {
            prelude,
            metadata,
            postlude,
        } = match ScriptTag::parse(&contents) {
            Ok(Some(tag)) => tag,
            Ok(None) => return Ok(None),
            Err(err) => return Err(err),
        };

        // Parse the metadata.
        let metadata = Pep723Metadata::from_str(&metadata)?;

        Ok(Some(Self {
            path: std::path::absolute(file)?,
            metadata,
            prelude,
            postlude,
        }))
    }

    /// Reads a Python script and generates a default PEP 723 metadata table.
    ///
    /// See: <https://peps.python.org/pep-0723/>
    pub async fn init(
        file: impl AsRef<Path>,
        requires_python: &VersionSpecifiers,
    ) -> Result<Self, Pep723Error> {
        let contents = fs_err::tokio::read(&file).await?;
        let (prelude, metadata, postlude) = Self::init_metadata(&contents, requires_python)?;
        Ok(Self {
            path: std::path::absolute(file)?,
            metadata,
            prelude,
            postlude,
        })
    }

    /// Generates a default PEP 723 metadata table from the provided script contents.
    ///
    /// See: <https://peps.python.org/pep-0723/>
    fn init_metadata(
        contents: &[u8],
        requires_python: &VersionSpecifiers,
    ) -> Result<(String, Pep723Metadata, String), Pep723Error> {
        // Define the default metadata.
        let default_metadata = if requires_python.is_empty() {
            indoc::formatdoc! {r"
                dependencies = []
            ",
            }
        } else {
            indoc::formatdoc! {r#"
                requires-python = "{requires_python}"
                dependencies = []
                "#,
                requires_python = requires_python,
            }
        };
        let metadata = Pep723Metadata::from_str(&default_metadata)?;

        // Extract the script header and script content.
        let (header, postlude, _) = extract_script_header(contents);
        let header = std::str::from_utf8(header)?;
        let postlude = std::str::from_utf8(postlude)?;

        // Add a newline to the beginning if it starts with a valid metadata comment line.
        let postlude = if postlude.strip_prefix('#').is_some_and(|postlude| {
            postlude
                .chars()
                .next()
                .is_some_and(|c| matches!(c, ' ' | '\r' | '\n'))
        }) {
            format!("\n{postlude}")
        } else {
            postlude.to_string()
        };

        Ok((
            if header.is_empty() {
                String::new()
            } else {
                format!("{header}\n")
            },
            metadata,
            postlude,
        ))
    }

    /// Create a PEP 723 script at the given path.
    pub async fn create(
        file: impl AsRef<Path>,
        requires_python: &VersionSpecifiers,
        existing_contents: Option<Vec<u8>>,
        bare: bool,
    ) -> Result<(), Pep723Error> {
        let file = file.as_ref();

        let script_name = file
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| Pep723Error::InvalidFilename(file.to_string_lossy().to_string()))?;

        let default_metadata = indoc::formatdoc! {r#"
            requires-python = "{requires_python}"
            dependencies = []
            "#,
        };
        let metadata = serialize_metadata(&default_metadata);

        let script = if let Some(existing_contents) = existing_contents {
            let (header, contents, has_shebang) = extract_script_header(&existing_contents);
            let mut script = Vec::with_capacity(header.len() + metadata.len() + contents.len() + 3);
            if !header.is_empty() {
                script.extend_from_slice(header);
                script.push(b'\n');
            }
            if has_shebang {
                script.extend_from_slice(b"#\n");
                // If the shebang doesn't contain `uv`, it's probably something like
                // `#! /usr/bin/env python`, which isn't going to respect the inline metadata.
                // Issue a warning for users who might not know that.
                // TODO: There are a lot of mistakes we could consider detecting here, like
                // `uv run` without `--script` when the file doesn't end in `.py`.
                if !std::str::from_utf8(header)?
                    .lines()
                    .next()
                    .is_some_and(|shebang| regex::regex!(r"\buv\b").is_match(shebang))
                {
                    warn_user!(
                        "If you execute {} directly, it might ignore its inline metadata.\nConsider replacing its shebang with: {}",
                        file.to_string_lossy().cyan(),
                        "#!/usr/bin/env -S uv run --script".cyan(),
                    );
                }
            }
            script.extend_from_slice(metadata.as_bytes());
            script.push(b'\n');
            script.extend_from_slice(contents);
            script
        } else if bare {
            metadata.into_bytes()
        } else {
            indoc::formatdoc! {r#"
            {metadata}

            def main() -> None:
                print("Hello from {name}!")


            if __name__ == "__main__":
                main()
        "#,
                metadata = metadata,
                name = script_name,
            }
            .into_bytes()
        };

        Ok(fs_err::tokio::write(file, script).await?)
    }

    /// Replace the existing metadata in the file with new metadata and write the updated content.
    pub fn write(&self, metadata: &str) -> Result<(), io::Error> {
        let content = format!(
            "{}{}{}",
            self.prelude,
            serialize_metadata(metadata),
            self.postlude
        );

        fs_err::write(&self.path, content)?;

        Ok(())
    }

    /// Return the [`Sources`] defined in the PEP 723 metadata.
    pub fn sources(&self) -> &BTreeMap<PackageName, Sources> {
        static EMPTY: BTreeMap<PackageName, Sources> = BTreeMap::new();

        self.metadata
            .tool
            .as_ref()
            .and_then(|tool| tool.uv.as_ref())
            .and_then(|uv| uv.sources.as_ref())
            .unwrap_or(&EMPTY)
    }
}

/// PEP 723 metadata as parsed from a `script` comment block.
///
/// See: <https://peps.python.org/pep-0723/>
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "kebab-case")]
pub struct Pep723Metadata {
    pub dependencies: Option<Vec<uv_pep508::Requirement<VerbatimParsedUrl>>>,
    pub requires_python: Option<VersionSpecifiers>,
    pub tool: Option<Tool>,
    /// The raw unserialized document.
    #[serde(skip)]
    pub raw: String,
}

impl Pep723Metadata {
    /// Parse the PEP 723 metadata from `stdin`.
    pub fn parse(contents: &[u8]) -> Result<Option<Self>, Pep723Error> {
        // Extract the `script` tag.
        let ScriptTag { metadata, .. } = match ScriptTag::parse(contents) {
            Ok(Some(tag)) => tag,
            Ok(None) => return Ok(None),
            Err(err) => return Err(err),
        };

        // Parse the metadata.
        Ok(Some(Self::from_str(&metadata)?))
    }

    /// Read the PEP 723 `script` metadata from a Python file, if it exists.
    ///
    /// Returns `None` if the file is missing a PEP 723 metadata block.
    ///
    /// See: <https://peps.python.org/pep-0723/>
    pub async fn read(file: impl AsRef<Path>) -> Result<Option<Self>, Pep723Error> {
        let contents = fs_err::tokio::read(&file).await?;

        // Extract the `script` tag.
        let ScriptTag { metadata, .. } = match ScriptTag::parse(&contents) {
            Ok(Some(tag)) => tag,
            Ok(None) => return Ok(None),
            Err(err) => return Err(err),
        };

        // Parse the metadata.
        Ok(Some(Self::from_str(&metadata)?))
    }
}

impl FromStr for Pep723Metadata {
    type Err = toml::de::Error;

    /// Parse `Pep723Metadata` from a raw TOML string.
    #[instrument(name = "toml::from_str PEP 723 metadata", skip_all)]
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let metadata = toml::from_str(raw)?;
        Ok(Self {
            raw: raw.to_string(),
            ..metadata
        })
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
pub struct Tool {
    pub uv: Option<ToolUv>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct ToolUv {
    #[serde(flatten)]
    pub globals: GlobalOptions,
    #[serde(flatten)]
    pub top_level: ResolverInstallerSchema,
    pub override_dependencies: Option<Vec<OverrideDependency>>,
    pub exclude_dependencies: Option<Vec<ExcludeDependency>>,
    pub constraint_dependencies: Option<Vec<uv_pep508::Requirement<VerbatimParsedUrl>>>,
    pub build_constraint_dependencies: Option<Vec<uv_pep508::Requirement<VerbatimParsedUrl>>>,
    pub extra_build_dependencies: Option<BTreeMap<PackageName, Vec<ExtraBuildDependency>>>,
    pub sources: Option<BTreeMap<PackageName, Sources>>,
}

#[derive(Debug, Error)]
pub enum Pep723Error {
    #[error(
        "An opening tag (`# /// script`) was found without a closing tag (`# ///`). Ensure that every line between the opening and closing tags (including empty lines) starts with a leading `#`."
    )]
    UnclosedBlock,
    #[error(
        "An opening tag (`# /// script`) was found, but the closing tag (`# ///`) has trailing content. Remove the trailing content so the line is exactly `# ///`."
    )]
    UnclosedBlockTrailingContent,
    #[error("The script contains multiple PEP 723 metadata blocks")]
    DuplicateBlock,
    #[error("The PEP 723 metadata block is missing from the script.")]
    MissingTag,
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Utf8(#[from] std::str::Utf8Error),
    #[error(transparent)]
    Toml(#[from] toml::de::Error),
    #[error("Invalid filename `{0}` supplied")]
    InvalidFilename(String),
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ScriptTag {
    /// The content of the script before the metadata block.
    prelude: String,
    /// The metadata block.
    metadata: String,
    /// The content of the script after the metadata block.
    postlude: String,
}

impl ScriptTag {
    /// Given the contents of a Python file, extract the `script` metadata block with leading
    /// comment hashes removed, any preceding shebang or content (prelude), and the remaining Python
    /// script.
    ///
    /// Given the following input string representing the contents of a Python script:
    ///
    /// ```python
    /// #!/usr/bin/env python3
    /// # /// script
    /// # requires-python = '>=3.11'
    /// # dependencies = [
    /// #   'requests<3',
    /// #   'rich',
    /// # ]
    /// # ///
    ///
    /// import requests
    ///
    /// print("Hello, World!")
    /// ```
    ///
    /// This function would return:
    ///
    /// - Preamble: `#!/usr/bin/env python3\n`
    /// - Metadata: `requires-python = '>=3.11'\ndependencies = [\n  'requests<3',\n  'rich',\n]`
    /// - Postlude: `import requests\n\nprint("Hello, World!")\n`
    ///
    /// See: <https://peps.python.org/pep-0723/>
    pub fn parse(contents: &[u8]) -> Result<Option<Self>, Pep723Error> {
        // Identify the opening pragma.
        let Some(index) = FINDER.find(contents) else {
            return Ok(None);
        };

        // The opening pragma must be the first line, or immediately preceded by a newline.
        if !(index == 0 || matches!(contents[index - 1], b'\r' | b'\n')) {
            return Ok(None);
        }

        // Extract the preceding content.
        let prelude = std::str::from_utf8(&contents[..index])?;

        // Decode as UTF-8.
        let contents = &contents[index..];
        let contents = std::str::from_utf8(contents)?;

        let mut lines = contents.lines();

        // Ensure that the first line is exactly `# /// script`.
        if lines.next().is_none_or(|line| line != "# /// script") {
            return Ok(None);
        }

        // > Every line between these two lines (# /// TYPE and # ///) MUST be a comment starting
        // > with #. If there are characters after the # then the first character MUST be a space. The
        // > embedded content is formed by taking away the first two characters of each line if the
        // > second character is a space, otherwise just the first character (which means the line
        // > consists of only a single #).
        let mut toml = vec![];

        for line in lines {
            // Remove the leading `#`.
            let Some(line) = line.strip_prefix('#') else {
                break;
            };

            // If the line is empty, continue.
            if line.is_empty() {
                toml.push("");
                continue;
            }

            // Otherwise, the line _must_ start with ` `.
            let Some(line) = line.strip_prefix(' ') else {
                break;
            };

            toml.push(line);
        }

        // Find the closing `# ///`. The precedence is such that we need to identify the _last_ such
        // line.
        //
        // For example, given:
        // ```python
        // # /// script
        // #
        // # ///
        // #
        // # ///
        // ```
        //
        // The latter `///` is the closing pragma. Track malformed terminators while searching, but
        // continue looking for an exact terminator so trailing comments cannot invalidate it.
        let mut has_trailing_content = false;
        let mut closing_index = None;

        for (index, line) in toml.iter().enumerate().rev() {
            if *line == "///" {
                closing_index = Some(index + 1);
                break;
            }

            if line.starts_with("///") {
                has_trailing_content = true;
            }
        }

        let Some(index) = closing_index else {
            return Err(if has_trailing_content {
                Pep723Error::UnclosedBlockTrailingContent
            } else {
                Pep723Error::UnclosedBlock
            });
        };

        // Discard any lines after the closing `# ///`.
        //
        // For example, given:
        // ```python
        // # /// script
        // #
        // # ///
        // #
        // #
        // ```
        //
        // We need to discard the last two lines.
        toml.truncate(index - 1);

        // Extract the remaining content.
        let postlude = contents.lines().skip(index + 1).collect::<Vec<_>>();

        // Ensure that the remaining content doesn't include another complete `script` block.
        // A `# /// script` line can be embedded content inside another typed block.
        let mut lines = postlude.iter().peekable();
        while let Some(line) = lines.next() {
            // Capture the metadata.
            let Some(metadata_type) = line.strip_prefix("# /// ") else {
                continue;
            };

            // Parse the metadata type per spec
            if metadata_type.is_empty()
                || !metadata_type
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            {
                continue;
            }

            let is_script_block = metadata_type == "script";
            let mut is_closed = false;
            while let Some(line) = lines.next() {
                // Per e.g. # dependencies = []
                let Some(content) = line.strip_prefix('#') else {
                    break;
                };
                if !(content.is_empty() || content.starts_with(' ')) {
                    break;
                }

                if *line == "# ///" {
                    let Some(next_line) = lines.peek() else {
                        is_closed = true;
                        break;
                    };

                    let Some(next_content) = next_line.strip_prefix('#') else {
                        is_closed = true;
                        break;
                    };

                    if !(next_content.is_empty() || next_content.starts_with(' ')) {
                        is_closed = true;
                        break;
                    }
                }
            }

            if is_script_block && is_closed {
                return Err(Pep723Error::DuplicateBlock);
            }
        }

        // Join the lines into a single string.
        let prelude = prelude.to_string();
        let metadata = toml.join("\n") + "\n";
        let postlude = postlude.join("\n") + "\n";

        Ok(Some(Self {
            prelude,
            metadata,
            postlude,
        }))
    }
}

/// Extracts the shebang and encoding cookie from the given file contents.
fn extract_script_header(contents: &[u8]) -> (&[u8], &[u8], bool) {
    let (first_line, remaining) = split_first_line(contents);
    let has_shebang = first_line.starts_with(b"#!");

    if is_encoding_cookie(first_line) {
        return (first_line, remaining, has_shebang);
    }

    let first_line_comment = trim_start(first_line);
    if first_line_comment.is_empty() || first_line_comment.starts_with(b"#") {
        let (second_line, script) = split_first_line(remaining);
        if is_encoding_cookie(second_line) {
            let header_length = contents.len() - script.len();
            let header = &contents[..header_length];
            let header = header.strip_suffix(b"\n").unwrap_or(header);
            let header = header.strip_suffix(b"\r").unwrap_or(header);
            return (header, script, has_shebang);
        }
    }

    if has_shebang {
        (first_line, remaining, true)
    } else {
        (b"", contents, false)
    }
}

/// Returns whether a line contains a PEP 263 encoding declaration.
fn is_encoding_cookie(line: &[u8]) -> bool {
    let line = trim_start(line);
    let Some(comment) = line.strip_prefix(b"#") else {
        return false;
    };

    comment
        .windows("coding".len())
        .enumerate()
        .any(|(index, word)| {
            if word != b"coding" {
                return false;
            }

            let suffix = &comment[index + "coding".len()..];
            let Some(delimiter) = suffix.first() else {
                return false;
            };
            if !matches!(delimiter, b':' | b'=') {
                return false;
            }

            suffix[1..]
                .iter()
                .find(|byte| !matches!(**byte, b' ' | b'\t'))
                .is_some_and(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
                })
        })
}

/// Splits the first line from the remaining contents.
fn split_first_line(contents: &[u8]) -> (&[u8], &[u8]) {
    let index = contents
        .iter()
        .position(|byte| matches!(byte, b'\r' | b'\n'))
        .unwrap_or(contents.len());
    let width = match contents.get(index) {
        Some(b'\r') if contents.get(index + 1) == Some(&b'\n') => 2,
        Some(b'\r' | b'\n') => 1,
        _ => 0,
    };

    (&contents[..index], &contents[index + width..])
}

/// Remove leading Python whitespace from a source line.
fn trim_start(line: &[u8]) -> &[u8] {
    let start = line
        .iter()
        .position(|byte| !matches!(byte, b' ' | b'\t' | b'\x0c'))
        .unwrap_or(line.len());
    &line[start..]
}

/// Formats the provided metadata by prefixing each line with `#` and wrapping it with script markers.
fn serialize_metadata(metadata: &str) -> String {
    let mut output = String::with_capacity(metadata.len() + 32);

    output.push_str("# /// script");
    output.push('\n');

    for line in metadata.lines() {
        output.push('#');
        if !line.is_empty() {
            output.push(' ');
            output.push_str(line);
        }
        output.push('\n');
    }

    output.push_str("# ///");
    output.push('\n');

    output
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use crate::{Pep723Error, Pep723Script, ScriptTag, serialize_metadata};
    use std::str::FromStr;

    #[test]
    fn missing_space() {
        let contents = indoc::indoc! {r"
        # /// script
        #requires-python = '>=3.11'
        # ///
    "};

        assert_matches!(
            ScriptTag::parse(contents.as_bytes()),
            Err(Pep723Error::UnclosedBlock)
        );
    }

    #[test]
    fn no_closing_pragma() {
        let contents = indoc::indoc! {r"
        # /// script
        # requires-python = '>=3.11'
        # dependencies = [
        #     'requests<3',
        #     'rich',
        # ]
    "};

        assert_matches!(
            ScriptTag::parse(contents.as_bytes()),
            Err(Pep723Error::UnclosedBlock)
        );
    }

    #[test]
    fn closing_tag_trailing_whitespace() {
        // Explicit string (not `indoc`) so the closing tag's trailing space is preserved.
        let contents = "# /// script\n# requires-python = '>=3.11'\n# /// \n";

        assert_matches!(
            ScriptTag::parse(contents.as_bytes()),
            Err(Pep723Error::UnclosedBlockTrailingContent)
        );
    }

    #[test]
    fn closing_tag_trailing_content() {
        let contents = indoc::indoc! {r"
            # /// script
            # requires-python = '>=3.11'
            # /// unexpected
        "};

        assert_matches!(
            ScriptTag::parse(contents.as_bytes()),
            Err(Pep723Error::UnclosedBlockTrailingContent)
        );
    }

    #[test]
    fn closing_tag_followed_by_prefixed_comment() {
        let contents = indoc::indoc! {r#"
            # /// script
            # dependencies = []
            # ///
            # /// documentation
            print("Hello, world!")
        "#};

        let actual = ScriptTag::parse(contents.as_bytes()).unwrap().unwrap();

        assert_eq!(actual.metadata, "dependencies = []\n");
        assert_eq!(
            actual.postlude,
            "# /// documentation\nprint(\"Hello, world!\")\n"
        );
    }

    #[test]
    fn closing_tag_followed_by_trailing_whitespace_comment() {
        let contents =
            "# /// script\n# dependencies = []\n# ///\n# /// \nprint(\"Hello, world!\")\n";

        let actual = ScriptTag::parse(contents.as_bytes()).unwrap().unwrap();

        assert_eq!(actual.metadata, "dependencies = []\n");
        assert_eq!(actual.postlude, "# /// \nprint(\"Hello, world!\")\n");
    }

    #[test]
    fn leading_content() {
        let contents = indoc::indoc! {r"
        pass # /// script
        # requires-python = '>=3.11'
        # dependencies = [
        #   'requests<3',
        #   'rich',
        # ]
        # ///
        #
        #
    "};

        assert_eq!(ScriptTag::parse(contents.as_bytes()).unwrap(), None);
    }

    #[test]
    fn simple() {
        let contents = indoc::indoc! {r"
        # /// script
        # requires-python = '>=3.11'
        # dependencies = [
        #     'requests<3',
        #     'rich',
        # ]
        # ///

        import requests
        from rich.pretty import pprint

        resp = requests.get('https://peps.python.org/api/peps.json')
        data = resp.json()
    "};

        let expected_metadata = indoc::indoc! {r"
        requires-python = '>=3.11'
        dependencies = [
            'requests<3',
            'rich',
        ]
    "};

        let expected_data = indoc::indoc! {r"

        import requests
        from rich.pretty import pprint

        resp = requests.get('https://peps.python.org/api/peps.json')
        data = resp.json()
    "};

        let actual = ScriptTag::parse(contents.as_bytes()).unwrap().unwrap();

        assert_eq!(actual.prelude, String::new());
        assert_eq!(actual.metadata, expected_metadata);
        assert_eq!(actual.postlude, expected_data);
    }

    #[test]
    fn simple_with_shebang() {
        let contents = indoc::indoc! {r"
        #!/usr/bin/env python3
        # /// script
        # requires-python = '>=3.11'
        # dependencies = [
        #     'requests<3',
        #     'rich',
        # ]
        # ///

        import requests
        from rich.pretty import pprint

        resp = requests.get('https://peps.python.org/api/peps.json')
        data = resp.json()
    "};

        let expected_metadata = indoc::indoc! {r"
        requires-python = '>=3.11'
        dependencies = [
            'requests<3',
            'rich',
        ]
    "};

        let expected_data = indoc::indoc! {r"

        import requests
        from rich.pretty import pprint

        resp = requests.get('https://peps.python.org/api/peps.json')
        data = resp.json()
    "};

        let actual = ScriptTag::parse(contents.as_bytes()).unwrap().unwrap();

        assert_eq!(actual.prelude, "#!/usr/bin/env python3\n".to_string());
        assert_eq!(actual.metadata, expected_metadata);
        assert_eq!(actual.postlude, expected_data);
    }

    #[test]
    fn embedded_comment() {
        let contents = indoc::indoc! {r"
        # /// script
        # embedded-csharp = '''
        # /// <summary>
        # /// text
        # ///
        # /// </summary>
        # public class MyClass { }
        # '''
        # ///
    "};

        let expected = indoc::indoc! {r"
        embedded-csharp = '''
        /// <summary>
        /// text
        ///
        /// </summary>
        public class MyClass { }
        '''
    "};

        let actual = ScriptTag::parse(contents.as_bytes())
            .unwrap()
            .unwrap()
            .metadata;

        assert_eq!(actual, expected);
    }

    #[test]
    fn trailing_lines() {
        let contents = indoc::indoc! {r"
            # /// script
            # requires-python = '>=3.11'
            # dependencies = [
            #     'requests<3',
            #     'rich',
            # ]
            # ///
            #
            #
        "};

        let expected = indoc::indoc! {r"
            requires-python = '>=3.11'
            dependencies = [
                'requests<3',
                'rich',
            ]
        "};

        let actual = ScriptTag::parse(contents.as_bytes())
            .unwrap()
            .unwrap()
            .metadata;

        assert_eq!(actual, expected);
    }

    #[test]
    fn unclosed_second_script_block_is_not_duplicate() {
        let contents = indoc::indoc! {r#"
            # /// script
            # dependencies = ["requests"]
            # ///

            print("Hello, world!")

            # /// script
        "#};

        assert!(ScriptTag::parse(contents.as_bytes()).is_ok());
    }

    #[test]
    fn adjacent_unclosed_second_script_block_is_not_duplicate() {
        let contents = indoc::indoc! {r#"
            # /// script
            # dependencies = []
            # ///
            # /// script
            print("Hello, world!")
        "#};

        let actual = ScriptTag::parse(contents.as_bytes()).unwrap().unwrap();

        assert_eq!(actual.metadata, "dependencies = []\n");
        assert_eq!(actual.postlude, "# /// script\nprint(\"Hello, world!\")\n");
    }

    #[test]
    fn other_script_block_is_ignored() {
        let contents = indoc::indoc! {r#"
            # /// script
            # dependencies = ["requests"]
            # ///


            # /// other
            # /// script
            # ///

            print("Hello, world!")
        "#};

        assert!(ScriptTag::parse(contents.as_bytes()).is_ok());
    }

    #[test]
    fn serialize_metadata_formatting() {
        let metadata = indoc::indoc! {r"
            requires-python = '>=3.11'
            dependencies = [
              'requests<3',
              'rich',
            ]
        "};

        let expected_output = indoc::indoc! {r"
            # /// script
            # requires-python = '>=3.11'
            # dependencies = [
            #   'requests<3',
            #   'rich',
            # ]
            # ///
        "};

        let result = serialize_metadata(metadata);
        assert_eq!(result, expected_output);
    }

    #[test]
    fn serialize_metadata_empty() {
        let metadata = "";
        let expected_output = "# /// script\n# ///\n";

        let result = serialize_metadata(metadata);
        assert_eq!(result, expected_output);
    }

    #[test]
    fn script_init_empty() {
        let contents = "".as_bytes();
        let (prelude, metadata, postlude) =
            Pep723Script::init_metadata(contents, &uv_pep440::VersionSpecifiers::default())
                .unwrap();
        assert_eq!(prelude, "");
        assert_eq!(
            metadata.raw,
            indoc::indoc! {r"
            dependencies = []
            "}
        );
        assert_eq!(postlude, "");
    }

    #[test]
    fn script_init_requires_python() {
        let contents = "".as_bytes();
        let (prelude, metadata, postlude) = Pep723Script::init_metadata(
            contents,
            &uv_pep440::VersionSpecifiers::from_str(">=3.8").unwrap(),
        )
        .unwrap();
        assert_eq!(prelude, "");
        assert_eq!(
            metadata.raw,
            indoc::indoc! {r#"
            requires-python = ">=3.8"
            dependencies = []
            "#}
        );
        assert_eq!(postlude, "");
    }

    #[test]
    fn script_init_with_hashbang() {
        let contents = indoc::indoc! {r#"
        #!/usr/bin/env python3

        print("Hello, world!")
        "#}
        .as_bytes();
        let (prelude, metadata, postlude) =
            Pep723Script::init_metadata(contents, &uv_pep440::VersionSpecifiers::default())
                .unwrap();
        assert_eq!(prelude, "#!/usr/bin/env python3\n");
        assert_eq!(
            metadata.raw,
            indoc::indoc! {r"
            dependencies = []
            "}
        );
        assert_eq!(
            postlude,
            indoc::indoc! {r#"

            print("Hello, world!")
            "#}
        );
    }

    #[test]
    fn script_init_with_other_metadata() {
        let contents = indoc::indoc! {r#"
        # /// noscript
        # Hello,
        #
        # World!
        # ///

        print("Hello, world!")
        "#}
        .as_bytes();
        let (prelude, metadata, postlude) =
            Pep723Script::init_metadata(contents, &uv_pep440::VersionSpecifiers::default())
                .unwrap();
        assert_eq!(prelude, "");
        assert_eq!(
            metadata.raw,
            indoc::indoc! {r"
            dependencies = []
            "}
        );
        // Note the extra line at the beginning.
        assert_eq!(
            postlude,
            indoc::indoc! {r#"

            # /// noscript
            # Hello,
            #
            # World!
            # ///

            print("Hello, world!")
            "#}
        );
    }

    #[test]
    fn script_init_with_hashbang_and_other_metadata() {
        let contents = indoc::indoc! {r#"
        #!/usr/bin/env python3
        # /// noscript
        # Hello,
        #
        # World!
        # ///

        print("Hello, world!")
        "#}
        .as_bytes();
        let (prelude, metadata, postlude) =
            Pep723Script::init_metadata(contents, &uv_pep440::VersionSpecifiers::default())
                .unwrap();
        assert_eq!(prelude, "#!/usr/bin/env python3\n");
        assert_eq!(
            metadata.raw,
            indoc::indoc! {r"
            dependencies = []
            "}
        );
        // Note the extra line at the beginning.
        assert_eq!(
            postlude,
            indoc::indoc! {r#"

            # /// noscript
            # Hello,
            #
            # World!
            # ///

            print("Hello, world!")
            "#}
        );
    }

    #[test]
    fn script_init_with_valid_metadata_line() {
        let contents = indoc::indoc! {r#"
        # Hello,
        # /// noscript
        #
        # World!
        # ///

        print("Hello, world!")
        "#}
        .as_bytes();
        let (prelude, metadata, postlude) =
            Pep723Script::init_metadata(contents, &uv_pep440::VersionSpecifiers::default())
                .unwrap();
        assert_eq!(prelude, "");
        assert_eq!(
            metadata.raw,
            indoc::indoc! {r"
            dependencies = []
            "}
        );
        // Note the extra line at the beginning.
        assert_eq!(
            postlude,
            indoc::indoc! {r#"

            # Hello,
            # /// noscript
            #
            # World!
            # ///

            print("Hello, world!")
            "#}
        );
    }

    #[test]
    fn script_init_with_valid_empty_metadata_line() {
        let contents = indoc::indoc! {r#"
        #
        # /// noscript
        # Hello,
        # World!
        # ///

        print("Hello, world!")
        "#}
        .as_bytes();
        let (prelude, metadata, postlude) =
            Pep723Script::init_metadata(contents, &uv_pep440::VersionSpecifiers::default())
                .unwrap();
        assert_eq!(prelude, "");
        assert_eq!(
            metadata.raw,
            indoc::indoc! {r"
            dependencies = []
            "}
        );
        // Note the extra line at the beginning.
        assert_eq!(
            postlude,
            indoc::indoc! {r#"

            #
            # /// noscript
            # Hello,
            # World!
            # ///

            print("Hello, world!")
            "#}
        );
    }

    #[test]
    fn script_init_with_non_metadata_comment() {
        let contents = indoc::indoc! {r#"
        #Hello,
        # /// noscript
        #
        # World!
        # ///

        print("Hello, world!")
        "#}
        .as_bytes();
        let (prelude, metadata, postlude) =
            Pep723Script::init_metadata(contents, &uv_pep440::VersionSpecifiers::default())
                .unwrap();
        assert_eq!(prelude, "");
        assert_eq!(
            metadata.raw,
            indoc::indoc! {r"
            dependencies = []
            "}
        );
        assert_eq!(
            postlude,
            indoc::indoc! {r#"
            #Hello,
            # /// noscript
            #
            # World!
            # ///

            print("Hello, world!")
            "#}
        );
    }
}
