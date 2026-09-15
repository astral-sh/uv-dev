pub use abi_tag::{AbiTag, CPythonAbiVariants};
pub use language_tag::LanguageTag;
pub use platform::{Arch, Os, Platform, PlatformError};
pub use platform_tag::{ParseReleaseArchError, PlatformTag, ReleaseArch};
pub use tags::{
    BinaryFormat, IncompatibleTag, TagCompatibility, TagPriority, Tags, TagsError, TagsOptions,
};

mod abi_tag;
mod language_tag;
mod platform;
mod platform_tag;
mod tags;
