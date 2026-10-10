//! Lockfile discovery, validation, and resolution for project and workspace workflows.

mod build_context;
mod discovery;
mod error;
mod lock;
mod lock_target;
mod lockfile;
mod validated_lock;

pub use discovery::DiscoveredProject;
pub use error::{LockError, LockValidationError, MissingLockfileSource};
pub use lock::{LockMode, LockOperation, LockResult, workspace_groups_with_cached_metadata};
pub use lock_target::LockTarget;
pub use lockfile::FrozenWorkspace;
pub use validated_lock::ValidatedLock;
