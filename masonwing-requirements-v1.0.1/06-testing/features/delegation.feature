@masonwing @F-008 @C1
Feature: Machine identity và ủy quyền agent
  Worker hợp lệ không đồng nghĩa có quyền mọi tenant.

  @REQ-049 @AC-051
  Scenario: Machine audience
    Given token staging dùng trên API production
    When worker gửi command
    Then 401 TOKEN_AUDIENCE_INVALID; command chưa enqueue

  @REQ-050 @AC-052
  Scenario: Phạm vi delegation
    Given run chỉ được draft article A; plugin có publish rộng hơn
    When tool publish A
    Then DELEGATION_DENIED; external calls=0

  @REQ-051 @AC-053
  Scenario: Automation grant tách session
    Given creator đã logout; automation grant còn active
    When scheduler tick
    Then run gắn grant ID và tenant; không sử dụng cookie creator

  @REQ-052 @AC-054
  Scenario: Grant hết hạn
    Given clock=expires_at; queued action chưa transmit
    When dispatch
    Then DELEGATION_EXPIRED; transmit=0

  @REQ-053 @AC-055
  Scenario: Không tăng quyền qua nội dung
    Given tool arguments chứa role=owner và tenant_id khác
    When execute tool proposal
    Then CAPABILITY_ESCALATION_DENIED; granted set không đổi

  @REQ-054 @AC-056
  Scenario: Không kế thừa mặc định giữa agent
    Given parent chỉ read A; child xin read A,B
    When spawn child
    Then child grant không có B; request vượt scope bị từ chối
