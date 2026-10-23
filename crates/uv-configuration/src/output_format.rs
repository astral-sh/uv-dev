/// The output format for listing Python installations.
#[derive(Debug, Default, Clone, Copy)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum PythonListFormat {
    /// Plain text (for humans).
    #[default]
    Text,
    /// JSON (for computers).
    Json,
    /// Newline-delimited JSON, including progress updates.
    Jsonl,
}

/// The output format for audit findings.
#[derive(Debug, Default, Clone, Copy)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum AuditOutputFormat {
    /// Display the result in a human-readable format.
    #[default]
    Text,
    /// Display the result in JSON format.
    Json,
    /// Stream progress updates and the result as newline-delimited JSON.
    Jsonl,
    /// Display the result in SARIF format.
    Sarif,
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum VersionFormat {
    /// Display the version as plain text.
    Text,
    /// Display the version as JSON.
    Json,
    /// Stream progress updates and the version as newline-delimited JSON.
    Jsonl,
}

#[derive(Debug, Default, Clone, Copy)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum SyncFormat {
    /// Display the result in a human-readable format.
    #[default]
    Text,
    /// Display the result in JSON format.
    Json,
    /// Stream progress updates and the result as newline-delimited JSON.
    Jsonl,
}

#[derive(Debug, Default, Clone, Copy)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum MetadataOutputFormat {
    /// Display workspace metadata as JSON.
    #[default]
    Json,
    /// Stream progress updates and workspace metadata as newline-delimited JSON.
    Jsonl,
}

#[derive(Debug, Default, Clone, Copy)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum PipInstallFormat {
    /// Display the result in a human-readable format.
    #[default]
    Text,
    /// Display the result in JSON format.
    Json,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum TreeFormat {
    /// Display the dependency graph as a human-readable tree.
    #[default]
    Text,
    /// Display the dependency graph as JSON.
    Json,
}

#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum ListFormat {
    /// Display the list of packages in a human-readable table.
    #[default]
    Columns,
    /// Display the list of packages in a `pip freeze`-like format, with one package per line
    /// alongside its version.
    Freeze,
    /// Display the list of packages in a machine-readable JSON format.
    Json,
}
