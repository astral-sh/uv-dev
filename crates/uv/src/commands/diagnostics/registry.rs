use std::error::Error as StdError;
use std::sync::Arc;

use uv_distribution_types::Name;
use uv_errors::{Hinted, Hints, Info};
use uv_resolver::ResolveError;

use crate::commands::build_frontend;
use crate::commands::pip;
use crate::commands::pip::install::ExternallyManagedError;
use crate::commands::pip::operations::ExtrasWithoutSourceError;
use crate::commands::project::ProjectError;
use crate::commands::project::add::AddDependencyError;
use crate::commands::project::remove::DependencyNotFoundError;
use crate::commands::project::run::RecursionLimitError;
use crate::commands::project::version::MissingProjectVersionError;
use crate::commands::python::install::InvalidUpgradeRequestError;
use crate::commands::tool::common::NoExecutablesError;
use crate::commands::tool::run::{ToolRunScriptError, ToolRunUsageError};

use super::{dist_hints, dist_info};

/// Context owned by one concrete error and any root hidden by its transparent presentation.
///
/// Ordinary sources are visited separately by the renderer.
#[derive(Default)]
pub(super) struct ErrorMetadata<'a> {
    pub(super) info: Vec<Info<'a>>,
    pub(super) hints: Hints<'a>,
    pub(super) transparent: Option<&'a (dyn StdError + 'static)>,
    pub(super) is_smart_pointer: bool,
}

impl<'a> ErrorMetadata<'a> {
    fn hinted(error: &'a impl Hinted) -> Self {
        Self {
            info: Vec::new(),
            hints: error.own_hints(),
            transparent: error.transparent_source(),
            is_smart_pointer: false,
        }
    }

    fn with_info(mut self, info: impl IntoIterator<Item = Info<'a>>) -> Self {
        self.info.extend(info);
        self
    }

    fn smart_pointer(error: &'a (dyn StdError + 'static)) -> Self {
        Self {
            info: Vec::new(),
            hints: Hints::none(),
            transparent: Some(error),
            is_smart_pointer: true,
        }
    }
}

/// The central registry of hint-bearing errors, source presentations, and transparent wrappers.
pub(super) fn metadata_for_error<'a>(error: &'a (dyn StdError + 'static)) -> ErrorMetadata<'a> {
    macro_rules! registered {
        ($($error_type:ty => $metadata:expr),+ $(,)?) => {
            $(
                if let Some(metadata) = metadata_for_type::<$error_type>(error, $metadata) {
                    return metadata;
                }
            )+
        };
    }

    registered!(
        ResolveError => metadata_for_resolve_error,
        uv_requirements::Error => metadata_for_requirements_error,
        uv_installer::PrepareError => metadata_for_prepare_error,
        pip::operations::Error => ErrorMetadata::hinted,
        ProjectError => ErrorMetadata::hinted,
        build_frontend::Error => |error| {
            ErrorMetadata::hinted(error).with_info(error.own_info())
        },
        uv_build_frontend::Error => |error| {
            ErrorMetadata::hinted(error).with_info(error.own_info())
        },
        uv_distribution::Error => ErrorMetadata::hinted,
        uv_distribution::MetadataError => ErrorMetadata::hinted,
        uv_dispatch::BuildDispatchError => ErrorMetadata::hinted,
        uv_types::AnyErrorBuild => ErrorMetadata::hinted,
        uv_python::Error => |error| ErrorMetadata::hinted(error).with_info(error.own_info()),
        uv_python::DiscoveryError => ErrorMetadata::hinted,
        uv_python::InterpreterError => ErrorMetadata::hinted,
        uv_python::downloads::Error => ErrorMetadata::hinted,
        uv_python::managed::Error => ErrorMetadata::hinted,
        uv_tool::Error => ErrorMetadata::hinted,
        uv_audit::osv::Error => ErrorMetadata::hinted,
        AddDependencyError => |error| ErrorMetadata::hinted(error).with_info(error.own_info()),
        ToolRunUsageError => |error| {
            ErrorMetadata::hinted(error).with_info([error.own_info()])
        },
        uv_resolver::NoSolutionError => ErrorMetadata::hinted,
        uv_resolver::LockError => ErrorMetadata::hinted,
        ToolRunScriptError => |error| ErrorMetadata::hinted(error).with_info(error.own_info()),
        RecursionLimitError => ErrorMetadata::hinted,
        DependencyNotFoundError => |error| {
            ErrorMetadata::hinted(error).with_info(error.own_info())
        },
        ExtrasWithoutSourceError => ErrorMetadata::hinted,
        NoExecutablesError => |error| ErrorMetadata::hinted(error).with_info(error.own_info()),
        ExternallyManagedError => |error| {
            ErrorMetadata::hinted(error).with_info(error.own_info())
        },
        MissingProjectVersionError => ErrorMetadata::hinted,
        InvalidUpgradeRequestError => |error| {
            ErrorMetadata::hinted(error).with_info(error.own_info())
        },
        uv_build_backend::Error => ErrorMetadata::hinted,
        uv_globfilter::PortableGlobError => ErrorMetadata::hinted,
        uv_installer::IncompatibleWheelError => |error| {
            ErrorMetadata::hinted(error).with_info(error.own_info())
        },
        uv_python::BrokenLink => ErrorMetadata::hinted,
        uv_resolver::PylockTomlError => ErrorMetadata::hinted,
        uv_resolver::PylockTomlErrorKind => ErrorMetadata::hinted,
        uv_requirements_txt::MakeEditableError => ErrorMetadata::hinted,
        uv_workspace::pyproject::SourceError => ErrorMetadata::hinted,
        uv_distribution::LoweringError => ErrorMetadata::hinted,
        uv_virtualenv::Error => ErrorMetadata::hinted,
        uv_client::Error => ErrorMetadata::hinted,
        uv_publish::PublishSendError => |_| ErrorMetadata::default(),
        uv_scripts::Pep723Error => |_| ErrorMetadata::default(),
        uv_settings::Error => |_| ErrorMetadata::default(),
        uv_workspace::pyproject::PyprojectTomlError => |_| ErrorMetadata::default(),
        uv_requirements_txt::RequirementsTxtFileError => |_| ErrorMetadata::default(),
        uv_workspace::WorkspaceError => |_| ErrorMetadata::default(),
        uv_workspace::dependency_groups::DependencyGroupError => |_| ErrorMetadata::default(),
    );
    #[cfg(not(feature = "self-update"))]
    registered!(crate::ExternallyInstalledError => |error| {
        ErrorMetadata::hinted(error).with_info(error.own_info())
    });

    ErrorMetadata::default()
}

/// Smart pointers delegate display and source to their inner error. Expose that root to every
/// native presentation provider, including when the actual source node is the smart pointer.
fn metadata_for_type<'a, T: StdError + 'static>(
    error: &'a (dyn StdError + 'static),
    metadata: impl FnOnce(&'a T) -> ErrorMetadata<'a>,
) -> Option<ErrorMetadata<'a>> {
    if let Some(error) = error.downcast_ref::<T>() {
        Some(metadata(error))
    } else if let Some(error) = error.downcast_ref::<Box<T>>() {
        Some(ErrorMetadata::smart_pointer(error.as_ref()))
    } else {
        error
            .downcast_ref::<Arc<T>>()
            .map(|error| ErrorMetadata::smart_pointer(error.as_ref()))
    }
}

fn metadata_for_resolve_error(error: &ResolveError) -> ErrorMetadata<'_> {
    let mut metadata = ErrorMetadata::hinted(error);
    if let ResolveError::Dependencies(_, name, version, chain) = error {
        metadata.info.extend(dist_info(name, Some(version), chain));
        metadata.hints.extend(dist_hints(name, Hints::none()));
    } else if let ResolveError::Dist(_, dist, chain, _) = error {
        metadata
            .info
            .extend(dist_info(dist.name(), dist.version(), chain));
        metadata
            .hints
            .extend(dist_hints(dist.name(), Hints::none()));
    }
    metadata
}

fn metadata_for_requirements_error(error: &uv_requirements::Error) -> ErrorMetadata<'_> {
    let mut metadata = ErrorMetadata::hinted(error);
    if let uv_requirements::Error::Dist(_, dist, _) = error {
        metadata
            .hints
            .extend(dist_hints(dist.name(), Hints::none()));
    }
    metadata
}

fn metadata_for_prepare_error(error: &uv_installer::PrepareError) -> ErrorMetadata<'_> {
    let mut metadata = ErrorMetadata::default();
    if let uv_installer::PrepareError::Dist(_, dist, chain, _) = error {
        metadata
            .info
            .extend(dist_info(dist.name(), dist.version(), chain));
        metadata
            .hints
            .extend(dist_hints(dist.name(), Hints::none()));
    }
    metadata
}
