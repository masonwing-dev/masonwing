@masonwing @F-013 @C1
Feature: Effect broker và đối soát kết quả chưa biết
  Mọi external write phải có intent, authorization, receipt hoặc trạng thái chưa xác định.

  @REQ-084 @AC-088
  Scenario: Intent trước transmit
    Given broker có authorized effect
    When dispatch với fault trước network
    Then effect intent tồn tại trước bất kỳ provider acceptance nào

  @REQ-085 @AC-089
  Scenario: Unknown timeout
    Given provider nhận request rồi cắt TCP trước response
    When dispatch
    Then OUTCOME_UNKNOWN; resend count=0; reconciliation queued

  @REQ-086 @AC-090
  Scenario: Receipt có bằng chứng
    Given remote operation ID đã tồn tại sau timeout
    When reconcile
    Then SUCCEEDED cùng remote ID/version/hash; không suy ra thành công từ HTTP retry count

  @REQ-087 @AC-091
  Scenario: Không chứng minh được absence
    Given provider không có key hoặc lookup đáng tin
    When reconcile
    Then MANUAL_REVIEW; không gửi lại; UI nói kết quả chưa xác định

  @REQ-088 @AC-092
  Scenario: Idempotency fingerprint
    Given K đã gắn content H1
    When submit K,H2
    Then 409 IDEMPOTENCY_CONFLICT; intent gốc không đổi

  @REQ-089 @AC-093
  Scenario: Kill switch trước PONR
    Given effect đã authorized nhưng chưa gửi; switch enabled
    When dispatch
    Then KILL_SWITCH_ACTIVE; transmit=0

  @REQ-090 @AC-094
  Scenario: Retry đã chứng minh không gửi
    Given connect failure before bytes sent; budget còn
    When schedule retry
    Then attempt dùng cùng effect identity; backoff hợp lệ; vượt budget vào DEAD_LETTER

  @REQ-091 @AC-095
  Scenario: Không gọi compensation là undo
    Given bài đã publish và user yêu cầu revert
    When submit compensation
    Then effect mới có approval riêng; original receipt giữ nguyên; UI không nói lịch sử đã bị xóa
