@masonwing @F-019 @C1
Feature: Tìm kiếm, thông báo và export có quyền
  Read surfaces không trở thành đường bypass quyền hoặc rò bí mật.

  @REQ-128 @AC-132
  Scenario: Search có tenant filter
    Given index chứa A/B cùng keyword
    When A search
    Then B không có trong hits, facets, snippets hay counts

  @REQ-129 @AC-133
  Scenario: Pagination ổn định
    Given nhiều records cùng timestamp; cursor page1
    When request page2
    Then không lặp/bỏ record trong snapshot; cursor đổi filters trả CURSOR_MISMATCH

  @REQ-130 @AC-134
  Scenario: Export snapshot
    Given auditor export 100 records
    When start rồi cập nhật source
    Then export thể hiện snapshot đã pin; manifest có schema/digest và data classification

  @REQ-131 @AC-135
  Scenario: Chống CSV formula injection
    Given cell bắt đầu =,+,-,@,tab,CR
    When export CSV
    Then mở fixture không thực thi formula; raw value có trong safe JSON export được auth

  @REQ-132 @AC-136
  Scenario: Notification không lộ nội dung
    Given event có secret canary và restricted draft
    When send notification
    Then chỉ summary được phép và deep link; canary=0 trong payload

  @REQ-133 @AC-137
  Scenario: Read status không làm side effect nghiệp vụ
    Given notification liên kết approval pending
    When mark read
    Then approval vẫn PENDING; publish calls=0
