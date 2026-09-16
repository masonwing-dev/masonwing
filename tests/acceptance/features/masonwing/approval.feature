@masonwing @F-012 @C1
Feature: Phê duyệt bất biến và race resolution
  Người duyệt biết chính xác nội dung và target sẽ bị thay đổi.

  @REQ-078 @AC-082
  Scenario: Approval bind digest
    Given proposal P hash H target A v2
    When approve P
    Then approval record chứa H,A,v2 và actor; không cấp blanket permission

  @REQ-079 @AC-083
  Scenario: Nội dung thay sau duyệt
    Given approved H1; draft hiện tại H2
    When dispatch
    Then APPROVAL_STALE; transmit=0

  @REQ-080 @AC-084
  Scenario: Expiry approval
    Given approval expires_at=T; authoritative time=T
    When dispatch
    Then APPROVAL_EXPIRED; transmit=0

  @REQ-081 @AC-085
  Scenario: Race approve cancel
    Given approval PENDING version1
    When approve và cancel đồng thời
    Then một terminal decision; loser 409 DECISION_CONFLICT; audit ghi winner

  @REQ-082 @AC-086
  Scenario: Tách người tạo và người duyệt
    Given policy requires distinct approver; author A
    When A approve
    Then 403 FOUR_EYES_REQUIRED; proposal còn PENDING

  @REQ-083 @AC-087
  Scenario: Preview đúng target
    Given proposal có before/after và target domain
    When mở approval UI
    Then UI hiển thị cùng digests trong API; không dùng latest draft để thay preview
