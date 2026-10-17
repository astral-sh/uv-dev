//! A read-only HTTP service backed by an explicitly admitted, in-memory checksum catalog.

use std::collections::{BTreeMap, btree_map::Entry};
use std::convert::Infallible;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use http::{Method, Request, Response, StatusCode};
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::{TokioIo, TokioTimer};
use ring::signature::Ed25519KeyPair;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use url::form_urlencoded;
use uv_checksum_authority::{ArtifactId, AuthorityPublicKey, ChecksumRecord, SignedRecord};

const MAX_CONNECTIONS: usize = 128;
const MAX_REQUEST_URI: usize = 8 * 1024;
const HEADER_TIMEOUT: Duration = Duration::from_secs(10);

/// An immutable catalog. Duplicate identical records are harmless; conflicting records are errors.
#[derive(Debug, Default)]
pub struct Catalog(BTreeMap<ArtifactId, ChecksumRecord>);

impl Catalog {
    pub fn from_records(records: impl IntoIterator<Item = ChecksumRecord>) -> Result<Self> {
        let mut catalog = Self::default();
        for record in records {
            catalog.insert(record)?;
        }
        Ok(catalog)
    }

    pub fn insert(&mut self, record: ChecksumRecord) -> Result<()> {
        match self.0.entry(record.artifact().clone()) {
            Entry::Occupied(existing) if existing.get() != &record => {
                bail!(
                    "Conflicting checksum for `{}`; existing records cannot be replaced",
                    record.artifact().filename()
                );
            }
            Entry::Occupied(_) => {}
            Entry::Vacant(entry) => {
                entry.insert(record);
            }
        }
        Ok(())
    }

    pub fn records(&self) -> impl Iterator<Item = &ChecksumRecord> {
        self.0.values()
    }
}

/// The same service implementation is used by the executable and in-memory integration tests.
pub struct AuthorityService {
    records: BTreeMap<ArtifactId, Bytes>,
    public_key: AuthorityPublicKey,
}

impl AuthorityService {
    pub fn new(catalog: Catalog, key: &Ed25519KeyPair) -> Result<Self> {
        let records = catalog
            .0
            .into_values()
            .map(|record| {
                let signed = serde_json::to_vec(&SignedRecord::sign(&record, key)?)?;
                Ok((record.artifact().clone(), Bytes::from(signed)))
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            records,
            public_key: AuthorityPublicKey::from_signing_key(key),
        })
    }

    pub fn public_key(&self) -> AuthorityPublicKey {
        self.public_key
    }

    fn handle(&self, request: &Request<Incoming>) -> Response<Full<Bytes>> {
        if request
            .uri()
            .path_and_query()
            .is_some_and(|uri| uri.as_str().len() > MAX_REQUEST_URI)
        {
            return response(StatusCode::URI_TOO_LONG, "request URI too long");
        }
        if request.method() != Method::GET {
            return response(StatusCode::METHOD_NOT_ALLOWED, "GET required");
        }
        if request.uri().path() == "/health" {
            return response(StatusCode::OK, "ok");
        }
        if request.uri().path() != "/v1/checksum" {
            return response(StatusCode::NOT_FOUND, "not found");
        }
        let mut source = None;
        let mut filename = None;
        for (key, value) in
            form_urlencoded::parse(request.uri().query().unwrap_or_default().as_bytes())
        {
            match key.as_ref() {
                "source" if source.is_none() => source = Some(value.into_owned()),
                "filename" if filename.is_none() => filename = Some(value.into_owned()),
                _ => return response(StatusCode::BAD_REQUEST, "invalid query"),
            }
        }
        let (Some(source), Some(filename)) = (source, filename) else {
            return response(StatusCode::BAD_REQUEST, "source and filename required");
        };
        let Ok(artifact) = ArtifactId::from_canonical(&source, &filename) else {
            return response(StatusCode::BAD_REQUEST, "invalid artifact identity");
        };
        let Some(record) = self.records.get(&artifact) else {
            return response(StatusCode::NOT_FOUND, "unknown artifact");
        };
        let mut response = response(StatusCode::OK, record.clone());
        response.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("application/json"),
        );
        response
    }

    pub async fn serve(
        self,
        listener: TcpListener,
        shutdown: impl Future<Output = ()>,
    ) -> Result<()> {
        let service = Arc::new(self);
        let mut connections = JoinSet::new();
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                () = &mut shutdown => break,
                connection = listener.accept(), if connections.len() < MAX_CONNECTIONS => {
                    let (stream, _) = connection.context("Failed to accept checksum authority connection")?;
                    let service = Arc::clone(&service);
                    connections.spawn(async move {
                        http1::Builder::new()
                            .timer(TokioTimer::new())
                            .header_read_timeout(HEADER_TIMEOUT)
                            .max_headers(32)
                            .max_buf_size(16 * 1024)
                            .keep_alive(false)
                            .serve_connection(TokioIo::new(stream), service_fn(move |request| {
                            let response = service.handle(&request);
                            async move { Ok::<_, Infallible>(response) }
                        })).await
                    });
                }
                Some(_) = connections.join_next(), if !connections.is_empty() => {}
            }
        }
        connections.abort_all();
        while connections.join_next().await.is_some() {}
        Ok(())
    }
}

fn response(status: StatusCode, body: impl Into<Bytes>) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(body.into()));
    *response.status_mut() = status;
    response
}
