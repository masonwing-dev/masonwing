# CR-BRAND-001 — Đổi tên framework và sản phẩm marketing

## Quyết định và phạm vi

Người dùng chọn **Masonwing** thay cho **Kernivo**, và **Gleanbird** thay cho **Evidemand**, ngày 16/09/2026. Nguồn: `SRC-CHAT-BRAND`. Đây là bản cập nhật nhận diện tài liệu **v1.0.1**, không phải một release sản phẩm đã triển khai.

| Hạng mục | Trước | Hiện hành |
|---|---|---|
| Core framework | Kernivo | Masonwing |
| Marketing agent | Evidemand | Gleanbird |
| Framework spec namespace | KERNIVO@1.0.0 | MASONWING@1.0.1 |
| Marketing spec namespace | EVIDEMAND@1.0.0 | GLEANBIRD@1.0.1 |
| Shared schema identity | urn:kernivo:shared:1.0.0 | urn:masonwing:shared:1.0.0 |
| Platform OpenAPI filename | kernivo-platform.openapi.json | masonwing-platform.openapi.json |

Áp dụng tên mới cho SRS, README, metadata, product catalogs, các đường dẫn/tên tệp có thương hiệu, OpenAPI titles, schema URNs, technical strings có tên thương hiệu, cross-product map, fixtures và scripts kiểm tra. Spec namespace/version là định danh tài liệu; không phải version của wire contract. Local IDs không renumber.

## Không đổi

Giữ nguyên toàn bộ scope và release partitions, functional requirements, AC, NFR, ADR nội dung kỹ thuật, các test case/suite và oracle, typed operations, schema cấu trúc, auth stack, tenant/capability policies, safety defaults, budget, approval và trạng thái chưa thực thi của product tests. Không giảm yêu cầu bảo mật hoặc biến Proposed/Pending thành Accepted/Verified, trừ quyết định tên được người dùng xác nhận.

## Compatibility và bàn giao

Shared/wire contract vẫn **1.0.0**. URN/title/tên tệp có chứa tên cũ được thay; cả hai bundle phải cập nhật đồng thời. `contract-lock.json` dùng digest mới của shared-contracts và consumer đúng tên. Không coi đây là cam kết backward-compatible runtime identity: chưa có mã nguồn triển khai nào được migrate trong tác vụ này. Khi có downstream implementation, dùng `brand-aliases.json` để tìm config/import/URN/fixtures cần thay. Không tự sửa tài nguyên đã deploy, dữ liệu khách hàng hay registry của bên ngoài.

## Bảo toàn nguồn và lịch sử

Giữ nguyên bytes của `inputs/` đã có và `vendor/requirements-spec/`. Thêm một bản ghi quyết định tên mới thay vì viết lại nguồn nghiên cứu cũ. Các báo cáo kiểm tra gốc v1.0.0 được giữ ở `09-audit/history/v1.0.0/`; không thay tên trong kết quả lịch sử để giả thành lượt kiểm tra mới. Bản semantic/AC review hiển thị theo tên mới được đánh dấu là review kế thừa.

Các receipt machine-check hiện hành được tạo lại. `MANIFEST.sha256` được tính lại sau khi toàn bộ file đã chốt và được kiểm tra read-only trên ZIP giải nén. Việc kiểm tra tài liệu không thay thế chạy các test sản phẩm.

## Phê duyệt còn mở

Quyết định tên đã được xác nhận. Trademark/domain/package namespace clearance, independent review, policy approvals và production gates giữ nguyên trạng thái. Q-006 không bị đóng bởi việc chọn tên; không có kiểm tra trademark/domain bổ sung trong bản cập nhật này.
