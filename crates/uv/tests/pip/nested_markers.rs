use anyhow::Result;
use assert_fs::prelude::*;
use async_zip::base::write::ZipFileWriter;
use async_zip::{Compression, ZipEntryBuilder};

use uv_static::EnvVars;
use uv_test::uv_snapshot;

#[tokio::test]
async fn deeply_nested_wheel_metadata_markers() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let metadata = format!(
        "Metadata-Version: 2.3\nName: demo\nVersion: 1.0\nRequires-Dist: unused; {}python_version < '0'{}\n",
        "(".repeat(100_000),
        ")".repeat(100_000),
    );
    let mut writer = ZipFileWriter::new(Vec::new());
    for (name, contents) in [
        ("demo-1.0.dist-info/METADATA", metadata.as_str()),
        (
            "demo-1.0.dist-info/WHEEL",
            "Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n",
        ),
        (
            "demo-1.0.dist-info/RECORD",
            "demo-1.0.dist-info/METADATA,,\ndemo-1.0.dist-info/WHEEL,,\ndemo-1.0.dist-info/RECORD,,\n",
        ),
    ] {
        writer
            .write_entry_whole(
                ZipEntryBuilder::new(name.into(), Compression::Stored),
                contents.as_bytes(),
            )
            .await?;
    }
    context
        .temp_dir
        .child("demo-1.0-py3-none-any.whl")
        .write_binary(&writer.close().await?)?;
    context
        .temp_dir
        .child("requirements.in")
        .write_str("demo")?;

    // Exercise the metadata parser in the CLI subprocess with its normal thread stack size.
    uv_snapshot!(context.filters(), context.pip_compile()
        .arg("requirements.in")
        .arg("--no-index")
        .arg("--find-links")
        .arg(context.temp_dir.path())
        .arg("--no-header")
        .arg("--no-annotate")
        .env_remove(EnvVars::UV_STACK_SIZE)
        .env_remove(EnvVars::RUST_MIN_STACK), @"
    exit_code: 0 (success)
    ----- stdout -----
    demo==1.0

    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    Ok(())
}
