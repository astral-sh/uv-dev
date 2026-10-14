use anyhow::Result;
use async_zip::base::write::ZipFileWriter;
use async_zip::{Compression, ZipEntryBuilder};
use futures::executor::block_on;
use indoc::indoc;

/// Archive a workspace whose dependency has both a workspace source and a registry candidate.
pub(super) fn archive() -> Result<Vec<u8>> {
    let files = [
        (
            "pyproject.toml",
            indoc! {r#"
                [project]
                name = "workspace"
                version = "0.1.0"
                requires-python = ">=3.12"
                dependencies = ["anyio"]

                [build-system]
                requires = ["uv_build>=0.7,<10000"]
                build-backend = "uv_build"

                [tool.uv.workspace]
                members = ["anyio"]

                [tool.uv.sources]
                anyio = { workspace = true }
            "#},
        ),
        ("src/workspace/__init__.py", ""),
        (
            "anyio/pyproject.toml",
            indoc! {r#"
                [project]
                name = "anyio"
                version = "0.1.0"
                requires-python = ">=3.12"

                [build-system]
                requires = ["uv_build>=0.7,<10000"]
                build-backend = "uv_build"
            "#},
        ),
        ("anyio/src/anyio/__init__.py", ""),
        // The consumer must resolve independently of the embedded workspace lockfile.
        (
            "uv.lock",
            indoc! {r#"
                version = 1
                requires-python = ">=3.12"

                [[package]]
                name = "anyio"
                version = "0.1.0"
                source = { editable = "anyio" }

                [[package]]
                name = "workspace"
                version = "0.1.0"
                source = { editable = "." }
                dependencies = [{ name = "anyio" }]
            "#},
        ),
    ];

    let mut archive = ZipFileWriter::new(Vec::new());
    for (path, contents) in files {
        let entry = ZipEntryBuilder::new(format!("workspace/{path}").into(), Compression::Stored);
        block_on(archive.write_entry_whole(entry, contents.as_bytes()))?;
    }
    Ok(block_on(archive.close())?)
}
