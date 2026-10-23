//! Print and check a bounded wheel-tag compatibility and preference matrix.
//!
//! Run with `cargo run -p uv-platform-tags --example compatibility_matrix`.

use std::error::Error;
use std::io::{self, Write};

use uv_platform_tags::{
    AbiTag, Arch, CompressedTags, LanguageTag, Os, Platform, PlatformTag, TagCompatibility, Tags,
    TagsOptions,
};

struct Artifact {
    name: &'static str,
    python: LanguageTag,
    abi: AbiTag,
    platform: PlatformTag,
}

impl Artifact {
    fn new(
        name: &'static str,
        python: &str,
        abi: &str,
        platform: &str,
    ) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            name,
            python: python.parse()?,
            abi: abi.parse()?,
            platform: platform.parse()?,
        })
    }

    fn compatibility(&self, tags: &Tags) -> TagCompatibility {
        tags.compatibility(CompressedTags {
            python_tags: std::slice::from_ref(&self.python),
            abi_tags: std::slice::from_ref(&self.abi),
            platform_tags: std::slice::from_ref(&self.platform),
        })
    }
}

struct Environment {
    name: &'static str,
    os: Os,
    arch: Arch,
    python: (u8, u8),
    freethreaded: bool,
    compatible: &'static [&'static str],
    preferred: &'static str,
}

fn main() -> Result<(), Box<dyn Error>> {
    let artifacts = [
        Artifact::new("pure", "py3", "none", "any")?,
        Artifact::new("glibc217", "cp311", "cp311", "manylinux_2_17_x86_64")?,
        Artifact::new("glibc228", "cp311", "cp311", "manylinux_2_28_x86_64")?,
        Artifact::new("stable", "cp311", "abi3", "manylinux_2_17_x86_64")?,
        Artifact::new("threaded", "cp313", "cp313t", "manylinux_2_17_x86_64")?,
        Artifact::new("musl", "cp311", "cp311", "musllinux_1_2_x86_64")?,
        Artifact::new("arm", "cp311", "cp311", "manylinux_2_17_aarch64")?,
        Artifact::new("mac", "cp311", "cp311", "macosx_11_0_arm64")?,
        Artifact::new("windows", "cp311", "cp311", "win_amd64")?,
    ];
    let environments = [
        Environment {
            name: "cp311-glibc217-x64",
            os: Os::Manylinux {
                major: 2,
                minor: 17,
            },
            arch: Arch::X86_64,
            python: (3, 11),
            freethreaded: false,
            compatible: &["pure", "glibc217", "stable"],
            preferred: "glibc217",
        },
        Environment {
            name: "cp311-glibc228-x64",
            os: Os::Manylinux {
                major: 2,
                minor: 28,
            },
            arch: Arch::X86_64,
            python: (3, 11),
            freethreaded: false,
            compatible: &["pure", "glibc217", "glibc228", "stable"],
            preferred: "glibc228",
        },
        Environment {
            name: "cp312-glibc228-x64",
            os: Os::Manylinux {
                major: 2,
                minor: 28,
            },
            arch: Arch::X86_64,
            python: (3, 12),
            freethreaded: false,
            compatible: &["pure", "stable"],
            preferred: "stable",
        },
        Environment {
            name: "cp313t-glibc228-x64",
            os: Os::Manylinux {
                major: 2,
                minor: 28,
            },
            arch: Arch::X86_64,
            python: (3, 13),
            freethreaded: true,
            compatible: &["pure", "threaded"],
            preferred: "threaded",
        },
        Environment {
            name: "cp311-musl12-x64",
            os: Os::Musllinux { major: 1, minor: 2 },
            arch: Arch::X86_64,
            python: (3, 11),
            freethreaded: false,
            compatible: &["pure", "musl"],
            preferred: "musl",
        },
        Environment {
            name: "cp311-glibc217-arm64",
            os: Os::Manylinux {
                major: 2,
                minor: 17,
            },
            arch: Arch::Aarch64,
            python: (3, 11),
            freethreaded: false,
            compatible: &["pure", "arm"],
            preferred: "arm",
        },
        Environment {
            name: "cp311-macos11-arm64",
            os: Os::Macos {
                major: 11,
                minor: 0,
            },
            arch: Arch::Aarch64,
            python: (3, 11),
            freethreaded: false,
            compatible: &["pure", "mac"],
            preferred: "mac",
        },
        Environment {
            name: "cp311-windows-x64",
            os: Os::Windows,
            arch: Arch::X86_64,
            python: (3, 11),
            freethreaded: false,
            compatible: &["pure", "windows"],
            preferred: "windows",
        },
    ];

    let mut output = io::stdout().lock();
    writeln!(output, "environment\tartifact\tcompatibility\tpreferred")?;
    for environment in environments {
        let tags = Tags::from_env(
            Platform::new(environment.os, environment.arch),
            environment.python,
            "cpython",
            environment.python,
            TagsOptions {
                manylinux_compatible: true,
                gil_disabled: environment.freethreaded,
                is_cross: true,
                ..TagsOptions::default()
            },
        )?;
        let mut preferred = None;
        for artifact in &artifacts {
            let compatibility = artifact.compatibility(&tags);
            let compatible = match compatibility {
                TagCompatibility::Compatible(_) => true,
                TagCompatibility::Incompatible(_) => false,
            };
            if compatible != environment.compatible.contains(&artifact.name) {
                return Err(format!(
                    "unexpected compatibility for {} / {}: {compatibility:?}",
                    environment.name, artifact.name
                )
                .into());
            }
            if compatible && preferred.is_none_or(|(_, priority)| compatibility > priority) {
                preferred = Some((artifact.name, compatibility));
            }
            writeln!(
                output,
                "{}\t{}\t{compatibility:?}\t{}",
                environment.name,
                artifact.name,
                artifact.name == environment.preferred
            )?;
        }
        if preferred.map(|(name, _)| name) != Some(environment.preferred) {
            return Err(format!("unexpected artifact preference for {}", environment.name).into());
        }
    }
    Ok(())
}
