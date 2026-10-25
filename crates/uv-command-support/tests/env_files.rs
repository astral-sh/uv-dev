use std::error::Error;
use std::io;

use uv_command_support::child::{EnvFileError, read_env_files};

#[test]
fn missing_environment_file() -> io::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("missing.env");
    let error =
        read_env_files(std::slice::from_ref(&path)).expect_err("environment loading should fail");
    assert!(matches!(&error, EnvFileError::Missing(missing) if missing == &path));
    assert!(error.source().is_none());
    Ok(())
}

#[test]
fn environment_file_read_error_keeps_source() -> io::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().to_path_buf();
    let error =
        read_env_files(std::slice::from_ref(&path)).expect_err("environment loading should fail");
    let EnvFileError::Read {
        path: failed_path,
        source,
    } = &error
    else {
        return Err(io::Error::other(format!(
            "expected a read failure: {error}"
        )));
    };
    assert_eq!(failed_path, &path);
    assert_eq!(
        error
            .source()
            .and_then(|error| error.downcast_ref::<io::Error>())
            .map(io::Error::kind),
        Some(source.kind())
    );
    assert_ne!(source.kind(), io::ErrorKind::NotFound);
    Ok(())
}
