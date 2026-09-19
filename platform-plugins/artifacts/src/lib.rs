//! Immutable artifact object storage and malware-scanner adapters.
//!
//! Metadata, lifecycle state, authorization and signed download locators live in
//! other platform boundaries. This crate only owns object bytes and scanning.

mod streaming;

use std::{env, fmt, sync::Arc, time::Duration};

use async_trait::async_trait;
use futures_util::StreamExt;
use masonwing_kernel::runtime::{ArtifactObjects, CommandFailure, FailureKind};
use object_store::{
    Attribute, Attributes, Error as ObjectStoreError, ObjectStore, ObjectStoreExt, PutMode,
    PutOptions,
    aws::{AmazonS3Builder, S3ConditionalPut, S3CopyIfNotExists},
    path::Path,
};
use tokio::{io::AsyncReadExt, net::TcpStream};

const STORE_UNAVAILABLE: &str = "ARTIFACT_STORE_UNAVAILABLE";
const SCANNER_UNAVAILABLE: &str = "ARTIFACT_SCANNER_UNAVAILABLE";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactConfigError {
    pub code: &'static str,
}

impl fmt::Display for ArtifactConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code)
    }
}

impl std::error::Error for ArtifactConfigError {}

/// S3-compatible store configuration. Credentials are intentionally private
/// and its Debug implementation always redacts them.
#[derive(Clone)]
pub struct S3Config {
    endpoint: String,
    bucket: String,
    region: String,
    access_key_id: String,
    secret_access_key: String,
    allow_http: bool,
}

impl fmt::Debug for S3Config {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("S3Config")
            .field("endpoint", &self.endpoint)
            .field("bucket", &self.bucket)
            .field("region", &self.region)
            .field("access_key_id", &"<redacted>")
            .field("secret_access_key", &"<redacted>")
            .field("allow_http", &self.allow_http)
            .finish()
    }
}

impl S3Config {
    pub fn new(
        endpoint: impl Into<String>,
        bucket: impl Into<String>,
        region: impl Into<String>,
        access_key_id: impl Into<String>,
        secret_access_key: impl Into<String>,
        allow_http: bool,
    ) -> Result<Self, ArtifactConfigError> {
        let config = Self {
            endpoint: endpoint.into(),
            bucket: bucket.into(),
            region: region.into(),
            access_key_id: access_key_id.into(),
            secret_access_key: secret_access_key.into(),
            allow_http,
        };
        config.validate()?;
        Ok(config)
    }

    /// Read the runtime boundary without ever returning or logging credential
    /// values. Local MinIO may use the synthetic `MINIO_ROOT_*` pair; deployed
    /// environments should provide standard AWS credential variables.
    pub fn from_env() -> Result<Self, ArtifactConfigError> {
        let endpoint = required_env("S3_ENDPOINT")?;
        let bucket = required_env("S3_BUCKET")?;
        let region = env::var("AWS_REGION")
            .or_else(|_| env::var("AWS_DEFAULT_REGION"))
            .unwrap_or_else(|_| "us-east-1".to_owned());
        let access_key_id = env::var("AWS_ACCESS_KEY_ID")
            .or_else(|_| env::var("MINIO_ROOT_USER"))
            .map_err(|_| ArtifactConfigError {
                code: "ARTIFACT_S3_CREDENTIALS_MISSING",
            })?;
        let secret_access_key = env::var("AWS_SECRET_ACCESS_KEY")
            .or_else(|_| env::var("MINIO_ROOT_PASSWORD"))
            .map_err(|_| ArtifactConfigError {
                code: "ARTIFACT_S3_CREDENTIALS_MISSING",
            })?;
        let local =
            env::var("MASONWING_ENV").is_ok_and(|value| value.eq_ignore_ascii_case("LOCAL"));
        let allow_http = endpoint.starts_with("http://") && local;
        Self::new(
            endpoint,
            bucket,
            region,
            access_key_id,
            secret_access_key,
            allow_http,
        )
    }

    fn validate(&self) -> Result<(), ArtifactConfigError> {
        if self.endpoint.trim().is_empty()
            || self.bucket.trim().is_empty()
            || self.region.trim().is_empty()
            || self.access_key_id.is_empty()
            || self.secret_access_key.is_empty()
        {
            return Err(ArtifactConfigError {
                code: "ARTIFACT_S3_CONFIG_INVALID",
            });
        }
        if self.endpoint.starts_with("http://") && !self.allow_http {
            return Err(ArtifactConfigError {
                code: "ARTIFACT_S3_INSECURE_ENDPOINT",
            });
        }
        if !(self.endpoint.starts_with("http://") || self.endpoint.starts_with("https://")) {
            return Err(ArtifactConfigError {
                code: "ARTIFACT_S3_ENDPOINT_INVALID",
            });
        }
        Ok(())
    }
}

fn required_env(name: &'static str) -> Result<String, ArtifactConfigError> {
    env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or(ArtifactConfigError {
            code: "ARTIFACT_S3_CONFIG_MISSING",
        })
}

/// Implements the exact host runtime `ArtifactObjects` boundary. `PutMode::Create`
/// is required so two writers racing on one key cannot turn immutability into a
/// head-then-put time-of-check/time-of-use bug.
#[derive(Clone)]
pub struct ArtifactStoreAdapter {
    store: Arc<dyn ObjectStore>,
}

impl fmt::Debug for ArtifactStoreAdapter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ArtifactStoreAdapter")
            .finish_non_exhaustive()
    }
}

impl ArtifactStoreAdapter {
    pub fn from_s3(config: S3Config) -> Result<Self, ArtifactConfigError> {
        let store = AmazonS3Builder::new()
            .with_endpoint(config.endpoint)
            .with_bucket_name(config.bucket)
            .with_region(config.region)
            .with_access_key_id(config.access_key_id)
            .with_secret_access_key(config.secret_access_key)
            .with_allow_http(config.allow_http)
            .with_virtual_hosted_style_request(false)
            .with_conditional_put(S3ConditionalPut::ETagMatch)
            .with_copy_if_not_exists(S3CopyIfNotExists::Multipart)
            .build()
            .map_err(|_| ArtifactConfigError {
                code: "ARTIFACT_S3_CONFIG_INVALID",
            })?;
        Ok(Self {
            store: Arc::new(store),
        })
    }

    pub fn from_object_store(store: Arc<dyn ObjectStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl ArtifactObjects for ArtifactStoreAdapter {
    async fn put_immutable(
        &self,
        key: &str,
        bytes: Vec<u8>,
        content_type: &str,
    ) -> Result<(), CommandFailure> {
        let path = checked_path(key)?;
        let mut attributes = Attributes::new();
        attributes.insert(
            Attribute::ContentType,
            checked_content_type(content_type)?.into(),
        );
        let options = PutOptions {
            mode: PutMode::Create,
            attributes,
            ..PutOptions::default()
        };
        self.store
            .put_opts(&path, bytes.into(), options)
            .await
            .map(|_| ())
            .map_err(map_put_error)
    }

    async fn get_bounded(&self, key: &str, max_bytes: usize) -> Result<Vec<u8>, CommandFailure> {
        let path = checked_path(key)?;
        let result = self.store.get(&path).await.map_err(map_get_error)?;
        if result.meta.size > max_bytes as u64 {
            return Err(CommandFailure::new(
                "ARTIFACT_READ_LIMIT_EXCEEDED",
                FailureKind::Exhausted,
            ));
        }

        let capacity = usize::try_from(result.meta.size)
            .unwrap_or(max_bytes)
            .min(max_bytes);
        let mut output = Vec::with_capacity(capacity);
        let mut stream = result.into_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(map_get_error)?;
            let next_len = output.len().checked_add(chunk.len()).ok_or_else(|| {
                CommandFailure::new("ARTIFACT_READ_LIMIT_EXCEEDED", FailureKind::Exhausted)
            })?;
            if next_len > max_bytes {
                return Err(CommandFailure::new(
                    "ARTIFACT_READ_LIMIT_EXCEEDED",
                    FailureKind::Exhausted,
                ));
            }
            output.extend_from_slice(&chunk);
        }
        Ok(output)
    }

    async fn put_reader(
        &self,
        key: &str,
        reader: &mut (dyn tokio::io::AsyncRead + Send + Unpin),
        size_bytes: u64,
        content_type: &str,
    ) -> Result<(), CommandFailure> {
        self.stream_put(key, reader, size_bytes, content_type).await
    }

    async fn open_reader(
        &self,
        key: &str,
        max_bytes: u64,
    ) -> Result<masonwing_kernel::runtime::ArtifactStream, CommandFailure> {
        self.stream_open(key, max_bytes).await
    }
}

fn checked_path(key: &str) -> Result<Path, CommandFailure> {
    if key.is_empty() {
        return Err(CommandFailure::invalid("ARTIFACT_OBJECT_KEY_INVALID"));
    }
    Path::parse(key).map_err(|_| CommandFailure::invalid("ARTIFACT_OBJECT_KEY_INVALID"))
}

fn checked_content_type(content_type: &str) -> Result<String, CommandFailure> {
    if content_type.is_empty()
        || content_type.len() > 255
        || !content_type.is_ascii()
        || content_type.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(CommandFailure::invalid("ARTIFACT_CONTENT_TYPE_INVALID"));
    }
    Ok(content_type.to_owned())
}

fn map_put_error(error: ObjectStoreError) -> CommandFailure {
    match error {
        ObjectStoreError::AlreadyExists { .. } | ObjectStoreError::Precondition { .. } => {
            CommandFailure::conflict("IMMUTABLE_ARTIFACT")
        }
        ObjectStoreError::NotSupported { .. } | ObjectStoreError::NotImplemented { .. } => {
            CommandFailure::unavailable("ARTIFACT_IMMUTABILITY_UNSUPPORTED")
        }
        _ => CommandFailure::unavailable(STORE_UNAVAILABLE),
    }
}

fn map_get_error(error: ObjectStoreError) -> CommandFailure {
    match error {
        ObjectStoreError::NotFound { .. } => CommandFailure::not_found(),
        _ => CommandFailure::unavailable(STORE_UNAVAILABLE),
    }
}

pub use masonwing_kernel::runtime::{ArtifactScanner, ScanVerdict};

#[derive(Clone, Copy, Debug, Default)]
pub struct FailClosedScanner;

#[async_trait]
impl ArtifactScanner for FailClosedScanner {
    async fn scan(&self, _bytes: &[u8]) -> Result<ScanVerdict, CommandFailure> {
        Err(CommandFailure::unavailable(SCANNER_UNAVAILABLE))
    }
}

#[derive(Clone)]
pub struct ClamAvScanner {
    address: String,
    connect_timeout: Duration,
    response_timeout: Duration,
    max_scan_bytes: usize,
    chunk_bytes: usize,
}

impl fmt::Debug for ClamAvScanner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClamAvScanner")
            .field("address", &self.address)
            .field("connect_timeout", &self.connect_timeout)
            .field("response_timeout", &self.response_timeout)
            .field("max_scan_bytes", &self.max_scan_bytes)
            .field("chunk_bytes", &self.chunk_bytes)
            .finish()
    }
}

impl ClamAvScanner {
    pub fn new(address: impl Into<String>) -> Result<Self, ArtifactConfigError> {
        let address = address.into();
        if address.trim().is_empty() {
            return Err(ArtifactConfigError {
                code: "ARTIFACT_SCANNER_CONFIG_INVALID",
            });
        }
        Ok(Self {
            address,
            connect_timeout: Duration::from_secs(3),
            response_timeout: Duration::from_secs(15),
            max_scan_bytes: 1024 * 1024 * 1024,
            chunk_bytes: 64 * 1024,
        })
    }

    pub fn with_limits(
        mut self,
        connect_timeout: Duration,
        response_timeout: Duration,
        max_scan_bytes: usize,
        chunk_bytes: usize,
    ) -> Result<Self, ArtifactConfigError> {
        if connect_timeout.is_zero()
            || response_timeout.is_zero()
            || max_scan_bytes == 0
            || chunk_bytes == 0
            || chunk_bytes > u32::MAX as usize
        {
            return Err(ArtifactConfigError {
                code: "ARTIFACT_SCANNER_CONFIG_INVALID",
            });
        }
        self.connect_timeout = connect_timeout;
        self.response_timeout = response_timeout;
        self.max_scan_bytes = max_scan_bytes;
        self.chunk_bytes = chunk_bytes;
        Ok(self)
    }
}

#[async_trait]
impl ArtifactScanner for ClamAvScanner {
    async fn scan(&self, bytes: &[u8]) -> Result<ScanVerdict, CommandFailure> {
        self.stream_scan(&mut std::io::Cursor::new(bytes), bytes.len() as u64)
            .await
    }

    async fn scan_reader(
        &self,
        reader: &mut (dyn tokio::io::AsyncRead + Send + Unpin),
        size_bytes: u64,
    ) -> Result<ScanVerdict, CommandFailure> {
        self.stream_scan(reader, size_bytes).await
    }
}

async fn read_clamd_response(socket: &mut TcpStream) -> Result<Vec<u8>, CommandFailure> {
    const MAX_RESPONSE: usize = 1024;
    let mut response = Vec::with_capacity(128);
    let mut byte = [0_u8; 1];
    loop {
        let count = socket
            .read(&mut byte)
            .await
            .map_err(|_| CommandFailure::unavailable(SCANNER_UNAVAILABLE))?;
        if count == 0 {
            break;
        }
        if byte[0] == 0 || byte[0] == b'\n' {
            break;
        }
        if response.len() == MAX_RESPONSE {
            return Err(CommandFailure::unavailable(SCANNER_UNAVAILABLE));
        }
        response.push(byte[0]);
    }
    if response.is_empty() {
        return Err(CommandFailure::unavailable(SCANNER_UNAVAILABLE));
    }
    Ok(response)
}

fn parse_clamd_response(response: &[u8]) -> Result<ScanVerdict, CommandFailure> {
    let response = std::str::from_utf8(response)
        .map_err(|_| CommandFailure::unavailable(SCANNER_UNAVAILABLE))?;
    let result = response
        .strip_prefix("stream: ")
        .ok_or_else(|| CommandFailure::unavailable(SCANNER_UNAVAILABLE))?;
    if result == "OK" {
        return Ok(ScanVerdict::Clean);
    }
    if let Some(signature) = result.strip_suffix(" FOUND") {
        return Ok(ScanVerdict::Quarantined {
            signature: sanitize_signature(signature),
        });
    }
    Err(CommandFailure::unavailable(SCANNER_UNAVAILABLE))
}

fn sanitize_signature(signature: &str) -> String {
    let sanitized: String = signature
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-' | ':')
        })
        .take(128)
        .collect();
    if sanitized.is_empty() {
        "malware".to_owned()
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use object_store::memory::InMemory;
    use tokio::{io::AsyncWriteExt, net::TcpListener};

    #[tokio::test]
    async fn immutable_put_rejects_overwrite_and_preserves_original_bytes() {
        let adapter = ArtifactStoreAdapter::from_object_store(Arc::new(InMemory::new()));
        adapter
            .put_immutable("tenant-a/artifact-1", b"first".to_vec(), "text/plain")
            .await
            .unwrap();
        let error = adapter
            .put_immutable("tenant-a/artifact-1", b"second".to_vec(), "text/plain")
            .await
            .unwrap_err();
        assert_eq!(error.code, "IMMUTABLE_ARTIFACT");
        assert_eq!(error.kind, FailureKind::Conflict);
        assert_eq!(
            adapter.get_bounded("tenant-a/artifact-1", 5).await.unwrap(),
            b"first"
        );
    }

    #[tokio::test]
    async fn bounded_read_denies_object_larger_than_limit() {
        let adapter = ArtifactStoreAdapter::from_object_store(Arc::new(InMemory::new()));
        adapter
            .put_immutable(
                "tenant-a/artifact-2",
                vec![7; 16],
                "application/octet-stream",
            )
            .await
            .unwrap();
        let error = adapter
            .get_bounded("tenant-a/artifact-2", 15)
            .await
            .unwrap_err();
        assert_eq!(error.code, "ARTIFACT_READ_LIMIT_EXCEEDED");
        assert_eq!(error.kind, FailureKind::Exhausted);
    }

    #[tokio::test]
    async fn absent_scanner_fails_closed() {
        let error = FailClosedScanner.scan(b"anything").await.unwrap_err();
        assert_eq!(error.code, SCANNER_UNAVAILABLE);
        assert_eq!(error.kind, FailureKind::Unavailable);
    }

    #[tokio::test]
    async fn scanner_size_limit_fails_before_any_network_attempt() {
        let scanner = ClamAvScanner::new("127.0.0.1:1")
            .unwrap()
            .with_limits(Duration::from_secs(1), Duration::from_secs(1), 4, 4)
            .unwrap();
        let error = scanner.scan(b"12345").await.unwrap_err();
        assert_eq!(error.code, "ARTIFACT_SCAN_LIMIT_EXCEEDED");
        assert_eq!(error.kind, FailureKind::Exhausted);
    }

    #[tokio::test]
    async fn clamd_instream_clean_response_is_clean() {
        let (address, server) = fake_clamd(b"hello", b"stream: OK\0").await;
        let scanner = ClamAvScanner::new(address).unwrap();
        assert_eq!(scanner.scan(b"hello").await.unwrap(), ScanVerdict::Clean);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn clamd_found_response_quarantines_and_unknown_response_fails_closed() {
        let (address, server) = fake_clamd(b"eicar", b"stream: Win.Test.EICAR_HDB-1 FOUND\0").await;
        let scanner = ClamAvScanner::new(address).unwrap();
        assert_eq!(
            scanner.scan(b"eicar").await.unwrap(),
            ScanVerdict::Quarantined {
                signature: "Win.Test.EICAR_HDB-1".to_owned()
            }
        );
        server.await.unwrap();

        let (address, server) = fake_clamd(b"data", b"stream: scanner internal ERROR\0").await;
        let scanner = ClamAvScanner::new(address).unwrap();
        let error = scanner.scan(b"data").await.unwrap_err();
        assert_eq!(error.code, SCANNER_UNAVAILABLE);
        server.await.unwrap();
    }

    #[test]
    fn s3_debug_never_contains_credentials() {
        let config = S3Config::new(
            "https://storage.example",
            "bucket",
            "us-east-1",
            "ACCESS-SECRET-MARKER",
            "SECRET-SECRET-MARKER",
            false,
        )
        .unwrap();
        let debug = format!("{config:?}");
        assert!(!debug.contains("ACCESS-SECRET-MARKER"));
        assert!(!debug.contains("SECRET-SECRET-MARKER"));
        assert!(debug.contains("<redacted>"));
    }

    async fn fake_clamd(
        expected: &'static [u8],
        response: &'static [u8],
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut command = [0_u8; 10];
            socket.read_exact(&mut command).await.unwrap();
            assert_eq!(&command, b"zINSTREAM\0");
            let mut received = Vec::new();
            loop {
                let size = socket.read_u32().await.unwrap() as usize;
                if size == 0 {
                    break;
                }
                let start = received.len();
                received.resize(start + size, 0);
                socket.read_exact(&mut received[start..]).await.unwrap();
            }
            assert_eq!(received, expected);
            socket.write_all(response).await.unwrap();
        });
        (address, server)
    }
}
