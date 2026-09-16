# Kiểm tra khả năng dựng fixture cho 10 AC

Chọn xác định bằng seed 20260916 để reviewer có thể lặp lại. Đây là tác giả đọc criterion và oracle, không phải kết quả chạy app.

| AC | Fixture/precondition | Trigger | Assertion |
|---|---|---|---|
| AC-011 | module A truy cập điểm riêng của B | register contribution | EXTENSION_SCOPE_DENIED; B không bị thay handler |
| AC-098 | provider timeout sau acceptance | settle attempt | cost_state UNKNOWN; không giải phóng reservation như zero; reconcile queued |
| AC-041 | invite đúng tenant/email đã verify; chưa hết hạn | hai request accept đồng thời | một membership; response replay trả cùng membership; không thêm role khác |
| AC-028 | mock issuer trả token đúng issuer, audience, nonce; code chưa dùng | GET /auth/callback | session mới gắn issuer+sub; cookie HttpOnly Secure; tokens không xuất hiện trong response body |
| AC-095 | bài đã publish và user yêu cầu revert | submit compensation | effect mới có approval riêng; original receipt giữ nguyên; UI không nói lịch sử đã bị xóa |
| AC-130 | chưa login; deep link resource A | login thành công | đến A sau auth; nếu không có quyền trả 404 thay vì landing giả |
| AC-062 | process bị kill ngay sau commit mutation | restart dispatcher | business event được giao; không mất acknowledged intent |
| AC-152 | listing chưa đủ reviews | request public install | LISTING_NOT_APPROVED; artifact không được activated |
| AC-113 | writer gọi tool với connection C | inspect tool context và output | chỉ handle C; refresh token không có trong prompt, logs hoặc browser payload |
| AC-147 | backup Postgres/object store/history khác watermark | restore | write readiness=false đến khi repair/reconcile hoàn tất; dangling refs được report |

Kết luận tác giả: mỗi mẫu mô tả được setup và assertion quan sát; reviewer độc lập vẫn PENDING. Những action cần remote provider dùng mock receipt trước, sau đó chạy lại ở staging có quyền theo gate, không tự suy ra live PASS.
