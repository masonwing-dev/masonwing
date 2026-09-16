@masonwing @F-018 @C1
Feature: React shell và UI plugin
  Một design system với extension slots; untrusted UI có origin boundary.

  @REQ-121 @AC-125
  Scenario: Host sở hữu chrome
    Given hai domain plugins dùng UI SDK
    When đi qua routes và mở dialog
    Then một h1/route; shared components; không thêm global header thứ hai

  @REQ-122 @AC-126
  Scenario: Contribution có namespace
    Given hai plugin khai báo cùng route /settings
    When load contributions
    Then UI_CONTRIBUTION_CONFLICT; không overwrite route im lặng

  @REQ-123 @AC-127
  Scenario: Untrusted iframe
    Given iframe origin khác; fixture cố đọc host DOM và gửi forged message
    When open contribution
    Then browser chặn DOM; bridge từ chối origin/schema; privileged calls=0

  @REQ-124 @AC-128
  Scenario: Permission change UI
    Given Editor được downgrade Viewer khi đang mở editor
    When nhận permission refresh
    Then Publish/Edit bị disable hoặc ẩn; cached data tenant khác không hiện

  @REQ-125 @AC-129
  Scenario: UI states
    Given mỗi state fixture trong UI matrix
    When render route
    Then title/recovery control đúng state; không hiển thị giả dữ liệu khi error

  @REQ-126 @AC-130
  Scenario: Deep link phục hồi
    Given chưa login; deep link resource A
    When login thành công
    Then đến A sau auth; nếu không có quyền trả 404 thay vì landing giả

  @REQ-127 @AC-131
  Scenario: Không mất draft im lặng
    Given editor dirty; autosave failed
    When navigate away
    Then stay/discard dialog; cancel giữ draft; không tự publish hoặc ghi đè server
