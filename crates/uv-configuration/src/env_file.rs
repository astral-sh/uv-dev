use std::path::PathBuf;

/// A collection of `.env` file paths.
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct EnvFile(Vec<PathBuf>);

impl EnvFile {
    /// Resolve the env file paths from command-line arguments or the environment.
    pub fn from_args(
        env_file: Vec<PathBuf>,
        env_file_environment: Option<String>,
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
        let mut current = String::new();
        let mut escape = false;
        let mut characters = env_file_environment.chars().peekable();
        while let Some(c) = characters.next() {
            if escape {
                if !c.is_whitespace() && c != '\\' {
                    current.push('\\');
                }
                current.push(c);
                escape = false;
            } else if c == '\\' {
                if current.is_empty() && characters.peek() == Some(&'\\') {
                    // Preserve the leading `\\` in UNC and extended-length Windows paths.
                    current.push('\\');
                    current.push('\\');
                    characters.next();
                } else {
                    escape = true;
                }
            } else if c.is_whitespace() {
                if !current.is_empty() {
                    paths.push(PathBuf::from(current));
                    current = String::new();
                }
            } else {
                current.push(c);
            }
        }
        if escape {
            current.push('\\');
        }
        if !current.is_empty() {
            paths.push(PathBuf::from(current));
        }

        Self(paths)
    }

    /// Iterate over the paths in the env file.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &PathBuf> {
        self.0.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_args_default() {
        let env_file = EnvFile::from_args(vec![], None, false);
        assert_eq!(env_file, EnvFile::default());
    }

    #[test]
    fn test_from_args_no_env_file() {
        let env_file = EnvFile::from_args(vec![], Some("path1 path2".to_string()), true);
        assert_eq!(env_file, EnvFile::default());
    }

    #[test]
    fn test_from_args_empty_string() {
        let env_file = EnvFile::from_args(vec![], Some(String::new()), false);
        assert_eq!(env_file, EnvFile::default());
    }

    #[test]
    fn test_from_args_whitespace_only() {
        let env_file = EnvFile::from_args(vec![], Some("   ".to_string()), false);
        assert_eq!(env_file, EnvFile::default());
    }

    #[test]
    fn test_from_args_single_path() {
        let env_file = EnvFile::from_args(vec![], Some("path1".to_string()), false);
        assert_eq!(env_file.0, vec![PathBuf::from("path1")]);
    }

    #[test]
    fn test_from_args_multiple_paths() {
        let env_file = EnvFile::from_args(vec![], Some("path1 path2 path3".to_string()), false);
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
        let env_file = EnvFile::from_args(vec![], Some(r"path\ with\ spaces".to_string()), false);
        assert_eq!(env_file.0, vec![PathBuf::from("path with spaces")]);
    }

    #[test]
    fn test_from_args_mixed_escaped_and_normal() {
        let env_file = EnvFile::from_args(
            vec![],
            Some(r"path1 path\ with\ spaces path2".to_string()),
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
        let env_file =
            EnvFile::from_args(vec![], Some(r"path\\with\\backslashes".to_string()), false);
        assert_eq!(env_file.0, vec![PathBuf::from(r"path\with\backslashes")]);
    }

    #[test]
    fn test_from_args_windows_paths() {
        let env_file = EnvFile::from_args(
            vec![],
            Some(r"C:\work\.env D:\other\.env".to_string()),
            false,
        );
        assert_eq!(
            env_file.0,
            vec![
                PathBuf::from(r"C:\work\.env"),
                PathBuf::from(r"D:\other\.env")
            ]
        );
    }

    #[test]
    fn test_from_args_windows_unc_and_extended_paths() {
        let env_file = EnvFile::from_args(
            vec![],
            Some(r"\\server\share\.env \\?\C:\work\.env \\?\UNC\server\share\.env".to_string()),
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
    fn test_from_args_windows_unc_and_extended_paths_with_escaped_spaces() {
        let env_file = EnvFile::from_args(
            vec![],
            Some(r"\\server\share\path\ with\ spaces\.env \\?\C:\other\ path\.env".to_string()),
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
            Some("ignored".to_string()),
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
            Some("ignored".to_string()),
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
    fn test_iter() {
        let env_file = EnvFile(vec![PathBuf::from("path1"), PathBuf::from("path2")]);
        let paths: Vec<_> = env_file.iter().collect();
        assert_eq!(
            paths,
            vec![&PathBuf::from("path1"), &PathBuf::from("path2")]
        );
    }
}
