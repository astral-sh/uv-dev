//! Lockfile discovery, validation, and resolution for project and workspace workflows.

mod discovery;
mod error;
mod lock;
mod lock_target;
mod lockfile;
mod reporter;
mod validated_lock;

pub use discovery::DiscoveredProject;
pub use error::{LockError, LockValidationError, MissingLockfileSource};
pub use lock::{LockMode, LockOperation, LockResult};
pub use lock_target::LockTarget;
pub use lockfile::FrozenWorkspace;
pub use reporter::{LockReporter, LockValidationReason, LockValidationReasonCode};
pub use validated_lock::ValidatedLock;
