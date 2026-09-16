# Phân tích requirements và quyết định scaffold

Nguồn: toàn bộ hai baseline trong thư mục dự án, đặc biệt SRS trong
`plans/reports/`, architecture/auth, protocol/OpenAPI/schema, data model,
screens, work packages, state/reference vectors, test catalog, release gates
và governance. `contracts/spec-inventory.json` ghi đường dẫn và SHA-256 của
từng input. Các test kiểm tra lại `MANIFEST.sha256` của từng bộ.

## Phạm vi thực sự của dự án

| Nội dung | Masonwing | Gleanbird | Tổng |
| --- | ---: | ---: | ---: |
| Feature | 23 | 22 | 45 |
| Requirement | 158 | 145 | 303 |
| Acceptance criterion | 162 | 145 | 307 |
| Quality requirement | 24 | 24 | 48 |
| Test case được thiết kế | 686 | 673 | 1.359 |
| Suite | 43 | 42 | 85 |
| Command | 43 | 49 | 92 |
| Schema definition | 82 | 104 | 186 |
| State vector | 314 | 294 | 608 |

Đây là hai sản phẩm có quan hệ platform → domain plugin, không phải hai phiên
bản thay thế nhau. Scaffold giữ cả C1/C2 và M1/M2/M3. Phạm vi C2/M3 vẫn có vị trí
trong source, contracts và test catalog; không tự bật các tính năng đó.

Các ID như `REQ-001` hoặc `TC-AC-001` trùng giữa hai bộ. Mọi đối chiếu tổng hợp
dùng namespace, ví dụ `MASONWING@1.0.1:TC-AC-001` và
`GLEANBIRD@1.0.1:TC-AC-001`. Các trường gốc không bị đổi tên. Shared schema và
platform OpenAPI được kiểm tra byte-identical giữa hai bundle.

## Các ràng buộc ảnh hưởng trực tiếp đến cấu trúc

Kernel phải giữ trung lập với nghiệp vụ. Registry, capability, lifecycle,
state/approval/effect/budget guards và application ports nằm ở Rust core.
Mọi thuật ngữ source rights, claim, draft, publication hay marketing measurement
thuộc plugin sản phẩm. SDK không trao API gửi mutation trực tiếp hoặc raw secret
cho plugin. Danh sách URL command của bản local được sinh tại composition/BFF,
không đưa catalog Gleanbird vào kernel hoặc contract library dùng chung.

Identity, policy, durable workflow, database và secret storage tái sử dụng adapter
cho Keycloak/OIDC, Cedar, Temporal, PostgreSQL và OpenBao. Scaffold không viết một
IdP/scheduler thay thế. Các dịch vụ hạ tầng chạy thật ở local; adapter Rust tương
ứng vẫn báo `UNQUALIFIED` cho đến khi có implementation và conformance evidence.

Command receipt không đồng nghĩa side effect đã thành công. Khi remote có thể
đã nhận request nhưng kết nối bị đứt, domain chuyển sang unknown và cần reconcile.
Budget với usage chưa biết phải giữ reservation. Approval phải gắn đúng revision,
digest, target, scope và expiry. Các quy tắc này có test output trực tiếp ngay
trong khung để tránh hướng triển khai sai từ đầu.

Gleanbird phải phân biệt nguồn có quyền sử dụng, bằng chứng thực, dữ liệu thiếu
và dữ liệu có giá trị bằng 0. SCORE-1 không được tự coi factor thiếu là 0; source
rights có thể chặn cơ hội dù score cao. Measurement dùng đúng eligible denominator,
zero eligible trả `NO_DATA`/null. Manual protected blocks giữ nguyên khi AI đề xuất
nội dung mới; stale revision không được ghi đè bản hiện hành.

## Cách chuyển spec thành nơi làm việc

`contracts/domain-contexts.json` nối đủ 45 features tới work package, commands,
screen và test IDs. `docs/feature-map.md` là bảng đọc nhanh được sinh từ dữ liệu
đó. Developer chọn một feature rồi mở command schema và acceptance case của
chính namespace đó; không cần suy đoán lại phạm vi.

Mỗi feature có route trong React shell, một tiêu đề chính, các action bị disable
cho tới khi handler có thật, và danh sách Given/When/Then trong spec. Đây là
catalog phát triển có thể chạy, không phải 45 màn hình nghiệp vụ đã nghiệm thu.
Trang chủ lấy số liệu cấu trúc từ spec và trạng thái thật của Rust process.

Wire examples chỉ được dùng kiểm tra schema. State vector chứng minh bảng chuyển
trạng thái và version behavior; riêng nó không chứng minh quyền, transaction,
revocation, fencing hay thời điểm provider thật nhận request. Các phép kiểm tra
đó được tách thành unit/domain tests, dependency integration tests và future
product scenario drivers.

## Các điểm cần triển khai tiếp theo dependency

Nền tảng đầu tiên là identity/session/current membership, Cedar authorization,
runtime DB transaction/RLS, registry và artifact references. Sau đó nối
workflow/approval/effect/budget theo một vertical slice có thể kiểm chứng. Gleanbird
M1 dùng các hợp đồng đã qualified, rồi mới tới content/publishing/measurement M2
và media/distribution M3. Work packages gốc vẫn là nguồn cho dependencies và AC.

Các boundary để phát triển đã có; các phần sau vẫn chưa có implementation đầy đủ:
OIDC BFF login, session store, policy evaluation, SQLx repositories/migration
runner, Temporal workflows, Wasmtime execution, remote protocol, provider/model
adapters, effect dispatch, catalog release flow và các workflow nghiệp vụ Gleanbird.
Không có thao tác live publishing nào được thực hiện trong lần scaffold.

## Các câu hỏi gốc vẫn còn hiệu lực

Q-001–Q-007 trong `08-governance/open-questions.json` của hai baseline vẫn giữ
nguyên. Chúng liên quan business freeze, region/retention/provider handling,
authorized live accounts/source licenses, production defaults, named owners và
reviewers, public brand/license strategy, dependency/performance qualification.
Chúng không chặn local synthetic coding; scaffold không tạo chữ ký phê duyệt
hoặc tự coi các release gate đã đạt.

Phiên bản dependency được chọn từ registry thực và khóa trong `Cargo.lock`,
`pnpm-lock.yaml`, `requirements-dev.txt` cùng Compose image digests. Việc khóa
version làm môi trường tái lập được; production security/license/capacity sign-off
vẫn là công việc riêng theo spec, đặc biệt trước khi phân phối service images.
