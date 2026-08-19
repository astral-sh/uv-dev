#[cfg(unix)]
use fs_err::os::unix::fs::OpenOptionsExt;
use std::io::{BufReader, Read, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use clap::{Parser, Subcommand};
use fs_err::{File, OpenOptions};
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::Ed25519KeyPair;
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use url::Url;
use uv_checksum_authority::{ArtifactId, AuthorityPublicKey, ChecksumRecord, Sha256Digest};
use uv_checksum_authority_service::{AuthorityService, Catalog};
use zeroize::Zeroizing;

#[derive(Parser)]
#[command(about = "Signed checksum authority for uv")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new signing seed and print its public verification key.
    Keygen {
        #[arg(long)]
        signing_key: PathBuf,
    },
    /// Admit a local archive, refusing to replace an existing checksum.
    Add {
        #[arg(long)]
        catalog: PathBuf,
        #[arg(long)]
        source: Url,
        #[arg(long)]
        filename: Option<String>,
        artifact: PathBuf,
    },
    /// Serve an immutable snapshot of a catalog. Use a TLS reverse proxy for remote access.
    Serve {
        #[arg(long)]
        catalog: PathBuf,
        #[arg(long)]
        signing_key: PathBuf,
        #[arg(long, default_value = "127.0.0.1:8080")]
        bind: SocketAddr,
    },
}

fn read_catalog(path: &Path) -> Result<Catalog> {
    let records: Vec<ChecksumRecord> = serde_json::from_reader(BufReader::new(File::open(path)?))?;
    Catalog::from_records(records)
}

fn read_key(path: &Path) -> Result<Ed25519KeyPair> {
    let encoded = Zeroizing::new(fs_err::read_to_string(path)?);
    let mut seed = Zeroizing::new([0; 32]);
    hex::decode_to_slice(encoded.trim(), seed.as_mut()).context("Invalid signing seed")?;
    Ed25519KeyPair::from_seed_unchecked(seed.as_ref()).map_err(|_| anyhow!("Invalid signing seed"))
}

#[tokio::main]
async fn main() -> Result<()> {
    match Args::parse().command {
        Command::Keygen { signing_key } => {
            let mut seed = Zeroizing::new([0; 32]);
            SystemRandom::new()
                .fill(seed.as_mut())
                .map_err(|_| anyhow!("Failed to generate signing seed"))?;
            let key = Ed25519KeyPair::from_seed_unchecked(seed.as_ref())
                .map_err(|_| anyhow!("Invalid signing seed"))?;
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            options.mode(0o600);
            let encoded = Zeroizing::new(hex::encode(seed.as_ref()));
            let mut file = options.open(signing_key)?;
            writeln!(file, "{}", encoded.as_str())?;
            file.sync_all()?;
            writeln!(
                std::io::stdout(),
                "{}",
                AuthorityPublicKey::from_signing_key(&key)
            )?;
        }
        Command::Add {
            catalog,
            source,
            filename,
            artifact,
        } => {
            let filename = filename
                .or_else(|| artifact.file_name()?.to_str().map(str::to_owned))
                .context("Pass --filename for an archive without a UTF-8 filename")?;
            let mut reader = BufReader::new(File::open(artifact)?);
            let mut hasher = Sha256::new();
            let mut archive_size = 0;
            let mut buffer = vec![0; 64 * 1024];
            loop {
                let size = reader.read(&mut buffer)?;
                if size == 0 {
                    break;
                }
                hasher.update(&buffer[..size]);
                archive_size += size as u64;
            }
            // Serialize cooperating writers across the atomic replacement of the catalog.
            let mut lock_path = catalog.as_os_str().to_owned();
            lock_path.push(".lock");
            let lock = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(PathBuf::from(lock_path))?;
            lock.lock()?;
            let mut records = if catalog.try_exists()? {
                read_catalog(&catalog)?
            } else {
                Catalog::default()
            };
            records.insert(ChecksumRecord::new(
                ArtifactId::new(&source, &filename)?,
                Sha256Digest::from_bytes(hasher.finalize().into()),
                archive_size,
            ))?;
            let parent = catalog
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
            serde_json::to_writer_pretty(&mut temporary, &records.records().collect::<Vec<_>>())?;
            writeln!(temporary)?;
            temporary.as_file().sync_all()?;
            temporary.persist(catalog)?;
        }
        Command::Serve {
            catalog,
            signing_key,
            bind,
        } => {
            let service = AuthorityService::new(read_catalog(&catalog)?, &read_key(&signing_key)?)?;
            let listener = TcpListener::bind(bind).await?;
            writeln!(
                std::io::stderr(),
                "Checksum authority listening on {}",
                listener.local_addr()?
            )?;
            writeln!(std::io::stderr(), "Public key: {}", service.public_key())?;
            service
                .serve(listener, async {
                    let _ = tokio::signal::ctrl_c().await;
                })
                .await?;
        }
    }
    Ok(())
}
