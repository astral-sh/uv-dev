use std::borrow::Cow;
use std::fs::Metadata;
use std::io::{BufWriter, Write};
use std::path::Path;

use either::Either;

/// A multicasting writer that writes to both the standard output and an output file, if present.
pub struct OutputWriter<'a> {
    stdout: Option<Either<anstream::Stdout, BufWriter<anstream::Stdout>>>,
    output_file: Option<&'a Path>,
    buffer: Vec<u8>,
}

impl<'a> OutputWriter<'a> {
    /// Create a new output writer.
    pub fn new(include_stdout: bool, output_file: Option<&'a Path>) -> Self {
        let stdout = include_stdout.then(|| {
            let stdout = anstream::stdout();
            if stdout.is_terminal() {
                Either::Left(stdout)
            } else {
                Either::Right(BufWriter::new(stdout))
            }
        });
        Self {
            stdout,
            output_file,
            buffer: Vec::new(),
        }
    }

    /// Flush standard output and commit the buffer to the output file.
    pub async fn commit(mut self) -> std::io::Result<()> {
        self.flush()?;

        if let Some(output_file) = self.output_file {
            let stream = anstream::adapter::strip_bytes(&self.buffer).into_vec();

            // Devices and pipes must be opened directly: an atomic rename would replace them
            // with a regular file. Follow symlinks when checking the destination's file type.
            let metadata = fs_err::symlink_metadata(output_file);
            let is_symlink = metadata.as_ref().is_ok_and(Metadata::is_symlink);
            let metadata = if is_symlink {
                fs_err::metadata(output_file)
            } else {
                metadata
            };
            match metadata {
                Ok(metadata) if !metadata.is_file() => {
                    return fs_err::tokio::write(output_file, &stream).await;
                }
                Ok(_) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(err),
            }

            if let Some(parent_dir) = output_file.parent() {
                fs_err::create_dir_all(parent_dir)?;
            }

            // If the output file is an existing symlink, write to the destination instead.
            let output_file = if is_symlink {
                Cow::Owned(output_file.with_file_name(fs_err::read_link(output_file)?))
            } else {
                Cow::Borrowed(output_file)
            };
            uv_fs::write_atomic(output_file, &stream).await?;
        }
        Ok(())
    }
}

impl Write for OutputWriter<'_> {
    /// Write to both standard output and the output buffer, if present.
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // Write to the buffer.
        if self.output_file.is_some() {
            self.buffer.write_all(buf)?;
        }

        // Write to standard output.
        if let Some(stdout) = &mut self.stdout {
            stdout.write_all(buf)?;
        }

        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if let Some(stdout) = &mut self.stdout {
            stdout.flush()?;
        }
        Ok(())
    }
}
