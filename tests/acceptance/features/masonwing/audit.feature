@masonwing @F-020 @C1
Feature: Quan sát, audit và support access
  Log có correlation nhưng không chứa secrets hay nội dung không cần thiết.

  @REQ-134 @AC-138
  Scenario: Trace propagation
    Given run R có trace T
    When invoke remote step
    Then trace chứa parent relation R/T và plugin digest; không dùng tenant text làm metric label

  @REQ-135 @AC-139
  Scenario: Audit bất biến
    Given runtime token; event E tồn tại
    When update/delete E
    Then AUDIT_IMMUTABLE; original digest không đổi

  @REQ-136 @AC-140
  Scenario: Secret redaction
    Given canary ở token, error body, header, tool args
    When inject fault qua các sinks
    Then canary không xuất hiện trong logs, traces, metrics, exports, screenshots

  @REQ-137 @AC-141
  Scenario: Support có thời hạn
    Given tenant owner cấp 15 phút chỉ đọc metadata
    When support đọc content ngoài scope
    Then SUPPORT_SCOPE_DENIED; metadata hợp lệ có audit; expiry chặn toàn access

  @REQ-138 @AC-142
  Scenario: Không thành công giả khi thiếu audit
    Given audit datastore lỗi write
    When publish request
    Then AUDIT_UNAVAILABLE; transmit=0; không success toast

  @REQ-139 @AC-143
  Scenario: Alert có hành động
    Given DLQ vượt L-ALERT threshold
    When evaluate alert
    Then alert có correlation, owner Ops, runbook ref và side-effect state
