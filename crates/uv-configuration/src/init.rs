/// The source to use for project author information.
#[derive(Debug, Default, Copy, Clone)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum AuthorFrom {
    /// Fetch the author information from some sources (e.g., Git) automatically.
    #[default]
    Auto,
    /// Fetch the author information from Git configuration only.
    Git,
    /// Do not infer the author information.
    None,
}

/// The kind of entity to initialize (either a PEP 723 script or a Python project).
#[derive(Debug, Copy, Clone)]
pub enum InitKind {
    /// Initialize a Python project.
    Project(InitProjectKind),
    /// Initialize a PEP 723 script.
    Script,
}

/// Whether to initialize a bare or full project.
#[derive(Debug, Copy, Clone)]
pub enum InitMode {
    /// Initialize only the required project files.
    Bare,
    /// Initialize the full project scaffold.
    Full,
}

impl InitMode {
    /// Determine the [`InitMode`] setting based on the command-line arguments.
    pub fn from_args(bare: bool) -> Self {
        if bare { Self::Bare } else { Self::Full }
    }
}

/// The description to include in a newly initialized project.
#[derive(Debug, Clone)]
pub enum InitDescription {
    /// Include the default project description.
    Default,
    /// Include a user-provided project description.
    Custom(String),
    /// Omit the project description.
    None,
}

impl InitDescription {
    /// Determine the [`InitDescription`] setting based on the command-line arguments.
    pub fn from_args(description: Option<String>, no_description: bool) -> Self {
        if no_description {
            Self::None
        } else if let Some(description) = description {
            Self::Custom(description)
        } else {
            Self::Default
        }
    }
}

/// Whether to include a README in a newly initialized project.
#[derive(Debug, Copy, Clone)]
pub enum InitReadme {
    /// Include a README.
    Include,
    /// Omit the README.
    Omit,
}

impl InitReadme {
    /// Determine the [`InitReadme`] setting based on the command-line arguments.
    pub fn from_args(no_readme: bool) -> Self {
        if no_readme { Self::Omit } else { Self::Include }
    }

    /// Omit the README for bare projects.
    #[must_use]
    pub fn for_mode(self, mode: InitMode) -> Self {
        match mode {
            InitMode::Bare => Self::Omit,
            InitMode::Full => self,
        }
    }
}

/// Whether to pin the selected Python version in a newly initialized project.
#[derive(Debug, Copy, Clone)]
pub enum InitPythonPin {
    /// Pin the selected Python version.
    Pin,
    /// Do not pin the selected Python version.
    DoNotPin,
}

impl InitPythonPin {
    /// Determine the [`InitPythonPin`] setting based on the command-line arguments.
    pub fn from_args(pin_python: bool) -> Self {
        if pin_python {
            Self::Pin
        } else {
            Self::DoNotPin
        }
    }
}

/// Whether to discover a parent workspace while initializing a project.
#[derive(Debug, Copy, Clone)]
pub enum InitWorkspaceDiscovery {
    /// Discover a parent workspace.
    Discover,
    /// Ignore any parent workspace.
    Ignore,
}

impl InitWorkspaceDiscovery {
    /// Determine the [`InitWorkspaceDiscovery`] setting based on the command-line arguments.
    pub fn from_args(no_workspace: bool) -> Self {
        if no_workspace {
            Self::Ignore
        } else {
            Self::Discover
        }
    }
}

/// The kind of Python project to initialize (either an application or a library).
#[derive(Debug, Copy, Clone, Default)]
pub enum InitProjectKind {
    /// A python package with a `main` function in a `__init__.py` and a script entrypoint pointing
    /// to that.
    #[default]
    ApplicationWithLibrary,
    /// A flat application with a `main.py`.
    Application,
    /// A python package, no entrypoint.
    Library,
    /// Initialize only a `pyproject.toml`
    Bare,
    /// Initialize only a `pyproject.toml` with `[build-system]` table (but without associated
    /// source files).
    BareWithBuildSystem,
}
