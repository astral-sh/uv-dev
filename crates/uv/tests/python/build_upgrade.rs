use std::path::PathBuf;

use anyhow::Result;
use filetime::FileTime;
use uv_python_managed::platform_key_from_env;
use uv_test::{LATEST_PYTHON_3_12, TestContext};

pub(super) struct OutdatedPythonBuild {
    build_file: PathBuf,
    original_build: String,
    executable: PathBuf,
    marked_mtime: FileTime,
}

impl OutdatedPythonBuild {
    /// Mark the installed Python 3.12 build as old and record the interpreter to replace.
    pub(super) fn python_3_12(context: &TestContext) -> Result<Self> {
        let installation = context.temp_dir.join("managed").join(format!(
            "cpython-{LATEST_PYTHON_3_12}-{}",
            platform_key_from_env()?
        ));
        let build_file = installation.join("BUILD");
        let original_build = fs_err::read_to_string(&build_file)?;
        fs_err::write(&build_file, "19000101")?;

        let executable = if cfg!(windows) {
            installation.join("python.exe")
        } else {
            installation.join("bin").join("python3.12")
        };
        // A build upgrade keeps the same Python version and installation key. Mark the executable
        // to observe whether the interpreter itself is replaced.
        filetime::set_file_mtime(&executable, FileTime::from_unix_time(1_700_000_000, 0))?;
        let marked_mtime = FileTime::from_last_modification_time(&fs_err::metadata(&executable)?);

        Ok(Self {
            build_file,
            original_build,
            executable,
            marked_mtime,
        })
    }

    pub(super) fn assert_replaced(self) -> Result<()> {
        assert_ne!(
            FileTime::from_last_modification_time(&fs_err::metadata(&self.executable)?),
            self.marked_mtime,
            "interpreter was not replaced: {}",
            self.executable.display()
        );
        assert_eq!(
            fs_err::read_to_string(&self.build_file)?,
            self.original_build,
            "build metadata was not restored: {}",
            self.build_file.display()
        );
        Ok(())
    }
}
