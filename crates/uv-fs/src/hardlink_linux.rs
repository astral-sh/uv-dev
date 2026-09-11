#[cfg(any(
    target_arch = "x86_64",
    target_arch = "aarch64",
    target_arch = "riscv64",
    target_arch = "loongarch64",
    target_arch = "powerpc64"
))]
mod uring;

#[cfg(any(
    target_arch = "x86_64",
    target_arch = "aarch64",
    target_arch = "riscv64",
    target_arch = "loongarch64",
    target_arch = "powerpc64"
))]
pub(super) use uring::Scanner;

// The io-uring crate does not provide prebuilt bindings for every uv target. Avoid requiring
// bindgen and a host libclang just to enable this optional fast path.
#[cfg(not(any(
    target_arch = "x86_64",
    target_arch = "aarch64",
    target_arch = "riscv64",
    target_arch = "loongarch64",
    target_arch = "powerpc64"
)))]
mod fallback {
    use std::io;
    use std::num::NonZeroU32;
    use std::path::{Path, PathBuf};

    pub(crate) struct Scanner;

    impl Scanner {
        pub(crate) fn new(_queue_depth: NonZeroU32) -> Self {
            Self
        }

        #[expect(clippy::unused_self, clippy::unnecessary_wraps)]
        pub(crate) fn files_with_one_hardlink(
            &mut self,
            _path: &Path,
        ) -> io::Result<Option<Vec<PathBuf>>> {
            Ok(None)
        }
    }
}

#[cfg(not(any(
    target_arch = "x86_64",
    target_arch = "aarch64",
    target_arch = "riscv64",
    target_arch = "loongarch64",
    target_arch = "powerpc64"
)))]
pub(super) use fallback::Scanner;
