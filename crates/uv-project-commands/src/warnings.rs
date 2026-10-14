use uv_configuration::{DependencyGroups, ExtrasSpecification};
use uv_settings::{FrozenSource, LockCheck};
use uv_warnings::warn_user;

/// Warn about project settings that cannot apply to the command.
pub(super) fn warn_ignored_project_settings(
    no_project: bool,
    extras: &ExtrasSpecification,
    groups: &DependencyGroups,
    lock_check: LockCheck,
    frozen: Option<FrozenSource>,
    no_sync: bool,
) {
    let context = if no_project {
        "alongside `--no-project`"
    } else {
        "outside of a project"
    };
    for flag in extras.history().as_flags_pretty() {
        warn_user!("`{flag}` has no effect when used {context}");
    }
    for flag in groups.history().as_flags_pretty() {
        warn_user!("`{flag}` has no effect when used {context}");
    }
    if let LockCheck::Enabled(lock_check) = lock_check {
        warn_user!("`{lock_check}` has no effect when used {context}");
    }
    if frozen.is_some() {
        warn_user!("`--frozen` has no effect when used {context}");
    }
    if no_sync {
        warn_user!("`--no-sync` has no effect when used {context}");
    }
}
