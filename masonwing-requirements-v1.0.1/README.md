# Masonwing — Requirements specification v1.0.1

Ngày baseline: 16/09/2026. Đây là một trong hai bộ: **Masonwing framework** và **Gleanbird marketing plugins**. Tên thương hiệu đã được người dùng chọn ngày 16/09/2026; việc chọn tên không thay thế trademark/domain/package clearance.

**Trạng thái: BUILDABLE PROPOSED BASELINE / DRAFT, chưa stakeholder freeze hoặc product acceptance.** Các hành vi, defaults, boundaries, contracts và test oracles được cụ thể hóa để bắt đầu implement. Những policy mới, live accounts và production approvals được ghi có owner; không giả là user đã chấp thuận. Không cam kết mọi khả năng vô hạn hoặc zero bug.

| Hạng mục | Số lượng |
|---|---|
| Features | 23 |
| Functional requirements | 158 |
| Acceptance criteria | 162 |
| NFR | 24 |
| ADR | 16 |
| Test cases đã thiết kế | 686 |
| Test suites đã thiết kế | 43 |
| Cặp state distinct trong mô hình | 314 |
| Typed command operations | 43 |
| JSON Schema definitions | 82 |


## Bắt đầu ở đâu

1. Đọc SRS chính: [`plans/reports/requirements-260916-0205-masonwing.md`](plans/reports/requirements-260916-0205-masonwing.md).
2. Đọc [`00-start/IMPLEMENTATION-ORDER.md`](00-start/IMPLEMENTATION-ORDER.md), [`00-start/IMPLEMENTER-HANDOFF.md`](00-start/IMPLEMENTER-HANDOFF.md) và work packages tại `07-delivery/`.
3. Implement theo `02-architecture/`, `03-contracts/`, `04-data/`, `05-ux/`; dựng test từ `06-testing/` trước khi nhận PASS.

## Nội dung bàn giao

`01-product/` giữ scope/source coverage và structured REQ/AC catalog. `02-architecture/` gồm kernel/plugin boundaries, OSS reuse, OIDC auth, authorization/delegation, threat model, limits và dependency qualification. `03-contracts/` có OpenAPI3.1, JSON Schema2020-12, state machines, permission/action catalog, data plane và shared contract lock. `04-data/` có data dictionary, ownership, tenancy, migration/retention. `05-ux/` có design-system rules, screen/state/viewport matrix, accessibility và observable UI behavior.

`06-testing/` chứa Gherkin, primary AC/NFR cases, supplementary negative API/state/pairwise cases, suites, fixture recipes, numeric oracles và CSV trace. `07-delivery/` có release partitions, work packages, deployment/rollback/DR runbook và gates. `08-governance/` giữ provenance, assumptions/open questions, mapping toàn bộ18 elicitation probes, template conformance và change control. `09-audit/` chứa kết quả kiểm tra thực tế của tài liệu và semantic author review.

`inputs/` giữ research/annex nguồn và phần trích hội thoại có attribution. `vendor/requirements-spec/` là bộ chuẩn người dùng cung cấp, giữ nguyên files thực (chỉ loại AppleDouble metadata); optional tracking templates được lưu nhưng **tracking chưa khởi tạo**.

## Kết quả kiểm tra và giới hạn

Validator SRS nguyên bản: **0 lỗi, 0 cảnh báo**. REQ→AC=100% và AC/NFR→primary designed case=100% trong inventory đã khai báo. Schema fixtures, reference-model state/pairwise/metric checks và local references được kiểm bởi script đi kèm; xem `09-audit/check-results.json`.

**Các con số test cases/suites ở trên là thiết kế, không phải số test sản phẩm đã PASS. Product tests đã chạy: 0.** Chưa có Rust/React product source, test automation handlers, app screenshots, live provider qualification, load/DR/security receipts hoặc independent approval. Dedicated external OpenAPI validator chưa chạy; đã kiểm local structure/refs và typed schemas bằng jsonschema. Full acceptance được quy định trong release gates, không được suy từ document checks.

Để chạy lại kiểm tra mà không sửa tài liệu: tạo virtualenv, cài `scripts/requirements-validation.txt`, chạy `sh scripts/run-checks.sh`. Xem `scripts/README.md`. File `MANIFEST.sha256` niêm phong nội dung khi đóng ZIP.

## Ranh giới hai gói

Masonwing sở hữu runtime/framework; Gleanbird sử dụng contracts, không fork auth/storage/durable/effect engine. Cả hai gói chứa cùng `shared-contracts.json` và `masonwing-platform.openapi.json`. `03-contracts/cross-product-map.json` nối đúng namespaced feature/REQ giữa hai dự án. Sửa breaking shared contract cần coordinated version change; không sửa bản copy marketing riêng lẻ.

Phase C1/M1/M2 là thứ tự giao hàng, không bỏ phạm vi đã thống nhất khỏi tổng SRS. C2 giữ external SDK/public catalog; M3 giữ video/social/distribution. Thanh toán marketplace, paid-media budget automation và voice cloning không được ngầm coi là approved scope.

## Bản cập nhật thương hiệu v1.0.1

**Masonwing** là core framework; **Gleanbird** là sản phẩm marketing agent chạy trên Masonwing. Bản này chỉ đổi thương hiệu, namespace tài liệu, schema identity có chứa tên, tên tệp và tham chiếu liên quan. Giữ nguyên ID và nội dung nghiệp vụ của REQ/AC/NFR, test cases, test suites, release scope, giới hạn bảo mật và trạng thái product tests.

Spec/bundle là `1.0.1`; wire/shared contract vẫn `1.0.0` vì không đổi cấu trúc hoặc hành vi. Gleanbird v1.0.1 ghép với Masonwing v1.0.1; không trộn schema identity theo tên cũ. Xem [`08-governance/BRAND-MIGRATION.md`](08-governance/BRAND-MIGRATION.md), [`08-governance/brand-aliases.json`](08-governance/brand-aliases.json) và [`09-audit/branding-validation.json`](09-audit/branding-validation.json).

`inputs/` và `vendor/` được giữ nguyên byte. Tên cũ trong nguồn/biên bản audit lịch sử là dữ liệu nguồn được bảo tồn, không phải tên triển khai hiện hành. Chỉ quyết định tên đã được xác nhận; các phê duyệt khác vẫn theo baseline.
