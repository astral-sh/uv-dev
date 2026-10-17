use anyhow::bail;
use std::path::PathBuf;
use tracing::debug;
use uv_fs::Simplified;
use uv_warnings::warn_user;
/// Read dotenv files into an overlay for a spawned process.
///
/// These values intentionally do not mutate uv's process environment and cannot mutate
/// the current uv process' settings.
pub fn read_env_files(env_file: &[PathBuf]) -> anyhow::Result<Vec<(String, String)>> {
    let mut environment = Vec::new();

    for env_file_path in env_file.iter().rev().map(PathBuf::as_path) {
        let iter = match dotenvy::from_path_iter(env_file_path) {
            Err(dotenvy::Error::Io(err)) if err.kind() == std::io::ErrorKind::NotFound => {
                bail!(
                    "No environment file found at: {}",
                    env_file_path.simplified_display()
                );
            }
            Err(dotenvy::Error::Io(err)) => {
                bail!(
                    "Failed to read environment file `{}`: {err}",
                    env_file_path.simplified_display()
                );
            }
            Err(dotenvy::Error::LineParse(content, position)) => {
                warn_user!(
                    "Failed to parse environment file `{}` at position {position}: {content}",
                    env_file_path.simplified_display(),
                );
                continue;
            }
            Err(err) => {
                warn_user!(
                    "Failed to parse environment file `{}`: {err}",
                    env_file_path.simplified_display(),
                );
                continue;
            }
            Ok(iter) => iter,
        };

        let mut parsed = true;
        for item in iter {
            match item {
                Ok((key, value)) => {
                    if std::env::var(&key).is_err() {
                        environment.push((key, value));
                    }
                }
                Err(dotenvy::Error::Io(err)) => {
                    bail!(
                        "Failed to read environment file `{}`: {err}",
                        env_file_path.simplified_display()
                    );
                }
                Err(dotenvy::Error::LineParse(content, position)) => {
                    warn_user!(
                        "Failed to parse environment file `{}` at position {position}: {content}",
                        env_file_path.simplified_display(),
                    );
                    parsed = false;
                    break;
                }
                Err(err) => {
                    warn_user!(
                        "Failed to parse environment file `{}`: {err}",
                        env_file_path.simplified_display(),
                    );
                    parsed = false;
                    break;
                }
            }
        }

        if parsed {
            debug!(
                "Read environment file at: {}",
                env_file_path.simplified_display()
            );
        }
    }

    // `dotenvy::from_path` preserves the first loaded value, while `Command::envs` preserves the
    // last value set for the child process.
    environment.reverse();

    Ok(environment)
}
