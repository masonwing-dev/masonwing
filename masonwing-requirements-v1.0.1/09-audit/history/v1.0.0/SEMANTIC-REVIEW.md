# Semantic author self-review

Mode: create. Đây là self-review của tác giả trên bộ tài liệu, không phải independent security review hoặc acceptance của Product Owner.

| Dimension | Disposition | Reason | Evidence |
|---|---|---|---|
| Failure-path coverage | PASS_WITHIN_DEFINED_SCOPE | 65/158 REQ là Unwanted behavior; ma trận state có cả cặp trái phép. Tỷ lệ không chứng minh mọi lỗi thực tế. | SRS;state-vectors;test-cases |
| Scope discipline | PASS_FOR_PROPOSED_BASELINE | Mỗi bullet In Scope mang F-ID có REQ; Out Of Scope mang ADR/F owner; M3 và C2 được giữ, không âm thầm xóa. | SRS Scope;release-partition;source-coverage |
| Constraint ownership | PASS | Key Constraints đều tham chiếu ADR. Các limit/quality defaults mới có assumption/owner; không coi là stakeholder-approved. | SRS Constraints/Decisions;assumptions.json |
| Actor coverage | PASS_WITH_DEPENDENCY_INHERITANCE | Tenant roles dùng action matrix; services dùng delegation; external providers/signers/support thuộc Kernivo. Evidemand kế thừa qua cross-product map, không viết lại core. | PERMISSIONS;cross-product-map;AUTH-AND-TENANCY |
| Testability | PASS_AS_DESIGN | 10 AC lấy mẫu đều có fixture/trigger/assertion và negative side-effect oracle; hiện chưa có implementation automation. | AC-SAMPLE-REVIEW;FIXTURE-RECIPES |
| Trace integrity | PASS | Mọi REQ có AC; mọi AC và NFR có primary designed case; suite/phase/dependency/source refs được kiểm bằng script. | check-results;traceability.csv |
| Quality coverage | PASS_WITH_DOCUMENTED_SIZE_DEVIATION | 24 NFR trên 9 characteristics ISO25010:2023. Vượt gợi ý12–20 vì tách isolation,provider/data boundary,restore,evidence freshness; không chia nhỏ chỉ để tăng số. | SRS Quality Requirements;TEST-STRATEGY |
| Decision coverage | PASS | 16 ADR; chỉ lựa chọn Rust/React rõ từ user Accepted. Các lựa chọn bổ sung Proposed; Evidemand có thêm6 quyết định domain ngoài16 quyết định platform nên vượt hướng dẫn10–18. | SRS Decisions |
| Independence | PASS_WITH_EXPLICIT_SHARED_OWNERSHIP | Core owns mechanisms; domain owns marketing rules. Negative tests bổ sung có thể hỗ trợ cùng AC nhưng không được tính là criterion mới. | cross-product-map;traceability relation |
| Implementation leakage | PASS_AS_SEPARATED_DESIGN | SRS mô tả hành vi; package/library/protocol choices nằm ADR và architecture. Provider-specific interoperability requirements giữ tên đúng vì đó là external contract. | SRS;architecture;dependency-candidates |
| Provenance | PASS_WITH_OPEN_APPROVALS | Thresholds/role mappings/retention/marketplace policies mới được ghi assumptions và owner;7 open questions phân biệt coding với production/public activation. Không bịa approval. | assumptions;open-questions;source-manifest |

## Kết quả theo chuẩn ZIP

Structural: 0 errors, 0 warnings qua validator nguyên bản.
Coverage: F=23 REQ=158 AC=162 NFR=24 ADR=16; REQ→AC=100%; AC/NFR→primary designed case=100%.
Unwanted-behaviour share: 41.14%.
Unowned assumptions: 0. Open questions: 7 (owner đã ghi, chưa có business approval).

Verdict: BUILDABLE theo proposed baseline đã chỉ định, không có structural/trace/scope orphan đã phát hiện. Không phải FROZEN, không phải independent completeness certificate. Những ngưỡng/policy chưa được người có thẩm quyền chấp thuận vẫn Proposed. Không khẳng định cover mọi khả năng vô hạn hoặc zero bug.

## Các sửa đổi cụ thể sau self-review

- Giữ capability/security adapter là trusted platform component, không cho tenant cài plugin tự thay auth; action catalog có PLATFORM_OPERATOR/SECURITY_REVIEWER riêng.
- Hoàn thiện OpenAPI cho upload bytes, artifact content, run alias/read; hai gói chứa cùng platform OpenAPI.
- Bổ sung typed UploadSession/ConnectorAuthorization để frontend không phải suy đoán upload hoặc OAuth handoff.
- Thêm workflow node/input/output contracts và reject unresolved handler/unbounded loop; deterministic fixture không LLM.
- Hoàn thiện known-failure/cancel trạng thái content/publication/distribution.
- Không giả WordPress có native conditional write/idempotency; thiếu capability thì BLOCKED, không get-before-post giả atomic.
- Tách schema/reference-model checks khỏi product tests và live evidence.

## Phần còn phải được kiểm chứng trong implementation

Dedicated OpenAPI validator chưa chạy do dependency không có trong môi trường; OpenAPI structure/local refs và toàn bộ typed schemas được kiểm. Không có Rust/TS product build, Wasm sandbox, browser snapshots, live IAM/CMS/LLM, performance, penetration hoặc DR receipts. Những mục này có test/gate nhưng hiện đều NOT_RUN/PENDING. Đây là giới hạn evidence, không được che bằng số lượng test case thiết kế.
