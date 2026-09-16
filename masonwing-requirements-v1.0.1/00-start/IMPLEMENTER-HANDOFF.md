# Handoff bắt đầu implement — MASONWING

Đầu vào bắt buộc: README, SRS tại `plans/reports/requirements-260916-0205-masonwing.md`, contracts, AUTH-AND-TENANCY, work-packages và test strategy. Tôn trọng ID; không tự đổi scope hoặc ghi quyết định Proposed thành Accepted. Tài liệu hiện không chứa mã nguồn sản phẩm.

## Công việc bắt đầu ngay

Khởi tạo monorepo theo architecture; khóa dependency versions bằng conformance bootstrap thay vì đoán “latest”; tạo schema-derived types và failing fixtures trước handler; dựng walking skeleton theo IMPLEMENTATION-ORDER. Chỉ sửa package thuộc ownership của work package. Dùng mock với synthetic data và live budget=0 khi chưa có activation evidence. Nối mỗi commit/test/screenshot thực tế với candidate digest và REQ/AC.

## Điều không được bỏ qua

Không clone auth/durable engine bằng mã tự viết; không import kernel internals từ plugin; không đưa OAuth tokens vào browser/model context; không gọi CMS trực tiếp ngoài effect broker; không coi summary/compaction là approval/evidence; không đánh dấu UI PASS nếu chưa có ảnh từ app thật. Không retry mutation OUTCOME_UNKNOWN nếu chưa reconcile. Giữ manual edits khi content refresh và recheck source-rights/current authorization trước external write.

## Điều kiện hoàn tất một slice

Mọi owned AC và impacted NFR có test thực thi, không skipped required case, có logs/ảnh/receipts đúng candidate; reviewer độc lập có danh tính; migration/rollback và no-side-effects đã kiểm; external production gates liên quan không bị giả thành PASS. Dùng work-packages như kế hoạch; không tự biến nó thành một authoritative tracker nếu dự án đã có tracker khác.

## Các quyết định chưa có thẩm quyền

Local implementation dùng defaults PROPOSED đã cụ thể hóa. Business freeze, public branding/license, region/retention/provider contracts, live credentials, named owners và benchmark capacity vẫn cần authority được ghi tại Q-001..Q-007. Không dùng pending approvals như lý do để bỏ qua contract-first local coding, và không dùng việc code xong như lý do tự activate production.
