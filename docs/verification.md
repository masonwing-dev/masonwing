# Kiểm chứng scaffold local — 16/09/2026

> Bản ghi bên dưới là snapshot scaffold trước khi nối runtime ứng dụng. Nó được
> giữ nguyên để bảo toàn lịch sử, không phải trạng thái source mới nhất. Phần
> registry, plugin invocation, Wasmtime và fixture PostgreSQL/MinIO mới được mô tả
> trong `registry-invocation-implementation.md`; bằng chứng mới nằm dưới `.evidence/`.

Đây là bằng chứng của **scaffold và các cơ chế đã triển khai**, không phải nghiệm
thu toàn bộ sản phẩm. Nguồn máy chạy: macOS arm64, Rust/Cargo 1.97.1, Node 26.8.1,
pnpm 11.24.0, Python 3.14.5, Docker 29.4.0 / Compose 5.1.2 trên OrbStack.

## Kết quả kiểm tra

| Nhóm | Kết quả | Bằng chứng local |
| --- | --- | --- |
| Rust workspace | 28 packages compile; 41 tests qua | `.dev/evidence/rust-tests.log` |
| State vectors | 314 Masonwing + 294 Gleanbird được thực thi trong 2 Rust tests | `crates/kernel/tests/state_vectors.rs`, `products/gleanbird/measurement-optimization/tests/state_vectors.rs` |
| Baseline/schema/kiến trúc | 14 tests qua; checksum spec gốc và generated drift hợp lệ | `.dev/evidence/scaffold.xml` |
| Web behavior | 57 tests qua | `.dev/evidence/web-tests.log` |
| API và dependency integration | 109 tests qua trên Compose thật, gồm đủ 92 command paths | `.dev/evidence/integration.xml` |
| Browser | 9 tests qua: 4 viewport × 2 theme, thêm toàn bộ 45 route với chữ 200% | `.dev/evidence/playwright-results.json` |
| Product acceptance backlog | 1.359 RED, 0 collection errors, 0 skipped | `.dev/evidence/acceptance-red.xml` |

`scripts/verify.py --integration --browser` ghi từng argv, exit code, thời lượng
và log vào `.dev/evidence/verification.json`. Script bao gồm kiểm tra drift của
spec và TypeScript contracts, Rust format/check/Clippy với warnings-as-errors,
Rust tests, Python scaffold/integration tests, TypeScript typecheck, UI tests,
production web build và browser suite. Đọc file JSON để xác nhận kết quả của
lần chạy gần nhất; không suy ra PASS từ việc tài liệu này tồn tại.

Lần chạy toàn bộ acceptance đã collect và thực thi đủ 1.359 test. Chúng fail
đúng tại `NotImplementedError` vì chưa có scenario driver cho handler sản phẩm,
không bị thay bằng skip/xfail. `make tdd CASE='MASONWING@1.0.1:TC-AC-088'`
chọn đúng một node theo namespace và in các đầu ra/bằng chứng yêu cầu. Log riêng
ở `.dev/evidence/tdd-single-red.log`. Các test domain nhỏ đã qua không làm cho
toàn bộ AC cùng tên tự chuyển thành PASS.

## Stack đang chạy

Compose project `masonwing-dev` có 12 container: gateway, web, api, worker,
component-runner, remote-runner, postgres, keycloak, temporal, storage, openbao
và provider-fixtures. Bản chụp trạng thái ở `.dev/evidence/compose-status.json`.
Gateway publish đúng các cổng loopback 39850–39859; các application/dependency
containers chỉ nằm trên mạng nội bộ. Named volumes được giữ lại.

Liveness của các tiến trình khỏe. API write readiness **503 / false** vì adapter
nghiệp vụ chưa qualified. Test HTTP xác nhận sự khác biệt này, xác nhận lỗi theo
schema và xác nhận credential giả không tạo ra successful receipt. Namespace
Temporal `masonwing-local` đã được kiểm tra qua CLI engine. Chưa chạy workflow
nghiệp vụ hay live model/provider.

## Kiểm tra hình ảnh

Đã đọc trực tiếp các ảnh browser thực sau các sửa lỗi giao diện:

- `.dev/evidence/browser/scaffold-development-shell-1440px-light/home-1440-light.png`
- `.dev/evidence/browser/scaffold-development-shell-320px-dark/home-320-dark.png`
- `.dev/evidence/browser/scaffold-development-shell-1440px-light/composition-1440-light.png`
- `.dev/evidence/browser/scaffold-all-45-route-scaf-78780-able-at-320px-with-200-text/masonwing-200-percent.png`

Ở các ảnh đã đọc, không thấy tiêu đề/card/control đè lên nhau. Header ở 320 px
với chữ 200% xuống hai hàng; nhãn Framework không còn bị badge Scaffold che.
Ảnh 200% là viewport capture, không phải toàn bộ chiều dài trang. Ảnh home và
composition desktop là full-page capture. Đây không phải so sánh pixel với một
prototype được duyệt hoặc nghiệm thu mọi trạng thái của 45 màn hình.

Browser tests kiểm tra horizontal overflow, một h1, disabled business actions,
menu close/focus restore, expected-output accordion và axe serious/critical
violations. Chúng đã bắt và dẫn tới sửa focus restoration, contrast của số module
và overlap header khi tăng cỡ chữ. Test hình học cho overlap header đã được giữ.

## Những phần chưa có bằng chứng sản phẩm

OIDC BFF login/session, Cedar policy, SQLx application repositories/incremental
migrations, Temporal workflows, Wasmtime/remote execution, model/connectors,
effect broker dispatch, full Gleanbird workflows và production gates vẫn cần
implementation và scenario evidence. Database/secret/provider fixture tests
chứng minh chính các cơ chế local đó; không thay cho guard/transaction/authority
được nối trong handler thật. State adjacency không thay thế guard conformance.

Không có tác động live ra provider, không publish, không commit/push và không
xóa volume để làm test đạt. Source baselines giữ nguyên checksum.
