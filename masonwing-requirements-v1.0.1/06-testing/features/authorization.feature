@masonwing @F-007 @C1
Feature: Cedar, tenant isolation và quyền theo tài nguyên
  Mọi API, tool và action đều dùng quyền hiện hành từ nguồn có thẩm quyền.

  @REQ-042 @AC-044
  Scenario: Deny mặc định
    Given user có membership nhưng action chưa được grant
    When gọi action
    Then 403 AUTHZ_DENIED; mutation=0

  @REQ-043 @AC-045
  Scenario: Forbid ưu tiên
    Given policy permit publish và forbid frozen-tenant
    When publish trong frozen tenant
    Then 403 AUTHZ_DENIED; provider calls=0

  @REQ-044 @AC-046
  Scenario: Policy evaluation lỗi
    Given permit khớp; forbid thiếu attribute gây diagnostics
    When gọi action
    Then AUTHZ_EVALUATION_ERROR; không chạy effect; audit ghi diagnostic code không chứa secrets

  @REQ-045 @AC-047
  Scenario: Không lộ tenant khác
    Given user tenant A; ID thật của B và ID không tồn tại
    When GET từng ID
    Then cùng 404 RESOURCE_NOT_FOUND shape; không lộ tên, existence, signed URL hay result count

  @REQ-046 @AC-048
  Scenario: Quyền hiện hành cho write
    Given approval còn hạn nhưng publisher bị revoke trước dispatch
    When worker dispatch
    Then 403 AUTHZ_DENIED; mutation transmit=0

  @REQ-047 @AC-049
  Scenario: Không tin scope từ client
    Given body tenant_id=B nhưng route/session thuộc A
    When submit command
    Then TENANT_CONTEXT_MISMATCH; không chọn B theo body

  @REQ-048 @AC-050
  Scenario: Step-up cho tác vụ nhạy cảm
    Given owner có session thường; action secret.rotate
    When request action
    Then STEP_UP_REQUIRED; sau verified MFA dưới L-STEPUP thì được đánh giá quyền tiếp
