//! Shared project, script, and tool environment workflows.

mod conflicts;
pub mod environment;
mod error;
pub mod install_target;
pub mod malware;
mod project;
mod requirements;
mod script;
mod sync;

pub use conflicts::{ConflictError, detect_conflicts};
pub use error::EnvironmentError;
pub use project::{
    LinkErrorReporting, ProjectEnvironment, ProjectEnvironmentPolicy, ProjectEnvironmentTarget,
    ProjectInterpreter, centralized_environment_root, centralized_environments_enabled,
    is_centralized_environment_reference, lock_project_environment,
    update_project_environment_link,
};
pub use requirements::{
    EnvironmentResolution, EnvironmentSpecification, EnvironmentUpdate, PreferenceLocation,
    resolve_environment, sync_environment, update_environment,
};
pub use script::ScriptEnvironment;
pub use sync::{store_credentials_from_target, sync_from_lock};
