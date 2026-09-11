use std::sync::OnceLock;

static FLAGS: OnceLock<EnvironmentFlags> = OnceLock::new();

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct EnvironmentFlags: u32 {
        const SKIP_WHEEL_FILENAME_CHECK = 1 << 0;
        const HIDE_BUILD_OUTPUT = 1 << 1;
    }
}

/// Initialize the environment flags.
#[expect(clippy::result_unit_err)]
pub fn init(flags: EnvironmentFlags) -> Result<(), ()> {
    FLAGS.set(flags).map_err(|_| ())
}

/// Check if a specific environment flag is set.
pub fn contains(flag: EnvironmentFlags) -> bool {
    FLAGS.get_or_init(EnvironmentFlags::default).contains(flag)
}

/// Read a flag without finalizing the process-global configuration.
pub fn contains_or_default(flag: EnvironmentFlags) -> bool {
    FLAGS.get().copied().unwrap_or_default().contains(flag)
}
