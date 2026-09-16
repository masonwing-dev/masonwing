@masonwing @F-003 @C1
Feature: Vòng đời, nâng cấp và thu hồi plugin
  Nâng cấp không phá run đang chạy; disable và xóa dữ liệu là hai quyết định khác nhau.

  @REQ-012 @AC-012
  Scenario: Cài nhưng chưa chạy
    Given manifest đã verify; migration additive thành công
    When commit install
    Then state INSTALLED_DISABLED; handler chưa nhận việc

  @REQ-013 @AC-013
  Scenario: Quyền khi bật plugin
    Given plugin xin read và publish; admin chỉ cấp read
    When enable rồi gọi publish
    Then read hoạt động; publish CAPABILITY_DENIED; provider mutation=0

  @REQ-014 @AC-014
  Scenario: Pin phiên bản run
    Given v1 đang active; tạo run R; sau đó cài v2
    When resume R
    Then R dùng digest v1; run mới dùng v2 theo rollout policy

  @REQ-015 @AC-015
  Scenario: Drain trước disable
    Given v1 có một run đang chạy
    When disable v1
    Then new run bị PLUGIN_DRAINING; run cũ tiếp tục theo cancellation policy

  @REQ-016 @AC-016
  Scenario: Thu hồi khẩn cấp
    Given run v1 đã pin nhưng signer hoặc digest bị revoke
    When gọi host action kế tiếp
    Then PLUGIN_REVOKED; external calls=0 sau điểm enforcement; run chuyển BLOCKED

  @REQ-017 @AC-017
  Scenario: Giữ dữ liệu khi uninstall
    Given run hoàn tất còn liên kết evidence thuộc plugin
    When uninstall với chế độ retain
    Then UI plugin biến mất; audit và evidence vẫn truy xuất bởi auditor được phép

  @REQ-018 @AC-018
  Scenario: Migration không rollback được
    Given migration ghi dữ liệu không tương thích v1
    When operator yêu cầu rollback v1
    Then ROLLBACK_UNSAFE; không deploy v1; cung cấp forward-repair runbook
