//! Fetch real package metadata over a fresh, latency-controlled TLS connection.

mod common;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use criterion::{
    BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime,
};
use rustls::pki_types::PrivatePkcs8KeyDer;
use rustls::{ServerConfig, ServerConnection, StreamOwned};
use uv_bench::{fixture_path, is_codspeed_simulation};
use uv_client::{BaseClient, BaseClientBuilder, Certificates};
use uv_redacted::DisplaySafeUrl;

struct TlsServer {
    address: SocketAddr,
    certificates: Certificates,
    stop: Arc<AtomicBool>,
    flights: Arc<AtomicUsize>,
    thread: Option<JoinHandle<()>>,
}

impl TlsServer {
    fn start(body: &[u8]) -> Self {
        let certificate = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()])
            .expect("Failed to create loopback certificate");
        let file = tempfile::NamedTempFile::new().expect("Failed to create certificate file");
        fs_err::write(file.path(), certificate.cert.pem()).expect("Failed to write certificate");
        let certificates =
            Certificates::from_file(file.path()).expect("Invalid loopback certificate");
        let mut provider = rustls::crypto::aws_lc_rs::default_provider();
        // Model an index that requires hybrid post-quantum key exchange. A client that does not
        // include this key share in its first flight incurs a HelloRetryRequest round trip.
        provider.kx_groups = vec![rustls::crypto::aws_lc_rs::kx_group::X25519MLKEM768];
        let config = ServerConfig::builder_with_provider(Arc::new(provider))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .expect("Unsupported TLS version")
            .with_no_client_auth()
            .with_single_cert(
                vec![certificate.cert.der().clone()],
                PrivatePkcs8KeyDer::from(certificate.signing_key.serialize_der()).into(),
            )
            .expect("Invalid server certificate");
        let config = Arc::new(config);
        let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind TLS server");
        let address = listener.local_addr().expect("Missing server address");
        let stop = Arc::new(AtomicBool::new(false));
        let server_stop = Arc::clone(&stop);
        let flights = Arc::new(AtomicUsize::new(0));
        let server_flights = Arc::clone(&flights);
        let mut response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        ).into_bytes();
        response.extend_from_slice(body);
        let thread = thread::spawn(move || {
            for socket in listener.incoming() {
                let mut socket = socket.expect("Failed to accept TLS connection");
                if server_stop.load(Ordering::Relaxed) {
                    break;
                }
                socket
                    .set_nodelay(true)
                    .expect("Failed to configure TCP connection");
                socket
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .expect("Failed to set read timeout");
                let mut connection = ServerConnection::new(Arc::clone(&config))
                    .expect("Failed to create TLS connection");
                while connection.is_handshaking() {
                    connection
                        .read_tls(&mut socket)
                        .expect("Failed to read TLS handshake");
                    connection
                        .process_new_packets()
                        .expect("Invalid TLS handshake");
                    if connection.wants_write() && connection.is_handshaking() {
                        server_flights.fetch_add(1, Ordering::Relaxed);
                        thread::sleep(Duration::from_millis(20));
                    }
                    while connection.wants_write() {
                        connection
                            .write_tls(&mut socket)
                            .expect("Failed to write TLS handshake");
                    }
                }
                let mut stream = StreamOwned::new(connection, socket);
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let count = stream
                        .read(&mut buffer)
                        .expect("Failed to read HTTP request");
                    assert_ne!(count, 0, "Incomplete HTTP request");
                    request.extend_from_slice(&buffer[..count]);
                    assert!(request.len() <= 16 * 1024, "Oversized HTTP request");
                }
                stream
                    .write_all(&response)
                    .expect("Failed to write metadata response");
                stream.conn.send_close_notify();
                stream.flush().expect("Failed to finish metadata response");
            }
        });
        Self {
            address,
            certificates,
            stop,
            flights,
            thread: Some(thread),
        }
    }

    fn client(&self) -> BaseClient {
        BaseClientBuilder::default()
            .custom_certificates(self.certificates.clone())
            .retries(0)
            .build()
            .expect("Failed to create uv HTTP client")
    }

    fn url(&self, project: &str) -> DisplaySafeUrl {
        DisplaySafeUrl::parse(&format!(
            "https://localhost:{}/files/{project}.whl.metadata",
            self.address.port()
        ))
        .expect("Invalid metadata URL")
    }
}

impl Drop for TlsServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        TcpStream::connect(self.address).expect("Failed to wake TLS server");
        self.thread
            .take()
            .expect("Missing TLS server thread")
            .join()
            .expect("TLS server failed");
    }
}

async fn fetch_metadata(client: &BaseClient, url: &DisplaySafeUrl) -> Vec<u8> {
    client
        .for_host(url)
        .get(url.as_str())
        .send()
        .await
        .expect("Metadata request failed")
        .error_for_status()
        .expect("Metadata request returned an error")
        .bytes()
        .await
        .expect("Failed to read metadata response")
        .to_vec()
}

fn tls_handshake(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create Tokio runtime");
    let mut group = c.benchmark_group("tls_handshake");
    for project in ["flask", "jupyterlab", "airflow"] {
        let body = fs_err::read(fixture_path(&format!("{project}.metadata")))
            .expect("Failed to read core metadata");
        let server = TlsServer::start(&body);
        let url = server.url(project);
        assert_eq!(
            runtime.block_on(fetch_metadata(&server.client(), &url)),
            body
        );
        let preferred = rustls::crypto::aws_lc_rs::default_provider().kx_groups[0].name();
        let expected_flights = if preferred == rustls::NamedGroup::X25519MLKEM768 {
            1
        } else {
            2
        };
        assert_eq!(server.flights.load(Ordering::Relaxed), expected_flights);
        group.bench_function(BenchmarkId::new("metadata", project), |b| {
            b.iter_batched(
                || server.client(),
                |client| runtime.block_on(fetch_metadata(&client, &url)),
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();
}

criterion_group! {
    name = handshake;
    config = common::walltime_criterion();
    targets = tls_handshake
}
criterion_main!(handshake);
