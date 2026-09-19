use std::time::{SystemTime, UNIX_EPOCH};

use masonwing_artifacts_adapter::{ArtifactStoreAdapter, S3Config};
use masonwing_kernel::runtime::ArtifactObjects;
use tokio::io::AsyncReadExt;

/// Live local-infrastructure probe. It is ignored by the normal unit suite and
/// must be run explicitly through tests/artifacts/run_minio_adapter_smoke.py.
/// A skipped probe is never acceptance evidence.
#[tokio::test]
#[ignore = "requires the masonwing-dev MinIO fixture"]
async fn local_minio_enforces_immutable_put_and_bounded_roundtrip() {
    let adapter = ArtifactStoreAdapter::from_s3(S3Config::from_env().expect("local S3 config"))
        .expect("S3 adapter");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let key = format!("qualification/object-store/{nonce}");
    let original = b"masonwing synthetic artifact".to_vec();

    adapter
        .put_immutable(&key, original.clone(), "text/plain")
        .await
        .expect("initial immutable write");
    let overwrite = adapter
        .put_immutable(&key, b"mutated".to_vec(), "text/plain")
        .await
        .expect_err("overwrite must fail");
    assert_eq!(overwrite.code, "IMMUTABLE_ARTIFACT");
    assert_eq!(
        adapter
            .get_bounded(&key, original.len())
            .await
            .expect("bounded read"),
        original
    );
    assert_eq!(
        adapter
            .get_bounded(&key, original.len() - 1)
            .await
            .expect_err("oversized read must fail")
            .code,
        "ARTIFACT_READ_LIMIT_EXCEEDED"
    );

    // Exercise the real S3 conditional multipart-copy commit, not only an
    // in-memory store's copy implementation. Ordinary multipart completion
    // alone would permit overwriting the final object.
    let stream_key = format!("qualification/object-store-stream/{nonce}");
    let streamed = vec![0x5a; 5 * 1024 * 1024 + 17];
    adapter
        .put_reader(
            &stream_key,
            &mut std::io::Cursor::new(&streamed),
            streamed.len() as u64,
            "application/octet-stream",
        )
        .await
        .expect("conditional multipart stream commit");
    let conflict = adapter
        .put_reader(
            &stream_key,
            &mut std::io::Cursor::new(b"other"),
            5,
            "text/plain",
        )
        .await
        .expect_err("stream overwrite must fail");
    assert_eq!(conflict.code, "IMMUTABLE_ARTIFACT");
    let mut download = adapter
        .open_reader(&stream_key, streamed.len() as u64)
        .await
        .expect("stream download");
    assert_eq!(download.size_bytes, streamed.len() as u64);
    let mut downloaded = Vec::new();
    download.reader.read_to_end(&mut downloaded).await.unwrap();
    assert_eq!(downloaded, streamed);
}
