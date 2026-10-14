use tracing::debug;

use uv_pep508::MarkerEnvironment;
use uv_platform_tags::{Arch, Os, Platform};
use uv_static::EnvVars;

/// The supported target triples. Each triple consists of an architecture, vendor, and operating
/// system.
///
/// See: <https://doc.rust-lang.org/nightly/rustc/platform-support.html>
#[derive(Debug, Clone, Copy, Eq, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum TargetTriple {
    /// An alias for `x86_64-pc-windows-msvc`, the default target for Windows.
    Windows,

    /// An alias for `x86_64-unknown-linux-gnu`, the default target for Linux.
    Linux,

    /// An alias for `aarch64-apple-darwin`, the default target for macOS.
    Macos,

    /// A 64-bit x86 Windows target.
    #[cfg_attr(feature = "clap", value(name = "x86_64-pc-windows-msvc"))]
    #[serde(rename = "x86_64-pc-windows-msvc")]
    #[serde(alias = "x8664-pc-windows-msvc")]
    X8664PcWindowsMsvc,

    /// An ARM64 Windows target.
    #[cfg_attr(feature = "clap", value(name = "aarch64-pc-windows-msvc"))]
    #[serde(rename = "aarch64-pc-windows-msvc")]
    #[serde(alias = "arm64-pc-windows-msvc")]
    Aarch64PcWindowsMsvc,

    /// A 32-bit x86 Windows target.
    #[cfg_attr(feature = "clap", value(name = "i686-pc-windows-msvc"))]
    #[serde(rename = "i686-pc-windows-msvc")]
    I686PcWindowsMsvc,

    /// An x86 Linux target. Equivalent to `x86_64-manylinux_2_28`.
    #[cfg_attr(feature = "clap", value(name = "x86_64-unknown-linux-gnu"))]
    #[serde(rename = "x86_64-unknown-linux-gnu")]
    #[serde(alias = "x8664-unknown-linux-gnu")]
    X8664UnknownLinuxGnu,

    /// An ARM-based macOS target, as seen on Apple Silicon devices
    ///
    /// By default, assumes the least-recent, non-EOL macOS version (13.0), but respects
    /// the `MACOSX_DEPLOYMENT_TARGET` environment variable if set.
    #[cfg_attr(feature = "clap", value(name = "aarch64-apple-darwin"))]
    #[serde(rename = "aarch64-apple-darwin")]
    Aarch64AppleDarwin,

    /// An x86 macOS target.
    ///
    /// By default, assumes the least-recent, non-EOL macOS version (13.0), but respects
    /// the `MACOSX_DEPLOYMENT_TARGET` environment variable if set.
    #[cfg_attr(feature = "clap", value(name = "x86_64-apple-darwin"))]
    #[serde(rename = "x86_64-apple-darwin")]
    #[serde(alias = "x8664-apple-darwin")]
    X8664AppleDarwin,

    /// An ARM64 Linux target. Equivalent to `aarch64-manylinux_2_28`.
    #[cfg_attr(feature = "clap", value(name = "aarch64-unknown-linux-gnu"))]
    #[serde(rename = "aarch64-unknown-linux-gnu")]
    Aarch64UnknownLinuxGnu,

    /// An ARM64 Linux target.
    #[cfg_attr(feature = "clap", value(name = "aarch64-unknown-linux-musl"))]
    #[serde(rename = "aarch64-unknown-linux-musl")]
    Aarch64UnknownLinuxMusl,

    /// An `x86_64` Linux target.
    #[cfg_attr(feature = "clap", value(name = "x86_64-unknown-linux-musl"))]
    #[serde(rename = "x86_64-unknown-linux-musl")]
    #[serde(alias = "x8664-unknown-linux-musl")]
    X8664UnknownLinuxMusl,

    /// An s390x Linux target. Equivalent to `s390x-manylinux_2_28`.
    #[cfg_attr(feature = "clap", value(name = "s390x-unknown-linux-gnu"))]
    #[serde(rename = "s390x-unknown-linux-gnu")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    S390XUnknownLinuxGnu,

    /// A little-endian `PowerPC64` Linux target. Equivalent to `ppc64le-manylinux_2_28`.
    #[cfg_attr(feature = "clap", value(name = "powerpc64le-unknown-linux-gnu"))]
    #[serde(rename = "powerpc64le-unknown-linux-gnu")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    Powerpc64LeUnknownLinuxGnu,

    /// A `LoongArch64` Linux target. Equivalent to `loongarch64-manylinux_2_36`.
    #[cfg_attr(feature = "clap", value(name = "loongarch64-unknown-linux-gnu"))]
    #[serde(rename = "loongarch64-unknown-linux-gnu")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    LoongArch64UnknownLinuxGnu,

    /// A RISCV64 Linux target.
    #[cfg_attr(feature = "clap", value(name = "riscv64-unknown-linux"))]
    #[serde(rename = "riscv64-unknown-linux")]
    Riscv64UnknownLinuxGnu,

    /// An `x86_64` target for the `manylinux2014` platform. Equivalent to `x86_64-manylinux_2_17`.
    #[cfg_attr(
        feature = "clap",
        value(name = "x86_64-manylinux2014", alias = "manylinux2014_x86_64")
    )]
    #[serde(rename = "x86_64-manylinux2014")]
    #[serde(alias = "x8664-manylinux2014")]
    #[serde(alias = "manylinux2014_x86_64")]
    X8664Manylinux2014,

    /// An `x86_64` target for the `manylinux_2_17` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "x86_64-manylinux_2_17", alias = "manylinux_2_17_x86_64")
    )]
    #[serde(rename = "x86_64-manylinux_2_17")]
    #[serde(alias = "x8664-manylinux217")]
    #[serde(alias = "manylinux_2_17_x86_64")]
    X8664Manylinux217,

    /// An `x86_64` target for the `manylinux_2_28` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "x86_64-manylinux_2_28", alias = "manylinux_2_28_x86_64")
    )]
    #[serde(rename = "x86_64-manylinux_2_28")]
    #[serde(alias = "x8664-manylinux228")]
    #[serde(alias = "manylinux_2_28_x86_64")]
    X8664Manylinux228,

    /// An `x86_64` target for the `manylinux_2_31` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "x86_64-manylinux_2_31", alias = "manylinux_2_31_x86_64")
    )]
    #[serde(rename = "x86_64-manylinux_2_31")]
    #[serde(alias = "x8664-manylinux231")]
    #[serde(alias = "manylinux_2_31_x86_64")]
    X8664Manylinux231,

    /// An `x86_64` target for the `manylinux_2_32` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "x86_64-manylinux_2_32", alias = "manylinux_2_32_x86_64")
    )]
    #[serde(rename = "x86_64-manylinux_2_32")]
    #[serde(alias = "x8664-manylinux232")]
    #[serde(alias = "manylinux_2_32_x86_64")]
    X8664Manylinux232,

    /// An `x86_64` target for the `manylinux_2_33` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "x86_64-manylinux_2_33", alias = "manylinux_2_33_x86_64")
    )]
    #[serde(rename = "x86_64-manylinux_2_33")]
    #[serde(alias = "x8664-manylinux233")]
    #[serde(alias = "manylinux_2_33_x86_64")]
    X8664Manylinux233,

    /// An `x86_64` target for the `manylinux_2_34` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "x86_64-manylinux_2_34", alias = "manylinux_2_34_x86_64")
    )]
    #[serde(rename = "x86_64-manylinux_2_34")]
    #[serde(alias = "x8664-manylinux234")]
    #[serde(alias = "manylinux_2_34_x86_64")]
    X8664Manylinux234,

    /// An `x86_64` target for the `manylinux_2_35` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "x86_64-manylinux_2_35", alias = "manylinux_2_35_x86_64")
    )]
    #[serde(rename = "x86_64-manylinux_2_35")]
    #[serde(alias = "x8664-manylinux235")]
    #[serde(alias = "manylinux_2_35_x86_64")]
    X8664Manylinux235,

    /// An `x86_64` target for the `manylinux_2_36` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "x86_64-manylinux_2_36", alias = "manylinux_2_36_x86_64")
    )]
    #[serde(rename = "x86_64-manylinux_2_36")]
    #[serde(alias = "x8664-manylinux236")]
    #[serde(alias = "manylinux_2_36_x86_64")]
    X8664Manylinux236,

    /// An `x86_64` target for the `manylinux_2_37` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "x86_64-manylinux_2_37", alias = "manylinux_2_37_x86_64")
    )]
    #[serde(rename = "x86_64-manylinux_2_37")]
    #[serde(alias = "x8664-manylinux237")]
    #[serde(alias = "manylinux_2_37_x86_64")]
    X8664Manylinux237,

    /// An `x86_64` target for the `manylinux_2_38` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "x86_64-manylinux_2_38", alias = "manylinux_2_38_x86_64")
    )]
    #[serde(rename = "x86_64-manylinux_2_38")]
    #[serde(alias = "x8664-manylinux238")]
    #[serde(alias = "manylinux_2_38_x86_64")]
    X8664Manylinux238,

    /// An `x86_64` target for the `manylinux_2_39` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "x86_64-manylinux_2_39", alias = "manylinux_2_39_x86_64")
    )]
    #[serde(rename = "x86_64-manylinux_2_39")]
    #[serde(alias = "x8664-manylinux239")]
    #[serde(alias = "manylinux_2_39_x86_64")]
    X8664Manylinux239,

    /// An `x86_64` target for the `manylinux_2_40` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "x86_64-manylinux_2_40", alias = "manylinux_2_40_x86_64")
    )]
    #[serde(rename = "x86_64-manylinux_2_40")]
    #[serde(alias = "x8664-manylinux240")]
    #[serde(alias = "manylinux_2_40_x86_64")]
    X8664Manylinux240,

    /// An ARM64 target for the `manylinux2014` platform. Equivalent to `aarch64-manylinux_2_17`.
    #[cfg_attr(
        feature = "clap",
        value(name = "aarch64-manylinux2014", alias = "manylinux2014_aarch64")
    )]
    #[serde(rename = "aarch64-manylinux2014")]
    #[serde(alias = "manylinux2014_aarch64")]
    Aarch64Manylinux2014,

    /// An ARM64 target for the `manylinux_2_17` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "aarch64-manylinux_2_17", alias = "manylinux_2_17_aarch64")
    )]
    #[serde(rename = "aarch64-manylinux_2_17")]
    #[serde(alias = "aarch64-manylinux217")]
    #[serde(alias = "manylinux_2_17_aarch64")]
    Aarch64Manylinux217,

    /// An ARM64 target for the `manylinux_2_28` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "aarch64-manylinux_2_28", alias = "manylinux_2_28_aarch64")
    )]
    #[serde(rename = "aarch64-manylinux_2_28")]
    #[serde(alias = "aarch64-manylinux228")]
    #[serde(alias = "manylinux_2_28_aarch64")]
    Aarch64Manylinux228,

    /// An ARM64 target for the `manylinux_2_31` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "aarch64-manylinux_2_31", alias = "manylinux_2_31_aarch64")
    )]
    #[serde(rename = "aarch64-manylinux_2_31")]
    #[serde(alias = "aarch64-manylinux231")]
    #[serde(alias = "manylinux_2_31_aarch64")]
    Aarch64Manylinux231,

    /// An ARM64 target for the `manylinux_2_32` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "aarch64-manylinux_2_32", alias = "manylinux_2_32_aarch64")
    )]
    #[serde(rename = "aarch64-manylinux_2_32")]
    #[serde(alias = "aarch64-manylinux232")]
    #[serde(alias = "manylinux_2_32_aarch64")]
    Aarch64Manylinux232,

    /// An ARM64 target for the `manylinux_2_33` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "aarch64-manylinux_2_33", alias = "manylinux_2_33_aarch64")
    )]
    #[serde(rename = "aarch64-manylinux_2_33")]
    #[serde(alias = "aarch64-manylinux233")]
    #[serde(alias = "manylinux_2_33_aarch64")]
    Aarch64Manylinux233,

    /// An ARM64 target for the `manylinux_2_34` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "aarch64-manylinux_2_34", alias = "manylinux_2_34_aarch64")
    )]
    #[serde(rename = "aarch64-manylinux_2_34")]
    #[serde(alias = "aarch64-manylinux234")]
    #[serde(alias = "manylinux_2_34_aarch64")]
    Aarch64Manylinux234,

    /// An ARM64 target for the `manylinux_2_35` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "aarch64-manylinux_2_35", alias = "manylinux_2_35_aarch64")
    )]
    #[serde(rename = "aarch64-manylinux_2_35")]
    #[serde(alias = "aarch64-manylinux235")]
    #[serde(alias = "manylinux_2_35_aarch64")]
    Aarch64Manylinux235,

    /// An ARM64 target for the `manylinux_2_36` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "aarch64-manylinux_2_36", alias = "manylinux_2_36_aarch64")
    )]
    #[serde(rename = "aarch64-manylinux_2_36")]
    #[serde(alias = "aarch64-manylinux236")]
    #[serde(alias = "manylinux_2_36_aarch64")]
    Aarch64Manylinux236,

    /// An ARM64 target for the `manylinux_2_37` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "aarch64-manylinux_2_37", alias = "manylinux_2_37_aarch64")
    )]
    #[serde(rename = "aarch64-manylinux_2_37")]
    #[serde(alias = "aarch64-manylinux237")]
    #[serde(alias = "manylinux_2_37_aarch64")]
    Aarch64Manylinux237,

    /// An ARM64 target for the `manylinux_2_38` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "aarch64-manylinux_2_38", alias = "manylinux_2_38_aarch64")
    )]
    #[serde(rename = "aarch64-manylinux_2_38")]
    #[serde(alias = "aarch64-manylinux238")]
    #[serde(alias = "manylinux_2_38_aarch64")]
    Aarch64Manylinux238,

    /// An ARM64 target for the `manylinux_2_39` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "aarch64-manylinux_2_39", alias = "manylinux_2_39_aarch64")
    )]
    #[serde(rename = "aarch64-manylinux_2_39")]
    #[serde(alias = "aarch64-manylinux239")]
    #[serde(alias = "manylinux_2_39_aarch64")]
    Aarch64Manylinux239,

    /// An ARM64 target for the `manylinux_2_40` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "aarch64-manylinux_2_40", alias = "manylinux_2_40_aarch64")
    )]
    #[serde(rename = "aarch64-manylinux_2_40")]
    #[serde(alias = "aarch64-manylinux240")]
    #[serde(alias = "manylinux_2_40_aarch64")]
    Aarch64Manylinux240,

    /// An s390x target for the `manylinux2014` platform. Equivalent to `s390x-manylinux_2_17`.
    #[cfg_attr(
        feature = "clap",
        value(name = "s390x-manylinux2014", alias = "manylinux2014_s390x")
    )]
    #[serde(rename = "s390x-manylinux2014")]
    #[serde(alias = "manylinux2014_s390x")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    S390XManylinux2014,

    /// An s390x target for the `manylinux_2_17` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "s390x-manylinux_2_17", alias = "manylinux_2_17_s390x")
    )]
    #[serde(rename = "s390x-manylinux_2_17")]
    #[serde(alias = "manylinux_2_17_s390x")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    S390XManylinux217,

    /// An s390x target for the `manylinux_2_28` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "s390x-manylinux_2_28", alias = "manylinux_2_28_s390x")
    )]
    #[serde(rename = "s390x-manylinux_2_28")]
    #[serde(alias = "manylinux_2_28_s390x")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    S390XManylinux228,

    /// An s390x target for the `manylinux_2_31` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "s390x-manylinux_2_31", alias = "manylinux_2_31_s390x")
    )]
    #[serde(rename = "s390x-manylinux_2_31")]
    #[serde(alias = "manylinux_2_31_s390x")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    S390XManylinux231,

    /// An s390x target for the `manylinux_2_32` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "s390x-manylinux_2_32", alias = "manylinux_2_32_s390x")
    )]
    #[serde(rename = "s390x-manylinux_2_32")]
    #[serde(alias = "manylinux_2_32_s390x")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    S390XManylinux232,

    /// An s390x target for the `manylinux_2_33` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "s390x-manylinux_2_33", alias = "manylinux_2_33_s390x")
    )]
    #[serde(rename = "s390x-manylinux_2_33")]
    #[serde(alias = "manylinux_2_33_s390x")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    S390XManylinux233,

    /// An s390x target for the `manylinux_2_34` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "s390x-manylinux_2_34", alias = "manylinux_2_34_s390x")
    )]
    #[serde(rename = "s390x-manylinux_2_34")]
    #[serde(alias = "manylinux_2_34_s390x")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    S390XManylinux234,

    /// An s390x target for the `manylinux_2_35` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "s390x-manylinux_2_35", alias = "manylinux_2_35_s390x")
    )]
    #[serde(rename = "s390x-manylinux_2_35")]
    #[serde(alias = "manylinux_2_35_s390x")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    S390XManylinux235,

    /// An s390x target for the `manylinux_2_36` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "s390x-manylinux_2_36", alias = "manylinux_2_36_s390x")
    )]
    #[serde(rename = "s390x-manylinux_2_36")]
    #[serde(alias = "manylinux_2_36_s390x")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    S390XManylinux236,

    /// An s390x target for the `manylinux_2_37` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "s390x-manylinux_2_37", alias = "manylinux_2_37_s390x")
    )]
    #[serde(rename = "s390x-manylinux_2_37")]
    #[serde(alias = "manylinux_2_37_s390x")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    S390XManylinux237,

    /// An s390x target for the `manylinux_2_38` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "s390x-manylinux_2_38", alias = "manylinux_2_38_s390x")
    )]
    #[serde(rename = "s390x-manylinux_2_38")]
    #[serde(alias = "manylinux_2_38_s390x")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    S390XManylinux238,

    /// An s390x target for the `manylinux_2_39` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "s390x-manylinux_2_39", alias = "manylinux_2_39_s390x")
    )]
    #[serde(rename = "s390x-manylinux_2_39")]
    #[serde(alias = "manylinux_2_39_s390x")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    S390XManylinux239,

    /// An s390x target for the `manylinux_2_40` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "s390x-manylinux_2_40", alias = "manylinux_2_40_s390x")
    )]
    #[serde(rename = "s390x-manylinux_2_40")]
    #[serde(alias = "manylinux_2_40_s390x")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    S390XManylinux240,

    /// A little-endian `PowerPC64` target for the `manylinux2014` platform. Equivalent to `ppc64le-manylinux_2_17`.
    #[cfg_attr(
        feature = "clap",
        value(name = "ppc64le-manylinux2014", alias = "manylinux2014_ppc64le")
    )]
    #[serde(rename = "ppc64le-manylinux2014")]
    #[serde(alias = "manylinux2014_ppc64le")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    Powerpc64LeManylinux2014,

    /// A little-endian `PowerPC64` target for the `manylinux_2_17` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "ppc64le-manylinux_2_17", alias = "manylinux_2_17_ppc64le")
    )]
    #[serde(rename = "ppc64le-manylinux_2_17")]
    #[serde(alias = "manylinux_2_17_ppc64le")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    Powerpc64LeManylinux217,

    /// A little-endian `PowerPC64` target for the `manylinux_2_28` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "ppc64le-manylinux_2_28", alias = "manylinux_2_28_ppc64le")
    )]
    #[serde(rename = "ppc64le-manylinux_2_28")]
    #[serde(alias = "manylinux_2_28_ppc64le")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    Powerpc64LeManylinux228,

    /// A little-endian `PowerPC64` target for the `manylinux_2_31` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "ppc64le-manylinux_2_31", alias = "manylinux_2_31_ppc64le")
    )]
    #[serde(rename = "ppc64le-manylinux_2_31")]
    #[serde(alias = "manylinux_2_31_ppc64le")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    Powerpc64LeManylinux231,

    /// A little-endian `PowerPC64` target for the `manylinux_2_32` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "ppc64le-manylinux_2_32", alias = "manylinux_2_32_ppc64le")
    )]
    #[serde(rename = "ppc64le-manylinux_2_32")]
    #[serde(alias = "manylinux_2_32_ppc64le")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    Powerpc64LeManylinux232,

    /// A little-endian `PowerPC64` target for the `manylinux_2_33` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "ppc64le-manylinux_2_33", alias = "manylinux_2_33_ppc64le")
    )]
    #[serde(rename = "ppc64le-manylinux_2_33")]
    #[serde(alias = "manylinux_2_33_ppc64le")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    Powerpc64LeManylinux233,

    /// A little-endian `PowerPC64` target for the `manylinux_2_34` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "ppc64le-manylinux_2_34", alias = "manylinux_2_34_ppc64le")
    )]
    #[serde(rename = "ppc64le-manylinux_2_34")]
    #[serde(alias = "manylinux_2_34_ppc64le")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    Powerpc64LeManylinux234,

    /// A little-endian `PowerPC64` target for the `manylinux_2_35` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "ppc64le-manylinux_2_35", alias = "manylinux_2_35_ppc64le")
    )]
    #[serde(rename = "ppc64le-manylinux_2_35")]
    #[serde(alias = "manylinux_2_35_ppc64le")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    Powerpc64LeManylinux235,

    /// A little-endian `PowerPC64` target for the `manylinux_2_36` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "ppc64le-manylinux_2_36", alias = "manylinux_2_36_ppc64le")
    )]
    #[serde(rename = "ppc64le-manylinux_2_36")]
    #[serde(alias = "manylinux_2_36_ppc64le")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    Powerpc64LeManylinux236,

    /// A little-endian `PowerPC64` target for the `manylinux_2_37` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "ppc64le-manylinux_2_37", alias = "manylinux_2_37_ppc64le")
    )]
    #[serde(rename = "ppc64le-manylinux_2_37")]
    #[serde(alias = "manylinux_2_37_ppc64le")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    Powerpc64LeManylinux237,

    /// A little-endian `PowerPC64` target for the `manylinux_2_38` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "ppc64le-manylinux_2_38", alias = "manylinux_2_38_ppc64le")
    )]
    #[serde(rename = "ppc64le-manylinux_2_38")]
    #[serde(alias = "manylinux_2_38_ppc64le")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    Powerpc64LeManylinux238,

    /// A little-endian `PowerPC64` target for the `manylinux_2_39` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "ppc64le-manylinux_2_39", alias = "manylinux_2_39_ppc64le")
    )]
    #[serde(rename = "ppc64le-manylinux_2_39")]
    #[serde(alias = "manylinux_2_39_ppc64le")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    Powerpc64LeManylinux239,

    /// A little-endian `PowerPC64` target for the `manylinux_2_40` platform.
    #[cfg_attr(
        feature = "clap",
        value(name = "ppc64le-manylinux_2_40", alias = "manylinux_2_40_ppc64le")
    )]
    #[serde(rename = "ppc64le-manylinux_2_40")]
    #[serde(alias = "manylinux_2_40_ppc64le")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    Powerpc64LeManylinux240,

    /// A `LoongArch64` target for the `manylinux_2_36` platform.
    #[cfg_attr(
        feature = "clap",
        value(
            name = "loongarch64-manylinux_2_36",
            alias = "manylinux_2_36_loongarch64"
        )
    )]
    #[serde(rename = "loongarch64-manylinux_2_36")]
    #[serde(alias = "manylinux_2_36_loongarch64")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    LoongArch64Manylinux236,

    /// A `LoongArch64` target for the `manylinux_2_37` platform.
    #[cfg_attr(
        feature = "clap",
        value(
            name = "loongarch64-manylinux_2_37",
            alias = "manylinux_2_37_loongarch64"
        )
    )]
    #[serde(rename = "loongarch64-manylinux_2_37")]
    #[serde(alias = "manylinux_2_37_loongarch64")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    LoongArch64Manylinux237,

    /// A `LoongArch64` target for the `manylinux_2_38` platform.
    #[cfg_attr(
        feature = "clap",
        value(
            name = "loongarch64-manylinux_2_38",
            alias = "manylinux_2_38_loongarch64"
        )
    )]
    #[serde(rename = "loongarch64-manylinux_2_38")]
    #[serde(alias = "manylinux_2_38_loongarch64")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    LoongArch64Manylinux238,

    /// A `LoongArch64` target for the `manylinux_2_39` platform.
    #[cfg_attr(
        feature = "clap",
        value(
            name = "loongarch64-manylinux_2_39",
            alias = "manylinux_2_39_loongarch64"
        )
    )]
    #[serde(rename = "loongarch64-manylinux_2_39")]
    #[serde(alias = "manylinux_2_39_loongarch64")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    LoongArch64Manylinux239,

    /// A `LoongArch64` target for the `manylinux_2_40` platform.
    #[cfg_attr(
        feature = "clap",
        value(
            name = "loongarch64-manylinux_2_40",
            alias = "manylinux_2_40_loongarch64"
        )
    )]
    #[serde(rename = "loongarch64-manylinux_2_40")]
    #[serde(alias = "manylinux_2_40_loongarch64")]
    #[cfg_attr(feature = "clap", value(hide = true))]
    LoongArch64Manylinux240,

    /// An ARM64 Android target.
    ///
    /// By default uses Android API level 24, but respects
    /// the `ANDROID_API_LEVEL` environment variable if set.
    #[cfg_attr(feature = "clap", value(name = "aarch64-linux-android"))]
    #[serde(rename = "aarch64-linux-android")]
    Aarch64LinuxAndroid,

    /// An `x86_64` Android target.
    ///
    /// By default uses Android API level 24, but respects
    /// the `ANDROID_API_LEVEL` environment variable if set.
    #[cfg_attr(feature = "clap", value(name = "x86_64-linux-android"))]
    #[serde(rename = "x86_64-linux-android")]
    X8664LinuxAndroid,

    /// A wasm32 target using the Pyodide 2024 platform. Meant for use with Python 3.12.
    /// See <https://pyodide.org/en/stable/development/abi/312.html>
    #[cfg_attr(feature = "clap", value(name = "wasm32-pyodide2024"))]
    Wasm32Pyodide2024,

    /// A wasm32 target using the Pyodide 2025 platform. Meant for use with Python 3.13.
    /// See <https://pyodide.org/en/stable/development/abi/313.html>
    #[cfg_attr(feature = "clap", value(name = "wasm32-pyodide2025"))]
    Wasm32Pyodide2025,

    /// An ARM64 target for iOS device
    ///
    /// By default, iOS 13.0 is used, but respects the `IPHONEOS_DEPLOYMENT_TARGET`
    /// environment variable if set.
    #[cfg_attr(feature = "clap", value(name = "arm64-apple-ios"))]
    #[serde(rename = "arm64-apple-ios")]
    Arm64Ios,

    /// An ARM64 target for iOS simulator
    ///
    /// By default, iOS 13.0 is used, but respects the `IPHONEOS_DEPLOYMENT_TARGET`
    /// environment variable if set.
    #[cfg_attr(feature = "clap", value(name = "arm64-apple-ios-simulator"))]
    #[serde(rename = "arm64-apple-ios-simulator")]
    Arm64IosSimulator,

    /// An `x86_64` target for iOS simulator
    ///
    /// By default, iOS 13.0 is used, but respects the `IPHONEOS_DEPLOYMENT_TARGET`
    /// environment variable if set.
    #[cfg_attr(feature = "clap", value(name = "x86_64-apple-ios-simulator"))]
    #[serde(rename = "x86_64-apple-ios-simulator")]
    X8664IosSimulator,
}

/// The operating system of a supported target triple.
enum TargetOs {
    Windows,
    Linux,
    Macos,
    Android,
    Emscripten { release: &'static str },
    Ios,
}

impl TargetOs {
    /// Return the `platform_release` and `platform_version` markers.
    fn platform_release_and_version(self) -> (&'static str, &'static str) {
        match self {
            // Emscripten reports `#1` as the platform version:
            // https://github.com/emscripten-core/emscripten/blob/4.0.8/system/lib/libc/emscripten_syscall_stubs.c#L63
            Self::Emscripten { release } => (release, "#1"),
            Self::Windows | Self::Linux | Self::Macos | Self::Android | Self::Ios => ("", ""),
        }
    }
}

impl TargetTriple {
    /// Return the [`Platform`] for the target.
    pub fn platform(self) -> Platform {
        match self {
            Self::Windows | Self::X8664PcWindowsMsvc => Platform::new(Os::Windows, Arch::X86_64),
            Self::Aarch64PcWindowsMsvc => Platform::new(Os::Windows, Arch::Aarch64),
            Self::Linux | Self::X8664UnknownLinuxGnu => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 28,
                },
                Arch::X86_64,
            ),
            Self::Macos | Self::Aarch64AppleDarwin => {
                let (major, minor) = macos_deployment_target().map_or((13, 0), |(major, minor)| {
                    debug!("Found macOS deployment target: {}.{}", major, minor);
                    (major, minor)
                });
                Platform::new(Os::Macos { major, minor }, Arch::Aarch64)
            }
            Self::I686PcWindowsMsvc => Platform::new(Os::Windows, Arch::X86),
            Self::X8664AppleDarwin => {
                let (major, minor) = macos_deployment_target().map_or((13, 0), |(major, minor)| {
                    debug!("Found macOS deployment target: {}.{}", major, minor);
                    (major, minor)
                });
                Platform::new(Os::Macos { major, minor }, Arch::X86_64)
            }
            Self::Aarch64UnknownLinuxGnu => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 28,
                },
                Arch::Aarch64,
            ),
            Self::S390XUnknownLinuxGnu => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 28,
                },
                Arch::S390X,
            ),
            Self::Powerpc64LeUnknownLinuxGnu => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 28,
                },
                Arch::Powerpc64Le,
            ),
            Self::LoongArch64UnknownLinuxGnu => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 36,
                },
                Arch::LoongArch64,
            ),
            Self::Riscv64UnknownLinuxGnu => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 39,
                },
                Arch::Riscv64,
            ),
            Self::Aarch64UnknownLinuxMusl => {
                Platform::new(Os::Musllinux { major: 1, minor: 2 }, Arch::Aarch64)
            }
            Self::X8664UnknownLinuxMusl => {
                Platform::new(Os::Musllinux { major: 1, minor: 2 }, Arch::X86_64)
            }
            Self::X8664Manylinux2014 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 17,
                },
                Arch::X86_64,
            ),
            Self::X8664Manylinux217 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 17,
                },
                Arch::X86_64,
            ),
            Self::X8664Manylinux228 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 28,
                },
                Arch::X86_64,
            ),
            Self::X8664Manylinux231 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 31,
                },
                Arch::X86_64,
            ),
            Self::X8664Manylinux232 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 32,
                },
                Arch::X86_64,
            ),
            Self::X8664Manylinux233 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 33,
                },
                Arch::X86_64,
            ),
            Self::X8664Manylinux234 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 34,
                },
                Arch::X86_64,
            ),
            Self::X8664Manylinux235 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 35,
                },
                Arch::X86_64,
            ),
            Self::X8664Manylinux236 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 36,
                },
                Arch::X86_64,
            ),
            Self::X8664Manylinux237 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 37,
                },
                Arch::X86_64,
            ),
            Self::X8664Manylinux238 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 38,
                },
                Arch::X86_64,
            ),
            Self::X8664Manylinux239 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 39,
                },
                Arch::X86_64,
            ),
            Self::X8664Manylinux240 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 40,
                },
                Arch::X86_64,
            ),
            Self::Aarch64Manylinux2014 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 17,
                },
                Arch::Aarch64,
            ),
            Self::Aarch64Manylinux217 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 17,
                },
                Arch::Aarch64,
            ),
            Self::Aarch64Manylinux228 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 28,
                },
                Arch::Aarch64,
            ),
            Self::Aarch64Manylinux231 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 31,
                },
                Arch::Aarch64,
            ),
            Self::Aarch64Manylinux232 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 32,
                },
                Arch::Aarch64,
            ),
            Self::Aarch64Manylinux233 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 33,
                },
                Arch::Aarch64,
            ),
            Self::Aarch64Manylinux234 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 34,
                },
                Arch::Aarch64,
            ),
            Self::Aarch64Manylinux235 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 35,
                },
                Arch::Aarch64,
            ),
            Self::Aarch64Manylinux236 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 36,
                },
                Arch::Aarch64,
            ),
            Self::Aarch64Manylinux237 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 37,
                },
                Arch::Aarch64,
            ),
            Self::Aarch64Manylinux238 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 38,
                },
                Arch::Aarch64,
            ),
            Self::Aarch64Manylinux239 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 39,
                },
                Arch::Aarch64,
            ),
            Self::Aarch64Manylinux240 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 40,
                },
                Arch::Aarch64,
            ),
            Self::S390XManylinux2014 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 17,
                },
                Arch::S390X,
            ),
            Self::S390XManylinux217 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 17,
                },
                Arch::S390X,
            ),
            Self::S390XManylinux228 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 28,
                },
                Arch::S390X,
            ),
            Self::S390XManylinux231 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 31,
                },
                Arch::S390X,
            ),
            Self::S390XManylinux232 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 32,
                },
                Arch::S390X,
            ),
            Self::S390XManylinux233 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 33,
                },
                Arch::S390X,
            ),
            Self::S390XManylinux234 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 34,
                },
                Arch::S390X,
            ),
            Self::S390XManylinux235 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 35,
                },
                Arch::S390X,
            ),
            Self::S390XManylinux236 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 36,
                },
                Arch::S390X,
            ),
            Self::S390XManylinux237 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 37,
                },
                Arch::S390X,
            ),
            Self::S390XManylinux238 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 38,
                },
                Arch::S390X,
            ),
            Self::S390XManylinux239 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 39,
                },
                Arch::S390X,
            ),
            Self::S390XManylinux240 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 40,
                },
                Arch::S390X,
            ),
            Self::Powerpc64LeManylinux2014 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 17,
                },
                Arch::Powerpc64Le,
            ),
            Self::Powerpc64LeManylinux217 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 17,
                },
                Arch::Powerpc64Le,
            ),
            Self::Powerpc64LeManylinux228 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 28,
                },
                Arch::Powerpc64Le,
            ),
            Self::Powerpc64LeManylinux231 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 31,
                },
                Arch::Powerpc64Le,
            ),
            Self::Powerpc64LeManylinux232 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 32,
                },
                Arch::Powerpc64Le,
            ),
            Self::Powerpc64LeManylinux233 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 33,
                },
                Arch::Powerpc64Le,
            ),
            Self::Powerpc64LeManylinux234 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 34,
                },
                Arch::Powerpc64Le,
            ),
            Self::Powerpc64LeManylinux235 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 35,
                },
                Arch::Powerpc64Le,
            ),
            Self::Powerpc64LeManylinux236 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 36,
                },
                Arch::Powerpc64Le,
            ),
            Self::Powerpc64LeManylinux237 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 37,
                },
                Arch::Powerpc64Le,
            ),
            Self::Powerpc64LeManylinux238 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 38,
                },
                Arch::Powerpc64Le,
            ),
            Self::Powerpc64LeManylinux239 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 39,
                },
                Arch::Powerpc64Le,
            ),
            Self::Powerpc64LeManylinux240 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 40,
                },
                Arch::Powerpc64Le,
            ),
            Self::LoongArch64Manylinux236 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 36,
                },
                Arch::LoongArch64,
            ),
            Self::LoongArch64Manylinux237 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 37,
                },
                Arch::LoongArch64,
            ),
            Self::LoongArch64Manylinux238 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 38,
                },
                Arch::LoongArch64,
            ),
            Self::LoongArch64Manylinux239 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 39,
                },
                Arch::LoongArch64,
            ),
            Self::LoongArch64Manylinux240 => Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 40,
                },
                Arch::LoongArch64,
            ),
            Self::Wasm32Pyodide2024 => Platform::new(
                Os::Pyodide {
                    major: 2024,
                    minor: 0,
                },
                Arch::Wasm32,
            ),
            Self::Wasm32Pyodide2025 => Platform::new(
                Os::Pyodide {
                    major: 2025,
                    minor: 0,
                },
                Arch::Wasm32,
            ),
            Self::Aarch64LinuxAndroid => {
                let api_level = android_api_level().map_or(24, |api_level| {
                    debug!("Found Android API level: {}", api_level);
                    api_level
                });
                Platform::new(Os::Android { api_level }, Arch::Aarch64)
            }
            Self::X8664LinuxAndroid => {
                let api_level = android_api_level().map_or(24, |api_level| {
                    debug!("Found Android API level: {}", api_level);
                    api_level
                });
                Platform::new(Os::Android { api_level }, Arch::X86_64)
            }
            Self::Arm64Ios => {
                let (major, minor) = ios_deployment_target().map_or((13, 0), |(major, minor)| {
                    debug!("Found iOS deployment target: {}.{}", major, minor);
                    (major, minor)
                });
                Platform::new(
                    Os::Ios {
                        major,
                        minor,
                        simulator: false,
                    },
                    Arch::Aarch64,
                )
            }
            Self::Arm64IosSimulator => {
                let (major, minor) = ios_deployment_target().map_or((13, 0), |(major, minor)| {
                    debug!("Found iOS deployment target: {}.{}", major, minor);
                    (major, minor)
                });
                Platform::new(
                    Os::Ios {
                        major,
                        minor,
                        simulator: true,
                    },
                    Arch::Aarch64,
                )
            }
            Self::X8664IosSimulator => {
                let (major, minor) = ios_deployment_target().map_or((13, 0), |(major, minor)| {
                    debug!("Found iOS deployment target: {}.{}", major, minor);
                    (major, minor)
                });
                Platform::new(
                    Os::Ios {
                        major,
                        minor,
                        simulator: true,
                    },
                    Arch::X86_64,
                )
            }
        }
    }

    /// Return the `platform_machine` value for the target.
    fn platform_machine(self) -> &'static str {
        match self {
            Self::Windows | Self::X8664PcWindowsMsvc => "x86_64",
            Self::Aarch64PcWindowsMsvc => "ARM64",
            Self::Linux | Self::X8664UnknownLinuxGnu => "x86_64",
            Self::Macos | Self::Aarch64AppleDarwin => "arm64",
            Self::I686PcWindowsMsvc => "x86",
            Self::X8664AppleDarwin => "x86_64",
            Self::Aarch64UnknownLinuxGnu => "aarch64",
            Self::Aarch64UnknownLinuxMusl => "aarch64",
            Self::X8664UnknownLinuxMusl => "x86_64",
            Self::Riscv64UnknownLinuxGnu => "riscv64",
            Self::S390XUnknownLinuxGnu => "s390x",
            Self::Powerpc64LeUnknownLinuxGnu => "ppc64le",
            Self::LoongArch64UnknownLinuxGnu => "loongarch64",
            Self::X8664Manylinux2014 => "x86_64",
            Self::X8664Manylinux217 => "x86_64",
            Self::X8664Manylinux228 => "x86_64",
            Self::X8664Manylinux231 => "x86_64",
            Self::X8664Manylinux232 => "x86_64",
            Self::X8664Manylinux233 => "x86_64",
            Self::X8664Manylinux234 => "x86_64",
            Self::X8664Manylinux235 => "x86_64",
            Self::X8664Manylinux236 => "x86_64",
            Self::X8664Manylinux237 => "x86_64",
            Self::X8664Manylinux238 => "x86_64",
            Self::X8664Manylinux239 => "x86_64",
            Self::X8664Manylinux240 => "x86_64",
            Self::Aarch64Manylinux2014 => "aarch64",
            Self::Aarch64Manylinux217 => "aarch64",
            Self::Aarch64Manylinux228 => "aarch64",
            Self::Aarch64Manylinux231 => "aarch64",
            Self::Aarch64Manylinux232 => "aarch64",
            Self::Aarch64Manylinux233 => "aarch64",
            Self::Aarch64Manylinux234 => "aarch64",
            Self::Aarch64Manylinux235 => "aarch64",
            Self::Aarch64Manylinux236 => "aarch64",
            Self::Aarch64Manylinux237 => "aarch64",
            Self::Aarch64Manylinux238 => "aarch64",
            Self::Aarch64Manylinux239 => "aarch64",
            Self::Aarch64Manylinux240 => "aarch64",
            Self::S390XManylinux2014 => "s390x",
            Self::S390XManylinux217 => "s390x",
            Self::S390XManylinux228 => "s390x",
            Self::S390XManylinux231 => "s390x",
            Self::S390XManylinux232 => "s390x",
            Self::S390XManylinux233 => "s390x",
            Self::S390XManylinux234 => "s390x",
            Self::S390XManylinux235 => "s390x",
            Self::S390XManylinux236 => "s390x",
            Self::S390XManylinux237 => "s390x",
            Self::S390XManylinux238 => "s390x",
            Self::S390XManylinux239 => "s390x",
            Self::S390XManylinux240 => "s390x",
            Self::Powerpc64LeManylinux2014 => "ppc64le",
            Self::Powerpc64LeManylinux217 => "ppc64le",
            Self::Powerpc64LeManylinux228 => "ppc64le",
            Self::Powerpc64LeManylinux231 => "ppc64le",
            Self::Powerpc64LeManylinux232 => "ppc64le",
            Self::Powerpc64LeManylinux233 => "ppc64le",
            Self::Powerpc64LeManylinux234 => "ppc64le",
            Self::Powerpc64LeManylinux235 => "ppc64le",
            Self::Powerpc64LeManylinux236 => "ppc64le",
            Self::Powerpc64LeManylinux237 => "ppc64le",
            Self::Powerpc64LeManylinux238 => "ppc64le",
            Self::Powerpc64LeManylinux239 => "ppc64le",
            Self::Powerpc64LeManylinux240 => "ppc64le",
            Self::LoongArch64Manylinux236 => "loongarch64",
            Self::LoongArch64Manylinux237 => "loongarch64",
            Self::LoongArch64Manylinux238 => "loongarch64",
            Self::LoongArch64Manylinux239 => "loongarch64",
            Self::LoongArch64Manylinux240 => "loongarch64",
            Self::Aarch64LinuxAndroid => "aarch64",
            Self::X8664LinuxAndroid => "x86_64",
            Self::Wasm32Pyodide2024 => "wasm32",
            Self::Wasm32Pyodide2025 => "wasm32",
            Self::Arm64Ios => "arm64",
            Self::Arm64IosSimulator => "arm64",
            Self::X8664IosSimulator => "x86_64",
        }
    }

    /// Return the target's operating system and known release.
    fn os(self) -> TargetOs {
        match self {
            Self::Windows
            | Self::X8664PcWindowsMsvc
            | Self::Aarch64PcWindowsMsvc
            | Self::I686PcWindowsMsvc => TargetOs::Windows,
            Self::Linux
            | Self::X8664UnknownLinuxGnu
            | Self::Aarch64UnknownLinuxGnu
            | Self::Aarch64UnknownLinuxMusl
            | Self::X8664UnknownLinuxMusl
            | Self::Riscv64UnknownLinuxGnu
            | Self::S390XUnknownLinuxGnu
            | Self::Powerpc64LeUnknownLinuxGnu
            | Self::LoongArch64UnknownLinuxGnu
            | Self::X8664Manylinux2014
            | Self::X8664Manylinux217
            | Self::X8664Manylinux228
            | Self::X8664Manylinux231
            | Self::X8664Manylinux232
            | Self::X8664Manylinux233
            | Self::X8664Manylinux234
            | Self::X8664Manylinux235
            | Self::X8664Manylinux236
            | Self::X8664Manylinux237
            | Self::X8664Manylinux238
            | Self::X8664Manylinux239
            | Self::X8664Manylinux240
            | Self::Aarch64Manylinux2014
            | Self::Aarch64Manylinux217
            | Self::Aarch64Manylinux228
            | Self::Aarch64Manylinux231
            | Self::Aarch64Manylinux232
            | Self::Aarch64Manylinux233
            | Self::Aarch64Manylinux234
            | Self::Aarch64Manylinux235
            | Self::Aarch64Manylinux236
            | Self::Aarch64Manylinux237
            | Self::Aarch64Manylinux238
            | Self::Aarch64Manylinux239
            | Self::Aarch64Manylinux240
            | Self::S390XManylinux2014
            | Self::S390XManylinux217
            | Self::S390XManylinux228
            | Self::S390XManylinux231
            | Self::S390XManylinux232
            | Self::S390XManylinux233
            | Self::S390XManylinux234
            | Self::S390XManylinux235
            | Self::S390XManylinux236
            | Self::S390XManylinux237
            | Self::S390XManylinux238
            | Self::S390XManylinux239
            | Self::S390XManylinux240
            | Self::Powerpc64LeManylinux2014
            | Self::Powerpc64LeManylinux217
            | Self::Powerpc64LeManylinux228
            | Self::Powerpc64LeManylinux231
            | Self::Powerpc64LeManylinux232
            | Self::Powerpc64LeManylinux233
            | Self::Powerpc64LeManylinux234
            | Self::Powerpc64LeManylinux235
            | Self::Powerpc64LeManylinux236
            | Self::Powerpc64LeManylinux237
            | Self::Powerpc64LeManylinux238
            | Self::Powerpc64LeManylinux239
            | Self::Powerpc64LeManylinux240
            | Self::LoongArch64Manylinux236
            | Self::LoongArch64Manylinux237
            | Self::LoongArch64Manylinux238
            | Self::LoongArch64Manylinux239
            | Self::LoongArch64Manylinux240 => TargetOs::Linux,
            Self::Macos | Self::Aarch64AppleDarwin | Self::X8664AppleDarwin => TargetOs::Macos,
            Self::Aarch64LinuxAndroid | Self::X8664LinuxAndroid => TargetOs::Android,
            // Emscripten compiler version: https://pyodide.org/en/stable/development/abi/312.html
            Self::Wasm32Pyodide2024 => TargetOs::Emscripten { release: "3.1.58" },
            // Emscripten compiler version: https://pyodide.org/en/stable/development/abi/313.html
            Self::Wasm32Pyodide2025 => TargetOs::Emscripten { release: "4.0.9" },
            Self::Arm64Ios | Self::Arm64IosSimulator | Self::X8664IosSimulator => TargetOs::Ios,
        }
    }

    /// Return `true` if the platform is compatible with manylinux.
    pub fn manylinux_compatible(self) -> bool {
        match self.os() {
            TargetOs::Linux => true,
            TargetOs::Windows
            | TargetOs::Macos
            | TargetOs::Android
            | TargetOs::Emscripten { .. }
            | TargetOs::Ios => false,
        }
    }

    /// Return a [`MarkerEnvironment`] compatible with the given [`TargetTriple`], based on
    /// a base [`MarkerEnvironment`].
    ///
    /// The returned [`MarkerEnvironment`] will preserve the base environment's Python version
    /// markers, but override its platform markers.
    pub fn markers(self, base: MarkerEnvironment) -> MarkerEnvironment {
        let os = self.os();
        let (os_name, platform_system, sys_platform) = match &os {
            TargetOs::Windows => ("nt", "Windows", "win32"),
            TargetOs::Linux => ("posix", "Linux", "linux"),
            TargetOs::Macos => ("posix", "Darwin", "darwin"),
            TargetOs::Android => ("posix", "Android", "android"),
            TargetOs::Emscripten { .. } => ("posix", "Emscripten", "emscripten"),
            TargetOs::Ios => ("posix", "iOS", "ios"),
        };
        let (platform_release, platform_version) = os.platform_release_and_version();
        base.with_os_name(os_name)
            .with_platform_machine(self.platform_machine())
            .with_platform_system(platform_system)
            .with_sys_platform(sys_platform)
            .with_platform_release(platform_release)
            .with_platform_version(platform_version)
    }
}

/// Return the macOS deployment target as parsed from the environment.
fn macos_deployment_target() -> Option<(u16, u16)> {
    let version = std::env::var(EnvVars::MACOSX_DEPLOYMENT_TARGET).ok()?;
    let mut parts = version.split('.');

    // Parse the major version (e.g., `12` in `12.0`).
    let major = parts.next()?.parse::<u16>().ok()?;

    // Parse the minor version (e.g., `0` in `12.0`), with a default of `0`.
    let minor = parts.next().unwrap_or("0").parse::<u16>().ok()?;

    Some((major, minor))
}

/// Return the iOS deployment target as parsed from the environment.
fn ios_deployment_target() -> Option<(u16, u16)> {
    let version = std::env::var(EnvVars::IPHONEOS_DEPLOYMENT_TARGET).ok()?;
    let mut parts = version.split('.');

    // Parse the major version (e.g., `12` in `12.0`).
    let major = parts.next()?.parse::<u16>().ok()?;

    // Parse the minor version (e.g., `0` in `12.0`), with a default of `0`.
    let minor = parts.next().unwrap_or("0").parse::<u16>().ok()?;

    Some((major, minor))
}

/// Return the Android API level as parsed from the environment.
fn android_api_level() -> Option<u16> {
    let api_level_str = std::env::var(EnvVars::ANDROID_API_LEVEL).ok()?;

    // Parse the api level.
    let api_level = api_level_str.parse::<u16>().ok()?;

    Some(api_level)
}
