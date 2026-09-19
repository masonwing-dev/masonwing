use futures_util::TryStreamExt;
use masonwing_kernel::runtime::{ArtifactStream, CommandFailure, FailureKind};
use object_store::{Attribute, Attributes, ObjectStoreExt, PutMultipartOptions};
use std::{io, time::Duration};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio_util::io::StreamReader;
use uuid::Uuid;

use super::{
    ArtifactStoreAdapter, ClamAvScanner, SCANNER_UNAVAILABLE, STORE_UNAVAILABLE, ScanVerdict,
    checked_content_type, checked_path, map_get_error, map_put_error, parse_clamd_response,
    read_clamd_response,
};

const MAX_BYTES: u64 = 1024 * 1024 * 1024;
const PART_BYTES: usize = 5 * 1024 * 1024;

impl ArtifactStoreAdapter {
    pub(crate) async fn stream_put(
        &self,
        key: &str,
        reader: &mut (dyn AsyncRead + Send + Unpin),
        size_bytes: u64,
        content_type: &str,
    ) -> Result<(), CommandFailure> {
        if !(1..=MAX_BYTES).contains(&size_bytes) {
            return Err(CommandFailure::invalid("ARTIFACT_SIZE_INVALID"));
        }
        let path = checked_path(key)?;
        let stage = checked_path(&format!("staging/{}", Uuid::new_v4()))?;
        let mut attributes = Attributes::new();
        attributes.insert(
            Attribute::ContentType,
            checked_content_type(content_type)?.into(),
        );
        let mut upload = self
            .store
            .put_multipart_opts(
                &stage,
                PutMultipartOptions {
                    attributes,
                    ..Default::default()
                },
            )
            .await
            .map_err(map_put_error)?;
        let result = tokio::time::timeout(Duration::from_secs(30), async {
            let mut remaining = size_bytes;
            while remaining > 0 {
                let count = remaining.min(PART_BYTES as u64) as usize;
                let mut chunk = vec![0; count];
                reader
                    .read_exact(&mut chunk)
                    .await
                    .map_err(|_| CommandFailure::invalid("ARTIFACT_STREAM_TRUNCATED"))?;
                upload.put_part(chunk.into()).await.map_err(map_put_error)?;
                remaining -= count as u64;
            }
            let mut extra = [0_u8; 1];
            if reader
                .read(&mut extra)
                .await
                .map_err(|_| CommandFailure::invalid("ARTIFACT_STREAM_INVALID"))?
                != 0
            {
                return Err(CommandFailure::invalid("ARTIFACT_SIZE_MISMATCH"));
            }
            upload.complete().await.map_err(map_put_error)?;
            // Multipart's ordinary completion permits overwrites. Commit through
            // maintained copy-if-absent instead; S3 uses conditional completion.
            // Final content type always comes from authorized host metadata.
            self.store
                .copy_if_not_exists(&stage, &path)
                .await
                .map_err(map_put_error)?;
            Ok(())
        })
        .await
        .unwrap_or_else(|_| Err(CommandFailure::unavailable(STORE_UNAVAILABLE)));
        if result.is_err() {
            let _ = tokio::time::timeout(Duration::from_secs(3), upload.abort()).await;
        }
        // Both keys are host-generated. This removes only this call's staging
        // object; a crash is handled by the staging-prefix lifecycle rule.
        let _ = tokio::time::timeout(Duration::from_secs(3), self.store.delete(&stage)).await;
        result
    }

    pub(crate) async fn stream_open(
        &self,
        key: &str,
        max_bytes: u64,
    ) -> Result<ArtifactStream, CommandFailure> {
        let path = checked_path(key)?;
        let result = self.store.get(&path).await.map_err(map_get_error)?;
        let size_bytes = result.meta.size;
        if size_bytes > max_bytes || size_bytes > MAX_BYTES {
            return Err(CommandFailure::new(
                "ARTIFACT_READ_LIMIT_EXCEEDED",
                FailureKind::Exhausted,
            ));
        }
        let stream = result
            .into_stream()
            .map_err(|_| io::Error::other(STORE_UNAVAILABLE));
        Ok(ArtifactStream {
            size_bytes,
            reader: Box::pin(StreamReader::new(stream)),
        })
    }
}

impl ClamAvScanner {
    pub(crate) async fn stream_scan(
        &self,
        reader: &mut (dyn AsyncRead + Send + Unpin),
        size_bytes: u64,
    ) -> Result<ScanVerdict, CommandFailure> {
        if size_bytes > self.max_scan_bytes as u64 {
            return Err(CommandFailure::new(
                "ARTIFACT_SCAN_LIMIT_EXCEEDED",
                FailureKind::Exhausted,
            ));
        }
        // One deadline covers connect, every write and the response. A scanner
        // which stops reading cannot retain the request indefinitely.
        tokio::time::timeout(self.response_timeout, async {
            let mut socket = tokio::time::timeout(
                self.connect_timeout,
                tokio::net::TcpStream::connect(&self.address),
            )
            .await
            .map_err(|_| CommandFailure::unavailable(SCANNER_UNAVAILABLE))?
            .map_err(|_| CommandFailure::unavailable(SCANNER_UNAVAILABLE))?;
            socket
                .write_all(b"zINSTREAM\0")
                .await
                .map_err(|_| CommandFailure::unavailable(SCANNER_UNAVAILABLE))?;
            let mut remaining = size_bytes;
            let mut chunk = vec![0_u8; self.chunk_bytes.min(64 * 1024)];
            while remaining > 0 {
                let count = remaining.min(chunk.len() as u64) as usize;
                reader
                    .read_exact(&mut chunk[..count])
                    .await
                    .map_err(|_| CommandFailure::invalid("ARTIFACT_STREAM_TRUNCATED"))?;
                socket
                    .write_all(&(count as u32).to_be_bytes())
                    .await
                    .map_err(|_| CommandFailure::unavailable(SCANNER_UNAVAILABLE))?;
                socket
                    .write_all(&chunk[..count])
                    .await
                    .map_err(|_| CommandFailure::unavailable(SCANNER_UNAVAILABLE))?;
                remaining -= count as u64;
            }
            let mut extra = [0_u8; 1];
            if reader
                .read(&mut extra)
                .await
                .map_err(|_| CommandFailure::invalid("ARTIFACT_STREAM_INVALID"))?
                != 0
            {
                return Err(CommandFailure::invalid("ARTIFACT_SIZE_MISMATCH"));
            }
            socket
                .write_all(&0_u32.to_be_bytes())
                .await
                .map_err(|_| CommandFailure::unavailable(SCANNER_UNAVAILABLE))?;
            socket
                .flush()
                .await
                .map_err(|_| CommandFailure::unavailable(SCANNER_UNAVAILABLE))?;
            parse_clamd_response(&read_clamd_response(&mut socket).await?)
        })
        .await
        .map_err(|_| CommandFailure::unavailable(SCANNER_UNAVAILABLE))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use masonwing_kernel::runtime::ArtifactObjects;
    use object_store::memory::InMemory;
    use std::sync::Arc;

    #[tokio::test]
    async fn multipart_copy_commit_is_immutable_and_stream_is_exact() {
        let adapter = ArtifactStoreAdapter::from_object_store(Arc::new(InMemory::new()));
        let bytes = vec![42_u8; PART_BYTES + 7];
        adapter
            .put_reader(
                "tenant/artifact",
                &mut io::Cursor::new(&bytes),
                bytes.len() as u64,
                "application/octet-stream",
            )
            .await
            .unwrap();
        let error = adapter
            .put_reader(
                "tenant/artifact",
                &mut io::Cursor::new(b"other"),
                5,
                "text/plain",
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, "IMMUTABLE_ARTIFACT");
        let mut stream = adapter
            .open_reader("tenant/artifact", MAX_BYTES)
            .await
            .unwrap();
        let mut actual = Vec::new();
        stream.reader.read_to_end(&mut actual).await.unwrap();
        assert_eq!(actual, bytes);
    }

    #[tokio::test]
    async fn underflow_and_overflow_never_commit_final_object() {
        let adapter = ArtifactStoreAdapter::from_object_store(Arc::new(InMemory::new()));
        for (key, bytes, size) in [
            ("tenant/short", b"a".as_slice(), 2),
            ("tenant/long", b"abc".as_slice(), 2),
        ] {
            assert!(
                adapter
                    .put_reader(key, &mut io::Cursor::new(bytes), size, "text/plain")
                    .await
                    .is_err()
            );
            assert_eq!(
                adapter.get_bounded(key, 10).await.unwrap_err().code,
                "RESOURCE_NOT_FOUND"
            );
        }
        let staged: Vec<_> = adapter
            .store
            .list(Some(&object_store::path::Path::from("staging")))
            .try_collect()
            .await
            .unwrap();
        assert!(staged.is_empty());
    }
}
