use std::ffi::{OsStr, OsString};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(windows)]
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;

/// A collection of `.env` file paths.
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct EnvFile(Vec<PathBuf>);

impl EnvFile {
    /// Resolve the env file paths from command-line arguments or the environment.
    pub fn from_args(
        env_file: Vec<PathBuf>,
        env_file_environment: Option<OsString>,
        no_env_file: bool,
    ) -> Self {
        if no_env_file {
            return Self::default();
        }

        if !env_file.is_empty() {
            return Self(env_file);
        }

        let Some(env_file_environment) = env_file_environment else {
            return Self::default();
        };

        let mut paths = Vec::new();

        // Split the environment variable on whitespace, while preserving literal backslashes in
        // paths and allowing whitespace or a backslash to be escaped.
        let mut current = OsString::new();
        let mut escape = false;
        let mut characters = native_characters(&env_file_environment).peekable();
        while let Some(character) = characters.next() {
            let whitespace = character.is_ok_and(char::is_whitespace);
            if escape {
                if !whitespace && character != Ok('\\') {
                    current.push("\\");
                }
                push_character(&mut current, character);
                escape = false;
            } else if character == Ok('\\') {
                if cfg!(windows) && current.is_empty() && characters.peek() == Some(&Ok('\\')) {
                    let mut count = 1;
                    while characters.peek() == Some(&Ok('\\')) {
                        characters.next();
                        count += 1;
                    }
                    // UNC prefixes can be literal or already escaped. Four leading backslashes
                    // encode two; two literal backslashes must also remain a UNC prefix.
                    current.push("\\".repeat(if count < 4 { 2 } else { count / 2 }));
                    escape = count % 2 != 0;
                } else {
                    escape = true;
                }
            } else if whitespace {
                if !current.is_empty() {
                    paths.push(PathBuf::from(std::mem::take(&mut current)));
                }
            } else {
                push_character(&mut current, character);
            }
        }
        if escape {
            current.push("\\");
        }
        if !current.is_empty() {
            paths.push(PathBuf::from(current));
        }

        Self(paths)
    }

    /// Return the paths to the environment files in their configured order.
    pub fn as_slice(&self) -> &[PathBuf] {
        &self.0
    }
}

#[cfg(unix)]
type NativeCharacter = Result<char, u8>;
#[cfg(windows)]
type NativeCharacter = Result<char, u16>;

#[cfg(unix)]
fn native_characters(value: &OsStr) -> impl Iterator<Item = NativeCharacter> {
    value.as_bytes().utf8_chunks().flat_map(|chunk| {
        chunk
            .valid()
            .chars()
            .map(Ok)
            .chain(chunk.invalid().iter().copied().map(Err))
    })
}

#[cfg(windows)]
fn native_characters(value: &OsStr) -> impl Iterator<Item = NativeCharacter> {
    char::decode_utf16(value.encode_wide())
        .map(|character| character.map_err(|error| error.unpaired_surrogate()))
}

fn push_character(target: &mut OsString, character: NativeCharacter) {
    match character {
        Ok(character) => {
            let mut buffer = [0; 4];
            target.push(character.encode_utf8(&mut buffer));
        }
        Err(unit) => {
            #[cfg(unix)]
            target.push(OsStr::from_bytes(&[unit]));
            #[cfg(windows)]
            target.push(OsString::from_wide(&[unit]));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt;

    #[test]
    #[cfg(unix)]
    fn test_from_args_escaped_leading_backslash() {
        let env_file = EnvFile::from_args(vec![], Some(r"\\config.env".into()), false);
        assert_eq!(env_file.0, vec![PathBuf::from(r"\config.env")]);
    }

    #[test]
    fn test_from_args_default() {
        let env_file = EnvFile::from_args(vec![], None, false);
        assert_eq!(env_file, EnvFile::default());
    }

    #[test]
    fn test_from_args_no_env_file() {
        let env_file = EnvFile::from_args(vec![], Some("path1 path2".into()), true);
        assert_eq!(env_file, EnvFile::default());
    }

    #[test]
    fn test_from_args_empty_string() {
        let env_file = EnvFile::from_args(vec![], Some(OsString::new()), false);
        assert_eq!(env_file, EnvFile::default());
    }

    #[test]
    fn test_from_args_whitespace_only() {
        let env_file = EnvFile::from_args(vec![], Some("   ".into()), false);
        assert_eq!(env_file, EnvFile::default());
    }

    #[test]
    fn test_from_args_single_path() {
        let env_file = EnvFile::from_args(vec![], Some("path1".into()), false);
        assert_eq!(env_file.0, vec![PathBuf::from("path1")]);
    }

    #[test]
    fn test_from_args_multiple_paths() {
        let env_file = EnvFile::from_args(vec![], Some("path1 path2 path3".into()), false);
        assert_eq!(
            env_file.0,
            vec![
                PathBuf::from("path1"),
                PathBuf::from("path2"),
                PathBuf::from("path3")
            ]
        );
    }

    #[test]
    fn test_from_args_escaped_spaces() {
        let env_file = EnvFile::from_args(vec![], Some(r"path\ with\ spaces".into()), false);
        assert_eq!(env_file.0, vec![PathBuf::from("path with spaces")]);
    }

    #[test]
    fn test_from_args_mixed_escaped_and_normal() {
        let env_file = EnvFile::from_args(
            vec![],
            Some(r"path1 path\ with\ spaces path2".into()),
            false,
        );
        assert_eq!(
            env_file.0,
            vec![
                PathBuf::from("path1"),
                PathBuf::from("path with spaces"),
                PathBuf::from("path2")
            ]
        );
    }

    #[test]
    fn test_from_args_escaped_backslash() {
        let env_file = EnvFile::from_args(vec![], Some(r"path\\with\\backslashes".into()), false);
        assert_eq!(env_file.0, vec![PathBuf::from(r"path\with\backslashes")]);
    }

    #[test]
    fn test_from_args_windows_paths() {
        let env_file =
            EnvFile::from_args(vec![], Some(r"C:\work\.env D:\other\.env".into()), false);
        assert_eq!(
            env_file.0,
            vec![
                PathBuf::from(r"C:\work\.env"),
                PathBuf::from(r"D:\other\.env")
            ]
        );
    }

    #[test]
    #[cfg(windows)]
    fn test_from_args_windows_unc_and_extended_paths() {
        let env_file = EnvFile::from_args(
            vec![],
            Some(r"\\server\share\.env \\?\C:\work\.env \\?\UNC\server\share\.env".into()),
            false,
        );
        assert_eq!(
            env_file.0,
            vec![
                PathBuf::from(r"\\server\share\.env"),
                PathBuf::from(r"\\?\C:\work\.env"),
                PathBuf::from(r"\\?\UNC\server\share\.env")
            ]
        );
    }

    #[test]
    #[cfg(windows)]
    fn test_from_args_windows_unc_and_extended_paths_with_escaped_spaces() {
        let env_file = EnvFile::from_args(
            vec![],
            Some(r"\\server\share\path\ with\ spaces\.env \\?\C:\other\ path\.env".into()),
            false,
        );
        assert_eq!(
            env_file.0,
            vec![
                PathBuf::from(r"\\server\share\path with spaces\.env"),
                PathBuf::from(r"\\?\C:\other path\.env")
            ]
        );
    }

    #[test]
    fn test_from_args_cli_path_with_spaces() {
        let env_file = EnvFile::from_args(
            vec![PathBuf::from("path with spaces")],
            Some("ignored".into()),
            false,
        );
        assert_eq!(env_file.0, vec![PathBuf::from("path with spaces")]);
    }

    #[test]
    fn test_from_args_cli_windows_paths_with_spaces() {
        let env_file = EnvFile::from_args(
            vec![
                PathBuf::from(r"\\server\share\path with spaces\.env"),
                PathBuf::from(r"\\?\C:\other path\.env"),
            ],
            Some("ignored".into()),
            false,
        );
        assert_eq!(
            env_file.0,
            vec![
                PathBuf::from(r"\\server\share\path with spaces\.env"),
                PathBuf::from(r"\\?\C:\other path\.env")
            ]
        );
    }

    #[test]
    fn test_from_args_escaped_unc_prefixes() {
        let env_file = EnvFile::from_args(
            vec![],
            Some(r"\\\\server\\share\\.env \\\\?\\C:\\work\\.env".into()),
            false,
        );
        assert_eq!(
            env_file.0,
            [
                PathBuf::from(r"\\server\share\.env"),
                PathBuf::from(r"\\?\C:\work\.env")
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_from_args_non_unicode_paths() {
        let paths =
            OsString::from_vec(b"first-\xff.env\xc2\xa0path\\ with\\ spaces-\xfe.env".to_vec());
        let env_file = EnvFile::from_args(vec![], Some(paths), false);
        assert_eq!(
            env_file.0,
            [
                PathBuf::from(OsString::from_vec(b"first-\xff.env".to_vec())),
                PathBuf::from(OsString::from_vec(b"path with spaces-\xfe.env".to_vec())),
            ]
        );
    }

    #[cfg(windows)]
    #[test]
    fn test_from_args_unpaired_surrogates() {
        let paths = OsString::from_wide(&[0x61, 0xd800, 0x20, 0x62, 0xdc00]);
        let env_file = EnvFile::from_args(vec![], Some(paths), false);
        assert_eq!(
            env_file.0,
            [
                PathBuf::from(OsString::from_wide(&[0x61, 0xd800])),
                PathBuf::from(OsString::from_wide(&[0x62, 0xdc00])),
            ]
        );
    }

    #[test]
    fn test_as_slice() {
        let env_file = EnvFile(vec![PathBuf::from("path1"), PathBuf::from("path2")]);
        let paths = env_file.as_slice();
        assert_eq!(paths, [PathBuf::from("path1"), PathBuf::from("path2")]);
    }
}
