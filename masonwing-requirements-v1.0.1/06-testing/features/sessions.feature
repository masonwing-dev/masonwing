@masonwing @F-006 @C1
Feature: Session, membership và tổ chức
  Quản lý phiên phía server, invitation và quyền tenant dùng chung framework.

  @REQ-034 @AC-035
  Scenario: Chống session fixation
    Given session guest có ID S
    When login hoặc hoàn tất step-up
    Then ID mới khác S; S không dùng được để gọi API

  @REQ-035 @AC-036
  Scenario: CSRF cho cookie API
    Given attacker origin hoặc token thiếu
    When POST request ghi dữ liệu
    Then 403 CSRF_REJECTED; state version không đổi

  @REQ-036 @AC-037
  Scenario: Expiry phiên
    Given virtual clock đúng expiry hoặc vượt expiry
    When GET /session
    Then 401 SESSION_EXPIRED; không gia hạn phiên đã hết hạn

  @REQ-037 @AC-038
  Scenario: Logout ứng dụng
    Given session S đang active
    When POST /auth/logout
    Then S bị revoke; API dùng S trả 401

  @REQ-038 @AC-039
  Scenario: Backchannel logout — case 1
    Given logout token có sid hợp lệ đúng issuer/audience
    When POST /auth/backchannel-logout
    Then các phiên matching bị revoke; unrelated sessions còn active

  @REQ-038 @AC-040
  Scenario: Backchannel logout — case 2
    Given logout token sai chữ ký hoặc replay jti
    When gửi logout event
    Then sai chữ ký bị từ chối; replay idempotent; không revoke unrelated subject

  @REQ-039 @AC-041
  Scenario: Invitation một lần
    Given invite đúng tenant/email đã verify; chưa hết hạn
    When hai request accept đồng thời
    Then một membership; response replay trả cùng membership; không thêm role khác

  @REQ-040 @AC-042
  Scenario: Owner cuối cùng
    Given tenant có đúng một owner
    When remove hoặc demote owner
    Then 409 LAST_OWNER; membership không đổi

  @REQ-041 @AC-043
  Scenario: Thu hồi phiên trên giao diện
    Given browser đang mở tenant A với SSE và cache
    When admin revoke membership
    Then SSE tenant A đóng theo L-REVOKE; cache A bị xóa; deep link trả 404
