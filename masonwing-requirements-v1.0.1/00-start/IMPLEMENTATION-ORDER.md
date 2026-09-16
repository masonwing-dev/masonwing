# Trình tự triển khai

Mode: **create**. Bộ này tạo SRS và kế hoạch kiểm chứng; không khởi tạo tracker, sửa repository, bật CI hoặc triển khai dịch vụ.

## Đọc và sử dụng

1. Đọc README, SRS, source-manifest và assumptions trước. Chuỗi authority: yêu cầu trực tiếp của người dùng > source zip về format/quality > scope được biểu đạt trong nghiên cứu và trao đổi > các lựa chọn tác giả đề xuất. Thông tin external technical chỉ dùng phần được xác minh trong web-references.
2. Chọn một release partition, đọc feature/REQ/AC của work package và contracts được pin. Không xóa phần chưa làm khỏi global scope.
3. Bootstrap toolchain và lock dependency candidate. Chạy contract lint trước khi viết domain handlers. Không đưa secret thật vào fixtures.
4. Viết failing tests cho AC, deny paths và transition matrix; triển khai đúng public boundary; thu evidence thật từ candidate.
5. QA độc lập kiểm assertion, fixture, artifact hashes và UI top/end. Không tăng trạng thái VERIFIED nếu chỉ có generic success hoặc không thấy provider counters.
6. Qua qualification local trước; live external gates có credentials/pháp lý/owner approval riêng. Không được dùng report do mock tạo làm bằng chứng live.

## Phạm vi sẵn sàng

Các hành vi, input/output, model dữ liệu, state transitions, lỗi, quyền, AC, test fixtures, work packages và release gates được mô tả cụ thể để bắt đầu lập trình. Giá trị cấu hình do tác giả chọn đều là **Proposed**, có owner trong assumptions. Không tự coi việc yêu cầu viết spec là chấp thuận mọi business policy hay quyền xuất bản.

SRS là **Draft — buildable proposed baseline** cho local implementation. Chưa freeze vì chưa có quyết định của Product Owner về các giả định. Freeze tài liệu không phải production PASS. Source code, integration test results, performance/DR evidence và independent review sản phẩm chưa tồn tại trong gói này.

## Khi gặp mâu thuẫn

Dừng hành vi bị ảnh hưởng; mở change request với IDs, nguồn, giải pháp và impact test. Không tự đổi public contract để code dễ hơn; không sửa threshold, bỏ test, thêm skip hay fake data để đạt gate. Một bản fix yêu cầu mapping/evidence mới; giữ kết quả cũ làm lịch sử.

## Hai repository

Masonwing xuất SDK/crates/npm packages/images và contract bundle. Gleanbird pin contract SHA và consume releases; không copy kernel sang sản phẩm. Bản shared-contracts.json trong hai ZIP phải có cùng SHA-256. Tác giả marketing dùng consumer conformance để phát hiện drift. Không yêu cầu fork source khi tạo domain thứ hai.
