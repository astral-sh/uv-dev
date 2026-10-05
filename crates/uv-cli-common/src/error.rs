/// A command error propagated to the entrypoint for exit-status selection.
#[derive(Debug, thiserror::Error)]
pub enum UvError {
    /// An error caused by invalid or unsatisfiable user input.
    #[error(transparent)]
    User(anyhow::Error),

    /// An error caused by invalid command-line arguments.
    #[error(transparent)]
    Argument(anyhow::Error),

    /// An unexpected internal or environmental error.
    #[error(transparent)]
    Unexpected(anyhow::Error),
}

impl UvError {
    /// Create a user-facing error.
    pub fn user(error: impl Into<anyhow::Error>) -> Self {
        Self::User(error.into())
    }

    /// Create an argument error.
    pub fn argument(error: anyhow::Error) -> Self {
        Self::Argument(error)
    }

    /// Create an unexpected error.
    pub fn unexpected(error: anyhow::Error) -> Self {
        Self::Unexpected(error)
    }

    /// Add command-specific context to a user error without changing unexpected errors.
    #[must_use]
    pub fn map_user(self, context: impl FnOnce(anyhow::Error) -> anyhow::Error) -> Self {
        match self {
            Self::User(error) => Self::User(context(error)),
            Self::Argument(error) => Self::Argument(error),
            Self::Unexpected(error) => Self::Unexpected(error),
        }
    }
}
