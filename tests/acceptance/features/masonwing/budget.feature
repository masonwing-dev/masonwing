@masonwing @F-014 @C1
Feature: Ngân sách, quota và fairness
  Fan-out không được vượt giới hạn do đua cập nhật hoặc coi unknown cost bằng zero.

  @REQ-092 @AC-096
  Scenario: Reserve trước provider
    Given budget còn 100 units; request upper_bound=20
    When dispatch
    Then reservation 20 tồn tại trước provider call; balance khả dụng 80

  @REQ-093 @AC-097
  Scenario: Race budget
    Given balance=100; hai request cùng đòi 80
    When reserve đồng thời
    Then một reservation 80; một BUDGET_EXCEEDED; tổng reserved<=100

  @REQ-094 @AC-098
  Scenario: Usage chưa biết
    Given provider timeout sau acceptance
    When settle attempt
    Then cost_state UNKNOWN; không giải phóng reservation như zero; reconcile queued

  @REQ-095 @AC-099
  Scenario: Settlement idempotent
    Given usage 12 cho reservation20
    When gửi receipt usage hai lần
    Then charged12; released8; lần replay không trừ thêm

  @REQ-096 @AC-100
  Scenario: Không có upper bound
    Given model profile không có output ceiling hoặc tool price cap
    When admit
    Then PRICE_BOUND_UNKNOWN; billable calls=0

  @REQ-097 @AC-101
  Scenario: Tenant fairness
    Given tenant A đầy slots; B còn share
    When submit A và B
    Then A queued hoặc 429 theo backlog; B vẫn được dispatch

  @REQ-098 @AC-102
  Scenario: BYOK vẫn kiểm quota
    Given BYOK profile; platform daily limit hết
    When dispatch
    Then QUOTA_EXCEEDED; customer provider calls=0
