//! Helper for adjusting Unix resource limits.
//!
//! Linux has a historically low default limit of 1024 open file descriptors per process.
//! macOS also defaults to a low soft limit (typically 256), though its hard limit is much
//! higher. On modern multi-core machines, these low defaults can cause "too many open files"
//! errors because uv infers concurrency limits from CPU count and may schedule more concurrent
//! work than the default file descriptor limit allows.
//!
//! This module attempts to raise the soft limit to the hard limit at startup to avoid these
//! errors without requiring users to manually configure their shell's `ulimit` settings.
//! The raised limit is inherited by child processes, which is important for commands like
//! `uv run` that spawn Python interpreters.
//!
//! See: <https://github.com/astral-sh/uv/issues/16999>

use nix::errno::Errno;
use nix::sys::resource::{RLIM_INFINITY, Resource, getrlimit, rlim_t, setrlimit};
use thiserror::Error;
use uv_static::EnvVars;

/// The resource limits supported by the current Unix platform.
pub const SUPPORTED_RESOURCE_LIMITS: &[(&str, RunResource)] = &[
    #[cfg(not(any(target_os = "freebsd", target_os = "netbsd", target_os = "openbsd")))]
    (
        EnvVars::UV_RUN_RLIMIT_AS,
        RunResource::Nix(Resource::RLIMIT_AS),
    ),
    #[cfg(target_os = "freebsd")]
    (
        EnvVars::UV_RUN_RLIMIT_AS,
        RunResource::Nix(Resource::RLIMIT_VMEM),
    ),
    (
        EnvVars::UV_RUN_RLIMIT_CORE,
        RunResource::Nix(Resource::RLIMIT_CORE),
    ),
    (
        EnvVars::UV_RUN_RLIMIT_CPU,
        RunResource::Nix(Resource::RLIMIT_CPU),
    ),
    (
        EnvVars::UV_RUN_RLIMIT_FSIZE,
        RunResource::Nix(Resource::RLIMIT_FSIZE),
    ),
    (
        EnvVars::UV_RUN_RLIMIT_NOFILE,
        RunResource::Nix(Resource::RLIMIT_NOFILE),
    ),
    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "aix"
    ))]
    (
        EnvVars::UV_RUN_RLIMIT_NPROC,
        RunResource::Nix(Resource::RLIMIT_NPROC),
    ),
    #[cfg(target_vendor = "apple")]
    (
        EnvVars::UV_RUN_RLIMIT_NPROC,
        RunResource::Apple(rustix::process::Resource::Nproc),
    ),
];

/// A platform-specific resource supported by the available safe Unix wrappers.
#[derive(Debug, Clone, Copy)]
pub enum RunResource {
    Nix(Resource),
    #[cfg(target_vendor = "apple")]
    Apple(rustix::process::Resource),
}

/// A soft resource limit to apply while preserving the corresponding hard limit.
#[derive(Debug, Clone, Copy)]
pub struct ResourceLimit {
    environment_variable: &'static str,
    resource: RunResource,
    value: u64,
}

impl ResourceLimit {
    /// Create a soft resource limit from its environment variable and parsed value.
    pub fn new(environment_variable: &'static str, resource: RunResource, value: u64) -> Self {
        Self {
            environment_variable,
            resource,
            value,
        }
    }

    /// Return the environment variable that configured this resource limit.
    pub fn environment_variable(self) -> &'static str {
        self.environment_variable
    }

    /// Return the configured soft resource limit.
    pub fn value(self) -> u64 {
        self.value
    }

    /// Apply the configured soft limit without changing the hard limit.
    pub fn apply(self) -> Result<(), ResourceLimitError> {
        let resource_name = self
            .environment_variable
            .strip_prefix("UV_RUN_")
            .unwrap_or(self.environment_variable);

        match self.resource {
            RunResource::Nix(resource) => {
                let (_, hard) =
                    getrlimit(resource).map_err(|source| ResourceLimitError::GetLimitFailed {
                        resource: resource_name,
                        source: source.into(),
                    })?;

                let Some(target) = u64_to_rlim_t(self.value) else {
                    return Err(ResourceLimitError::InvalidLimit {
                        resource: resource_name,
                        value: self.value,
                    });
                };

                if hard != RLIM_INFINITY && target > hard {
                    return Err(ResourceLimitError::ExceedsHardLimit {
                        resource: resource_name,
                        target: self.value,
                        hard: rlim_t_to_u64(hard).unwrap_or(u64::MAX),
                    });
                }

                setrlimit(resource, target, hard).map_err(|source| {
                    ResourceLimitError::SetLimitFailed {
                        resource: resource_name,
                        target: self.value,
                        source: source.into(),
                    }
                })?;
            }
            #[cfg(target_vendor = "apple")]
            RunResource::Apple(resource) => {
                let limit = rustix::process::getrlimit(resource);
                if let Some(hard) = limit.maximum
                    && self.value > hard
                {
                    return Err(ResourceLimitError::ExceedsHardLimit {
                        resource: resource_name,
                        target: self.value,
                        hard,
                    });
                }

                rustix::process::setrlimit(
                    resource,
                    rustix::process::Rlimit {
                        current: Some(self.value),
                        maximum: limit.maximum,
                    },
                )
                .map_err(|source| ResourceLimitError::SetLimitFailed {
                    resource: resource_name,
                    target: self.value,
                    source: source.into(),
                })?;
            }
        }

        Ok(())
    }
}

/// Errors that can occur when applying a configured resource limit.
#[derive(Debug, Error)]
pub enum ResourceLimitError {
    #[error("failed to get {resource} limit: {source}")]
    GetLimitFailed {
        resource: &'static str,
        source: std::io::Error,
    },

    #[error("requested {resource} limit ({value}) is not supported on this platform")]
    InvalidLimit { resource: &'static str, value: u64 },

    #[error("requested {resource} limit ({target}) exceeds the hard limit ({hard})")]
    ExceedsHardLimit {
        resource: &'static str,
        target: u64,
        hard: u64,
    },

    #[error("failed to set {resource} limit to {target}: {source}")]
    SetLimitFailed {
        resource: &'static str,
        target: u64,
        source: std::io::Error,
    },
}

/// Errors that can occur when adjusting resource limits.
#[derive(Debug, Error)]
pub enum OpenFileLimitError {
    #[error("failed to get open file limit: {}", .0.desc())]
    GetLimitFailed(Errno),

    #[error("encountered unexpected negative soft limit: {value}")]
    NegativeSoftLimit { value: rlim_t },

    #[error("soft limit ({current}) already meets the target ({target})")]
    AlreadySufficient { current: u64, target: u64 },

    #[error("requested open file limit ({target}) exceeds the hard limit ({hard})")]
    ExceedsHardLimit { target: u64, hard: rlim_t },

    #[error("failed to set open file limit from {current} to {target}: {}", source.desc())]
    SetLimitFailed {
        current: u64,
        target: u64,
        source: Errno,
    },
}

/// Maximum file descriptor limit to request.
///
/// We cap at 0x100000 (1,048,576) to match the typical Linux default (`/proc/sys/fs/nr_open`)
/// and to avoid issues with extremely high limits.
///
/// `OpenJDK` uses this same cap because:
///
/// 1. Some code breaks if `RLIMIT_NOFILE` exceeds `i32::MAX` (despite the type being `u64`)
/// 2. Code that iterates over all possible FDs, e.g., to close them, can timeout
///
/// See: <https://bugs.openjdk.org/browse/JDK-8324577>
/// See: <https://github.com/oracle/graal/issues/11136>
///
/// Note: `rlim_t` is platform-specific (`u64` on Linux/macOS, `i64` on FreeBSD).
const MAX_NOFILE_LIMIT: rlim_t = 0x0010_0000;

/// Attempt to raise the open file descriptor limit to the maximum allowed.
///
/// This function tries to set the soft limit to `min(hard_limit, 0x100000)`. If the operation
/// fails, it returns an error since the default limits may still be sufficient for the
/// current workload.
///
/// Returns [`Ok`] with the new soft limit on successful adjustment, or an appropriate
/// [`OpenFileLimitError`] if adjustment failed.
///
/// Note the type of `rlim_t` is platform-specific (`u64` on Linux/macOS, `i64` on FreeBSD), but
/// this function always returns a [`u64`].
pub fn adjust_open_file_limit() -> Result<u64, OpenFileLimitError> {
    let (soft, hard) =
        getrlimit(Resource::RLIMIT_NOFILE).map_err(OpenFileLimitError::GetLimitFailed)?;

    // Convert `rlim_t` to `u64`. On FreeBSD, `rlim_t` is `i64` which may fail.
    // On Linux and macOS, `rlim_t` is a `u64`, and the conversion is infallible.
    let Some(soft) = rlim_t_to_u64(soft) else {
        return Err(OpenFileLimitError::NegativeSoftLimit { value: soft });
    };

    // Cap the target limit to avoid issues with extremely high values.
    // If hard is negative or exceeds MAX_NOFILE_LIMIT, use MAX_NOFILE_LIMIT.
    #[expect(clippy::unnecessary_cast)]
    let target = rlim_t_to_u64(hard.min(MAX_NOFILE_LIMIT)).unwrap_or(MAX_NOFILE_LIMIT as u64);

    if soft >= target {
        return Err(OpenFileLimitError::AlreadySufficient {
            current: soft,
            target,
        });
    }

    // Try to raise the soft limit to the target.
    // Safe because target <= MAX_NOFILE_LIMIT which fits in both i64 and u64.
    let target_rlim = target as rlim_t;

    set_open_file_limit_to(soft, target, target_rlim, hard)
}

/// Update the soft open-file descriptor limit while preserving the hard limit.
fn set_open_file_limit_to(
    current: u64,
    target: u64,
    target_rlim: rlim_t,
    hard: rlim_t,
) -> Result<u64, OpenFileLimitError> {
    setrlimit(Resource::RLIMIT_NOFILE, target_rlim, hard).map_err(|err| {
        OpenFileLimitError::SetLimitFailed {
            current,
            target,
            source: err,
        }
    })?;

    Ok(target)
}

/// Convert `rlim_t` to `u64`, returning `None` if negative.
///
/// On Linux/macOS, `rlim_t` is `u64` so this always succeeds.
/// On FreeBSD, `rlim_t` is `i64` so negative values return `None`.
#[expect(clippy::useless_conversion)]
fn rlim_t_to_u64(value: rlim_t) -> Option<u64> {
    u64::try_from(value).ok()
}

/// Convert `u64` to `rlim_t`, returning `None` if it exceeds a signed platform limit.
fn u64_to_rlim_t(value: u64) -> Option<rlim_t> {
    rlim_t::try_from(value).ok()
}
