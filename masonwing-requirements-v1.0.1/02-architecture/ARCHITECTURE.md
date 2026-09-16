# Kiến trúc triển khai

## Runtime và ranh giới

Microkernel giữ plugin registry/lifecycle, contract resolver, capability enforcement, execution envelope và audit boundary. Platform adapters cung cấp identity OIDC, authorization Cedar, PostgreSQL, artifacts, Temporal, model, connections, budgets và UI. Domain plugins sở hữu business models/handlers/read projections/UI contributions. **Nghiệp vụ không gọi provider mutation, secret store hoặc private SQL của plugin khác.**

Backend: Rust/Tokio/Axum + Serde + reqwest + SQLx. Frontend: React/TypeScript/Vite, TanStack Query, shared shadcn/ui/Tailwind package. Default infrastructure: PostgreSQL, Keycloak, Temporal, S3-compatible object storage, OpenBao/approved secret manager; **không tự viết distributed scheduler, IdP, crypto hoặc database**. Các lựa chọn package/phiên bản là candidate cho integration lock; exact locks được tạo ở bootstrap và phải qualification trước promotion.

Deploy đầu tiên là modular application với API host, workflow workers và isolated runners. Không mặc định mỗi agent là một process. Không bắt buộc Kafka, Redis, graph DB, vector DB, Kubernetes ở baseline. PostgreSQL text/full-text và vector extension chỉ khi ADR riêng được duyệt; evidence graph là typed relationships trước, không phụ thuộc graph vendor.

## Dòng xử lý command

HTTP request -> session/service-token verification -> tenant resolution -> schema validation -> current authorization -> optimistic version/idempotency -> transaction gồm business mutation,audit,outbox -> CommandReceipt. Worker đọc committed outbox -> durable execution -> scoped plugin -> model/read tool -> proposed effect -> current guards -> provider -> receipt/reconciliation.

CommandReceipt ACCEPTED chỉ là đã nhận command, không là business success. Một CommandReceipt SUCCEEDED cho create-proposal cũng không có nghĩa proposal đã publish. UI xem effect receipt hoặc domain terminal state cho outcome.

## Authority table

| Dữ liệu | Nguồn thẩm quyền | Projection không được quyết định thay |
|---|---|---|
| User identity | Verified OIDC issuer+sub | email,display name |
| Membership/permission/grants | Current platform records + evaluated policy | JWT role cũ, UI cache, model text |
| Business revision/approval | PostgreSQL versioned record | model summary, search index |
| Remote mutation outcome | Provider evidence theo operation contract | HTTP timeout, local retry count |
| Execution history | Durable engine | frontend progress bar |
| Cost | Reservation ledger + verified usage receipts | model estimate hoặc missing=0 |
| Artifact bytes | Content-addressed immutable store | filename hoặc title |

## Transactions và consistency

Không có transaction phân tán giữa PostgreSQL,Temporal,object store và CMS. Finalize upload thành immutable artifact trước khi business reference active. Outbox ghi cùng transaction business; dispatcher idempotent bằng workflow/run ID. Inbox dedupe bằng (tenant,consumer,event_id). Audit writes không được deferred nếu action cần atomic audit. Projections có watermark và stale status.

Nguồn authority không truy cập được: bảo vệ write fail closed. Cached authorized reads chỉ theo lease đã xác định; restricted access không dùng stale policy. Unknown remote outcome giữ reservation và chặn retry; recovery sau restore phải đối soát remote effects có thể đã xảy ra sau backup.

## Bốn lớp state

Business/evidence/approval/receipt; durable execution; encrypted provider-native conversation; retrieval/cache. Compaction chỉ tác động lớp conversation. Đổi model/provider phải kiểm capability và privacy profile, không chuyển opaque state không tương thích. Model output không là quyền hoặc bằng chứng tự chứng nhận.

## Extension classes

Trusted native đã code-reviewed có toàn trust của process: không gọi là sandbox. Wasm dùng Wasmtime Component/WIT, fuel/memory/epoch/deadline và scoped host handles. Remote worker cho browser/media/non-Rust libraries, chạy nonroot, read-only rootfs, seccomp/container boundary và bounded egress. Host HTTP có timeout riêng; sandbox guest không tự chặn tất cả blocking I/O.

## Quy ước repo

`crates/contracts`, `crates/kernel`, `crates/sdk`, `crates/host-api`, `crates/worker`, `crates/component-runner`, `crates/remote-runner`, `crates/cli`; `packages/ui`, `packages/web-shell`, `packages/plugin-sdk`, `packages/api-client`; `platform-plugins/*`; `products/*`; `contracts/*`; `tests/*`. Cargo workspace và pnpm workspace không chia sẻ secret config. Generated contracts được kiểm drift, không sửa tay generated output.

## Dependency/license process

Reuse first, isolate behind contract, inspect license, pin version, run conformance, then promote. `dependency-candidates.json` không phải SBOM của app đã build. Bootstrap sinh Cargo.lock,pnpm-lock,container digests,SBOM và vulnerability/license reports; giữ GPL/AGPL/BSL/unknown dưới legal review thay vì tự gọi là permissive. Không copy code từ source review nếu chưa kiểm license và notices.
