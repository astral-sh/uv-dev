//! Project-specific CLI functionality for uv.

pub mod environment;
pub mod error;
pub mod settings;
pub mod sync;

pub use error::ProjectError;
pub use sync::{
    EnvironmentSpecification, EnvironmentUpdate, resolve_environment, resolve_names,
    sync_environment, update_environment,
};
