use std::io;
use std::path::Path;

/// Files recorded by a legacy `installed-files.txt` metadata file.
pub(crate) struct InstalledFiles {
    pub(crate) paths: Vec<String>,
}

impl InstalledFiles {
    pub(crate) fn read(path: impl AsRef<Path>) -> io::Result<Option<Self>> {
        let contents = match fs_err::read_to_string(path) {
            Ok(contents) => contents,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err),
        };

        Ok(Some(Self {
            paths: contents
                .lines()
                .filter(|line| !line.is_empty())
                .map(ToString::to_string)
                .collect(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use assert_fs::prelude::*;

    use super::InstalledFiles;

    #[test]
    fn parse_installed_files() {
        let temp_dir = assert_fs::TempDir::new().unwrap();
        let installed_files = temp_dir.child("installed-files.txt");
        installed_files
            .write_str("\n../package/file-with-space.py \r\n../../bin/package\n\n")
            .unwrap();

        let installed_files = InstalledFiles::read(installed_files.path())
            .unwrap()
            .unwrap();

        assert_eq!(
            installed_files.paths,
            ["../package/file-with-space.py ", "../../bin/package"]
        );
    }

    #[test]
    fn missing_installed_files() {
        let temp_dir = assert_fs::TempDir::new().unwrap();

        assert!(
            InstalledFiles::read(temp_dir.child("installed-files.txt").path())
                .unwrap()
                .is_none()
        );
    }
}
