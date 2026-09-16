# DDD: ranh giới domain và hợp đồng triển khai

Domain-Driven Design ở đây là tổ chức code theo quyền sở hữu và invariant nghiệp
vụ. Given/When/Then là behavior specification; chúng không thay thế domain model.
`contracts/domain-contexts.json` và `docs/feature-map.md` là bản đồ đủ 45 feature.

## Dependency direction

```text
Browser contribution → shared UI / plugin SDK / typed API client
                                       ↓
                                Rust BFF composition
                                       ↓
                             application ports / kernel
                                       ↓
                         qualified infrastructure adapters

Gleanbird / document-review / checksum → public Rust SDK → host capabilities
```

Domain plugin không import implementation của kernel/platform adapter, không
thực hiện SQL trực tiếp sang plugin khác và không gọi provider mutation trực tiếp.
Host broker nhận effect proposal; authority, approval, budget và actual dispatch
thuộc host. Các command URL trong local BFF được sinh từ composition's OpenAPI.

## Platform ownership

| Boundary | Giá trị/aggregate khởi đầu | Quy tắc cần giữ |
| --- | --- | --- |
| Identity và membership | TenantId, PrincipalId, membership epoch | Tenant/current authority do server xác định; dữ liệu phiên không ra browser |
| Registry và execution | PluginId, dependency graph, versioned lifecycle | Không cycle; permission không rộng hơn manifest/grant; execution cô lập |
| Authorization và delegation | ScopedResource, RunGrant, action/resource sets | Child scope/expiry không vượt parent; hết hạn tại đúng boundary bị từ chối |
| Workflow | Run state, bounded loop | Terminal không resume; không dispatch thêm sau giới hạn |
| Approval | Immutable ApprovalBinding | Revision, digest, target, scope và expiry khớp tại thời điểm thực thi |
| Effects | Versioned effect state / outcome | Unknown không resend mù; reconcile trước; không fabricated success |
| Budget | Reservation và usage | Unknown usage giữ reservation; không biến null thành 0 |
| Data/events | Tenant transaction, audit, inbox/outbox | Business/audit/outbox cùng transaction; RLS và dedupe độc lập |
| Artifacts/connections | ArtifactId, digest, connection handle | Bytes nằm trong artifact store; không đưa raw secret vào domain |

Các Rust newtype cho opaque ID, Action và Digest validate ở cả constructor và
serde deserialization. Command handlers
phải kiểm tra schema rồi tái kiểm tra các invariant liên quan dữ liệu hiện hành
trong transaction; schema-valid JSON không tự tạo ra authority.

## Gleanbird ownership

| Package | Features | Aggregate/behavior |
| --- | --- | --- |
| `brand-truth` | F-001–F-002 | Brand, verified facts, voice revisions |
| `sources-voc` | F-003–F-005 | Source rights/revisions, ingestion, pain clusters |
| `opportunities` | F-006–F-007 | SCORE-1, hard blockers, opportunity selection, inventory/URL plans |
| `evidence-content` | F-008–F-012 | Claim evidence, content blocks/revisions, quality/SEO, approval proposals |
| `delivery` | F-013–F-014 | Publication state, effect receipts, crawl verification |
| `measurement-optimization` | F-015–F-017, F-020–F-022 | Eligible samples, Wilson intervals, experiments, autonomy, reporting/qualification |
| `media` | F-018 | Render/reuse and derived rights |
| `distribution` | F-019 | Scheduled occurrence, cancellation and live connection state |

Các quy tắc thuần — SCORE-1, source rights, protected blocks, stale revisions,
denominators và Wilson95 — có implementation và test trong các package tương
ứng. Phần command orchestration/persistence còn `NOT_IMPLEMENTED`; một rule unit
test không nghiệm thu cả feature.

## Khi implement một slice

Tạo aggregate/value objects ở package sở hữu, application service qua port và
adapter riêng. Chọn đúng immutable revision/version; inject clock/provider/ID
port để test có tính xác định. Transaction repository cung cấp unit of work cho
business state + audit + outbox. Các truy vấn projection không trở thành nguồn
authority; một refresh không được làm mất manual edit đã lưu.

Test phải chứng minh cả output và absence of forbidden changes. Với command bị
từ chối, snapshot version, persisted rows, audit lý do từ chối theo spec và
provider counters phải được so sánh. Với external effects, checkpoint, fence và
idempotency fingerprint thuộc persisted intent; không dùng một biến trong process
để thay durable delivery guarantee.

`products/document-review` và `products/checksum` là hai fixture chống việc đẩy
logic marketing vào kernel. Checksum chứng minh deterministic/no-model behavior;
document-review giữ second-domain integration point, chưa có workflow nghiệp vụ.
