@masonwing @F-009 @C1
Feature: Quyền sở hữu dữ liệu và atomic audit
  Business state là authority; plugin không tự đọc SQL của domain khác.

  @REQ-055 @AC-057
  Scenario: Schema ownership
    Given plugin A không có query contract B
    When A đọc record nội bộ B
    Then DATA_SCOPE_DENIED; không trả raw SQL handle

  @REQ-056 @AC-058
  Scenario: Audit cùng transaction
    Given inject audit write failure trong mutation
    When submit update
    Then transaction rollback; không tồn tại business update thiếu audit

  @REQ-057 @AC-059
  Scenario: Optimistic concurrency — case 1
    Given resource version 2; request If-Match=1
    When update
    Then 409 STALE_VERSION; resource giữ v2

  @REQ-057 @AC-060
  Scenario: Optimistic concurrency — case 2
    Given hai writer cùng base version=2
    When gửi hai update đồng thời
    Then một update thành công v3; bên thua 409; không merge ngầm

  @REQ-058 @AC-061
  Scenario: RLS tenant
    Given RLS fixture A/B; runtime DB role không owner/BYPASSRLS
    When chạy negative isolation suite
    Then không đọc hoặc ghi record B từ tenant A kể cả connection reuse

  @REQ-059 @AC-062
  Scenario: Transactional outbox
    Given process bị kill ngay sau commit mutation
    When restart dispatcher
    Then business event được giao; không mất acknowledged intent

  @REQ-060 @AC-063
  Scenario: Input validation
    Given unknown field, payload quá lớn hoặc số ngoài range
    When gửi command cho từng vector
    Then 400 hoặc 413 theo error catalog; mutation=0
