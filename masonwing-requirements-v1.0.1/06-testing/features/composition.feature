@masonwing @F-001 @C1
Feature: Lắp ghép sản phẩm, không fork kernel
  Ứng dụng mới chọn năng lực qua product manifest; nghiệp vụ không xâm nhập kernel.

  @REQ-001 @AC-001
  Scenario: Lắp ghép sản phẩm
    Given product A bật document-review; marketing không được cài
    When khởi động product A
    Then route document-review tồn tại; route marketing trả 404; không tạo bảng marketing

  @REQ-002 @AC-002
  Scenario: Không rò nghiệp vụ vào kernel
    Given kernel digest K và hai fixture document-review, marketing
    When build hai sản phẩm dùng K
    Then kernel digest giống nhau; cả hai fixture hoàn thành workflow được khai báo

  @REQ-003 @AC-003
  Scenario: Không bắt buộc AI
    Given fixture checksum chỉ có input artifact và deterministic handler
    When thực thi workflow
    Then output hash đúng fixture; provider calls=0; không yêu cầu model key

  @REQ-004 @AC-004
  Scenario: Cấu hình không hợp lệ
    Given manifest khai báo capability không có trong registry
    When activate sản phẩm
    Then CONFIG_UNSUPPORTED; readiness=false; tenant routes chưa được phục vụ

  @REQ-005 @AC-005
  Scenario: Bản đóng gói tái sử dụng
    Given sample app bên ngoài monorepo dùng package phát hành
    When build và chạy conformance smoke
    Then không có source fork; compatible contract hoạt động; digest package có trong lockfile
