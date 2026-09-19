# Artifact object-store implementation

This slice implements the byte-store boundary needed by WP-010 without taking ownership of artifact metadata, lifecycle SQL, HTTP download authorization, signed URLs, deletion propagation or legal-hold orchestration.

## Runtime boundary

`ArtifactStoreAdapter` implements the exact `masonwing_kernel::runtime::ArtifactObjects` trait:

- `put_immutable(key, bytes, content_type)` maps the host-provided key to an object-store path and performs one atomic conditional create using `object_store::PutMode::Create`. It never performs a head-then-put fallback. An existing object or failed create precondition maps to `IMMUTABLE_ARTIFACT`.
- `get_bounded(key, max_bytes)` checks the object metadata size before consumption and still counts every streamed chunk before appending it to memory. A result over the caller's bound fails with `ARTIFACT_READ_LIMIT_EXCEEDED`.
- Store failures are converted to stable `CommandFailure` codes. Raw S3/MinIO errors, endpoints and credential values are not propagated through the application boundary.

The S3 implementation uses Apache Arrow `object_store` 0.14.2 with its S3 backend. MinIO conditional puts use standard `If-None-Match` semantics through `S3ConditionalPut::ETagMatch`; this is what makes concurrent duplicate writes reject instead of overwrite. Object content type is written as an object attribute.

`S3Config` accepts explicit configuration or reads `S3_ENDPOINT`, `S3_BUCKET`, region and credentials from the runtime environment. Standard AWS credential names are preferred; the synthetic local environment may use `MINIO_ROOT_USER` and `MINIO_ROOT_PASSWORD`. Its Debug representation redacts both credential fields. Plain HTTP endpoints are accepted only when `MASONWING_ENV=LOCAL`.

## Scanner boundary

`ArtifactScanner` is a separate async port for finalize orchestration. `FailClosedScanner` always returns `ARTIFACT_SCANNER_UNAVAILABLE`; absence of a scanner can therefore never be interpreted as CLEAN.

`ClamAvScanner` implements the clamd `INSTREAM` protocol over a bounded TCP connection. It sends fixed-size chunks, caps the total scan input, caps the response size and uses connection/response timeouts. Only an exact `stream: OK` response becomes `ScanVerdict::Clean`. A `FOUND` response becomes `ScanVerdict::Quarantined`; unknown, malformed, timeout, socket and clamd `ERROR` responses fail closed. This crate does not mark artifact metadata QUARANTINED itself because metadata/lifecycle orchestration is outside this owned slice.

No ClamAV service is added to Compose by this change. Until an integration environment supplies a real clamd endpoint and finalization wires this port, scanner qualification remains outside this slice.

## Tests

Crate unit tests use `object_store::memory::InMemory` to prove immutable duplicate-write denial, original-byte preservation, bounded reads, credential Debug redaction and fail-closed scanner behavior. The ClamAV protocol tests run against a local TCP fixture that verifies the emitted `INSTREAM` frame before returning clean, malware or error responses; these are protocol tests, not malware-engine acceptance.

`platform-plugins/artifacts/tests/minio.rs` is an explicit live-local probe. It is ignored by normal unit runs so a missing developer MinIO is not mislabeled as a product test result. `tests/artifacts/run_minio_adapter_smoke.py` loads synthetic local credentials with `scripts.local_config`, points the adapter at the already-owned MinIO endpoint on `127.0.0.1:39856`, and runs that probe. A successful explicit run is required before claiming MinIO integration evidence.

The probe writes a unique synthetic immutable object and deliberately leaves it in the local object store; the adapter exposes no delete operation and the test does not mutate/delete Compose volumes.

Worker validation on 2026-09-16 first used Rust 1.97.1 and an isolated temporary manifest containing the exact owned adapter source so the prime-owned workspace lockfile was not changed. After the prime integration step refreshed `Cargo.lock`, the workspace-native `cargo test -p masonwing-artifacts-adapter` passed all seven unit tests, `cargo clippy -p masonwing-artifacts-adapter --all-targets -- -D warnings` passed, and `python3 tests/artifacts/run_minio_adapter_smoke.py` passed the live MinIO probe against `127.0.0.1:39856`. The live probe verified the initial immutable write, duplicate-write rejection, exact bounded read and undersized-bound rejection.

## Dependency integration

The owned crate manifest adds direct dependencies on `object_store 0.14.2`, `async-trait`, `futures-util` and Tokio networking/I/O/time features. The workspace root and `Cargo.lock` remain prime-owned and were refreshed by the integration owner. No generated contract is changed by this implementation.
