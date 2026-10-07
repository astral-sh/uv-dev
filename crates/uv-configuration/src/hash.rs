use uv_distribution_types::Sourced;

#[derive(Debug, Default, Copy, Clone)]
pub enum HashCheckingMode {
    /// Hashes should be validated against a pre-defined list of hashes. Every requirement must
    /// itself be hashable (e.g., Git dependencies are forbidden) _and_ have a hash in the lockfile.
    Require,
    /// Hashes should be validated, if present, but ignored if absent.
    #[default]
    Verify,
}

impl HashCheckingMode {
    /// Return the [`HashCheckingMode`] from the command-line arguments, if any.
    ///
    /// By default, the hash checking mode is [`HashCheckingMode::Verify`]. If `--require-hashes` is
    /// passed, the hash checking mode is [`HashCheckingMode::Require`]. If `--no-verify-hashes` is
    /// passed, then no hash checking is performed.
    pub fn from_args(require_hashes: Option<bool>, verify_hashes: Option<bool>) -> Option<Self> {
        if require_hashes == Some(true) {
            // Given `--require-hashes`, always require hashes, regardless of any other flags.
            Some(Self::Require)
        } else if verify_hashes == Some(false) {
            // Given `--no-verify-hashes` (without `--require-hashes`), do not verify hashes.
            None
        } else {
            // By default, verify hashes, even if `--no-require-hashes` was provided.
            Some(Self::Verify)
        }
    }

    /// Resolve hash checking while retaining the source of the effective policy.
    pub fn from_settings(
        require_hashes: Option<Sourced<bool>>,
        verify_hashes: Option<Sourced<bool>>,
    ) -> Option<Sourced<Self>> {
        let mode = Self::from_args(
            require_hashes.as_ref().map(|setting| *setting.value()),
            verify_hashes.as_ref().map(|setting| *setting.value()),
        )?;
        match mode {
            Self::Require => require_hashes.map(|setting| setting.map(|_| mode)),
            Self::Verify => {
                Some(verify_hashes.map_or_else(|| mode.into(), |setting| setting.map(|_| mode)))
            }
        }
    }

    /// Apply cumulative requirements-file directives to the selected hash-checking policy.
    pub fn from_requirements_txt(
        mode: Option<Sourced<Self>>,
        require_hashes: Sourced<bool>,
    ) -> Option<Sourced<Self>> {
        if *require_hashes.value() {
            let required = require_hashes.map(|_| Self::Require);
            Some(
                if let Some(mode) = mode.filter(|mode| mode.value().is_require()) {
                    mode.with_sources(required.sources())
                } else {
                    required
                },
            )
        } else {
            mode
        }
    }

    /// Returns `true` if the hash checking mode is `Require`.
    pub fn is_require(&self) -> bool {
        matches!(self, Self::Require)
    }
}

impl std::fmt::Display for HashCheckingMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Require => write!(f, "--require-hashes"),
            Self::Verify => write!(f, "--verify-hashes"),
        }
    }
}
