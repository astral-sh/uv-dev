use std::error::Error as _;
use std::str::FromStr;

use anyhow::Result;
use assert_fs::prelude::*;
use uv_distribution_filename::WheelFilename;
use uv_install_wheel::{Error, InstallState, Layout, LinkMode, install_wheel};
use uv_pypi_types::{MetadataError, Scheme};

#[test]
fn wheel_metadata_failure_keeps_source() -> Result<()> {
    for (metadata, missing) in [("Version: 1.0\n", "Name"), ("Name: demo\n", "Version")] {
        let wheel = assert_fs::TempDir::new()?;
        let dist_info = wheel.child("demo-1.0.dist-info");
        dist_info
            .child("WHEEL")
            .write_str("Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n")?;
        dist_info.child("METADATA").write_str(metadata)?;
        let destination = assert_fs::TempDir::new()?;
        let layout = Layout {
            sys_executable: destination.path().join("python"),
            python_version: (3, 12),
            os_name: "posix".to_string(),
            scheme: Scheme {
                purelib: destination.path().join("site-packages"),
                platlib: destination.path().join("site-packages"),
                scripts: destination.path().join("bin"),
                data: destination.path().to_path_buf(),
                include: destination.path().join("include"),
            },
        };
        let error = install_wheel::<(), ()>(
            &layout,
            false,
            wheel.path(),
            &WheelFilename::from_str("demo-1.0-py3-none-any.whl")?,
            None,
            None,
            None,
            None,
            false,
            LinkMode::Copy,
            &InstallState::default(),
        )
        .expect_err("incomplete metadata must fail installation");
        assert!(matches!(&error, Error::InvalidMetadata(_)));
        assert!(error.is_user_failure());
        assert!(
            matches!(
                error.source().and_then(|error| error.downcast_ref::<MetadataError>()),
                Some(MetadataError::FieldNotFound(field)) if *field == missing
            ),
            "error: {error:?}; source: {:?}",
            error.source()
        );
        assert!(!layout.scheme.purelib.exists());
    }
    Ok(())
}
