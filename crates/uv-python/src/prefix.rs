use std::path::{Path, PathBuf};

use same_file::is_same_file;

use uv_pypi_types::Scheme;

/// A `--prefix` directory into which packages can be installed, separate from a virtual environment
/// or system Python interpreter.
#[derive(Debug, Clone)]
pub struct Prefix(PathBuf);

impl Prefix {
    /// Return the [`Scheme`] for the `--prefix` directory.
    pub(crate) fn scheme(&self, virtualenv: &Scheme) -> Scheme {
        Scheme {
            purelib: self.0.join(&virtualenv.purelib),
            platlib: self.0.join(&virtualenv.platlib),
            scripts: self.0.join(&virtualenv.scripts),
            data: self.0.join(&virtualenv.data),
            include: self.0.join(&virtualenv.include),
        }
    }

    /// Return an iterator over the `site-packages` directories inside the environment.
    pub(crate) fn site_packages(&self, virtualenv: &Scheme) -> impl Iterator<Item = PathBuf> {
        let purelib = self.0.join(&virtualenv.purelib);
        let platlib = self.0.join(&virtualenv.platlib);
        let distinct = purelib != platlib && !is_same_file(&purelib, &platlib).unwrap_or(false);
        std::iter::once(purelib).chain(distinct.then_some(platlib))
    }

    /// Initialize the `--prefix` directory.
    pub(crate) fn init(&self, virtualenv: &Scheme) -> std::io::Result<()> {
        for site_packages in self.site_packages(virtualenv) {
            fs_err::create_dir_all(site_packages)?;
        }
        Ok(())
    }

    /// Return the path to the `--prefix` directory.
    pub fn root(&self) -> &Path {
        &self.0
    }
}

impl From<PathBuf> for Prefix {
    fn from(path: PathBuf) -> Self {
        Self(path)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use anyhow::Result;
    use uv_pypi_types::Scheme;

    use super::Prefix;

    #[test]
    fn split_prefix_site_packages() -> Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let root = temp_dir.path().join("prefix");
        let prefix = Prefix::from(root.clone());
        let virtualenv = Scheme {
            purelib: "lib/python3.12/site-packages".into(),
            platlib: "lib64/python3.12/site-packages".into(),
            scripts: "bin".into(),
            data: PathBuf::new(),
            include: "include".into(),
        };

        assert_eq!(
            prefix.site_packages(&virtualenv).collect::<Vec<_>>(),
            [
                root.join("lib/python3.12/site-packages"),
                root.join("lib64/python3.12/site-packages")
            ]
        );

        prefix.init(&virtualenv)?;
        assert!(root.join("lib/python3.12/site-packages").is_dir());
        assert!(root.join("lib64/python3.12/site-packages").is_dir());

        let combined = Scheme {
            platlib: virtualenv.purelib.clone(),
            ..virtualenv
        };
        assert_eq!(
            prefix.site_packages(&combined).collect::<Vec<_>>(),
            [root.join("lib/python3.12/site-packages")]
        );

        Ok(())
    }
}
