use std::path::{Path, PathBuf};

use data_encoding::BASE64URL_NOPAD;
use rustc_hash::FxHashMap;
use sha2::{Digest, Sha256};
use tracing::debug;

use uv_fs::{Simplified, normalize_path, relative_to};

use crate::{Error, Layout, RecordEntry};

/// Make plain `.pth` paths within an editable source tree relative to their installed location.
///
/// Editable backends can also use executable `.pth` files, import hooks, or link trees. Those
/// mechanisms may contain other absolute paths and are not covered by this transformation.
pub(crate) fn relocate_editable(
    layout: &Layout,
    site_packages: &Path,
    source: &Path,
    record: &mut [RecordEntry],
) -> Result<(), Error> {
    let source = normalize_path(source);
    let purelib = normalize_path(&layout.scheme.purelib);
    let platlib = normalize_path(&layout.scheme.platlib);
    let mut rewritten: FxHashMap<PathBuf, (String, u64)> = FxHashMap::default();

    // The wheel's data directories have already been installed, so `RECORD` also contains the
    // final paths of `.pth` files installed through `.data/purelib` or `.data/platlib`.
    for entry in record {
        let path = normalize_path(site_packages.join(&entry.path)).into_owned();
        if path.extension().is_none_or(|extension| extension != "pth") {
            continue;
        }
        let Some(parent) = path.parent() else {
            continue;
        };
        if parent.simplified() != purelib.simplified()
            && parent.simplified() != platlib.simplified()
        {
            continue;
        }
        // Root files and `.data` files can resolve to the same installed path. Every `RECORD`
        // entry for that path must describe the final contents.
        if let Some((hash, size)) = rewritten.get(&path) {
            entry.hash = Some(hash.clone());
            entry.size = Some(*size);
            continue;
        }

        let content = fs_err::read(&path)?;
        let Ok(content) = str::from_utf8(&content) else {
            continue;
        };
        let Some(content) = relative_paths(content, &source, parent) else {
            continue;
        };

        debug!("Relocating editable paths in {}", path.user_display());
        // Replacing the file atomically breaks any hardlink or symlink to the cached wheel.
        uv_fs::write_atomic_sync(&path, content.as_bytes())?;
        let hash = format!(
            "sha256={}",
            BASE64URL_NOPAD.encode(&Sha256::digest(content.as_bytes()))
        );
        let size = content.len() as u64;
        entry.hash = Some(hash.clone());
        entry.size = Some(size);
        rewritten.insert(path, (hash, size));
    }

    Ok(())
}

/// Relativize source paths using Python's `.pth` line semantics.
fn relative_paths(content: &str, source: &Path, site_packages: &Path) -> Option<String> {
    // Python versions differ in their handling of a UTF-8 BOM and Unicode line separators.
    // Leave those files intact instead of changing where their path or executable lines begin.
    if content.starts_with('\u{feff}')
        || content.chars().any(|character| {
            matches!(
                character,
                '\u{000b}' | '\u{000c}' | '\u{001c}'
                    ..='\u{001f}' | '\u{0085}' | '\u{2028}' | '\u{2029}'
            )
        })
    {
        return None;
    }

    // A backend's executable `.pth` may depend on the rest of the file or install its own import
    // hook. Only plain path files have semantics that can be adjusted independently.
    if content
        .split(['\n', '\r'])
        .any(|line| line.starts_with("import ") || line.starts_with("import\t"))
    {
        return None;
    }

    let mut relocated = String::with_capacity(content.len());
    let mut changed = false;
    for line in content.split_inclusive(['\n', '\r']) {
        // Python strips trailing whitespace, but leading whitespace is part of the path.
        let value = line.trim_end();
        let path = Path::new(value);
        if path.is_absolute() {
            let path = normalize_path(path);
            if path.simplified().starts_with(source.simplified())
                && let Ok(relative) = relative_to(&path, site_packages)
                && let Some(relative) = relative.to_str()
            {
                // An explicit `./` keeps names such as `#package`, `import helper`, or a leading
                // BOM from being interpreted as `.pth` syntax.
                relocated.push_str("./");
                relocated.push_str(relative);
                relocated.push_str(&line[value.len()..]);
                changed = true;
                continue;
            }
        }
        relocated.push_str(line);
    }

    changed.then_some(relocated)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use anyhow::Result;
    use assert_fs::prelude::*;
    use uv_fs::{Simplified, relative_to};
    use uv_pypi_types::Scheme;

    use super::{relative_paths, relocate_editable};
    use crate::{Layout, RecordEntry};

    #[test]
    fn distinct_site_packages() -> Result<()> {
        let root = assert_fs::TempDir::new()?;
        let source = root.child("source");
        let purelib = root.child("purelib");
        let platlib = root.child("platform/site-packages");
        let content = format!("{}\n", source.path().join("src").display());
        purelib.child("project.pth").write_str(&content)?;
        platlib.child("project.pth").write_str(&content)?;
        purelib.child("nested/project.pth").write_str(&content)?;

        let layout = Layout {
            sys_executable: root.path().join("bin/python"),
            python_version: (3, 12),
            os_name: "posix".to_string(),
            scheme: Scheme {
                purelib: purelib.path().to_path_buf(),
                platlib: platlib.path().to_path_buf(),
                scripts: root.path().join("bin"),
                data: root.path().to_path_buf(),
                include: root.path().join("include"),
            },
        };
        let mut record = [
            purelib.child("project.pth"),
            platlib.child("project.pth"),
            purelib.child("nested/project.pth"),
            purelib.child("project.pth"),
        ]
        .iter()
        .map(|path| {
            Ok(RecordEntry {
                path: relative_to(path.path(), purelib.path())?
                    .portable_display()
                    .to_string(),
                hash: None,
                size: None,
            })
        })
        .collect::<Result<Vec<_>>>()?;

        relocate_editable(&layout, purelib.path(), source.path(), &mut record)?;

        for (directory, index) in [(&purelib, 0), (&platlib, 1)] {
            let expected = format!(
                "./{}\n",
                relative_to(source.path().join("src"), directory.path())?.display()
            );
            directory.child("project.pth").assert(expected.as_str());
            assert!(record[index].hash.is_some());
            assert_eq!(record[index].size, Some(expected.len() as u64));
        }
        purelib.child("nested/project.pth").assert(content.as_str());
        assert_eq!(record[2].hash, None);
        assert_eq!(record[2].size, None);
        assert_eq!(record[3].hash, record[0].hash);
        assert_eq!(record[3].size, record[0].size);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn plain_path_lines() {
        let source = Path::new("/project");
        let site_packages = Path::new("/project/.venv/lib/python3.12/site-packages");
        let content = "# comment\r\n/project/src \t\r/project/other\n/project/../outside\n /project/src\nrelative\n";
        assert_eq!(
            relative_paths(content, source, site_packages).as_deref(),
            Some(
                "# comment\r\n./../../../../src \t\r./../../../../other\n/project/../outside\n /project/src\nrelative\n"
            )
        );
        assert_eq!(
            relative_paths("/project/src\rimport\tcustom_hook\n", source, site_packages),
            None
        );
        for content in [
            "\u{feff}/project/src\n",
            "/project/src\u{2028}../../outside\n",
            "/project/src\u{2029}import custom_hook\n",
        ] {
            assert_eq!(relative_paths(content, source, site_packages), None);
        }
        assert_eq!(
            relative_paths(
                "/project/import helper/src\n/project/#package\n/project/\u{feff}package\n",
                source,
                source,
            )
            .as_deref(),
            Some("./import helper/src\n./#package\n./\u{feff}package\n")
        );
    }

    #[cfg(windows)]
    #[test]
    fn different_drives() {
        assert_eq!(
            relative_paths(
                "C:\\project\\src\n",
                Path::new("C:\\project"),
                Path::new("D:\\venv\\Lib\\site-packages"),
            ),
            None
        );
    }
}
