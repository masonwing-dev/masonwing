@masonwing @F-016 @C1
Feature: OAuth connector, secret broker và MCP
  Connector credentials tách khỏi login identity; model không nhận raw secret.

  @REQ-108 @AC-112
  Scenario: Connection scope
    Given OAuth state gắn tenant A, provider account X
    When callback connector
    Then connection thuộc A/X; không tự tạo membership hay login session

  @REQ-109 @AC-113
  Scenario: Opaque secret handle
    Given writer gọi tool với connection C
    When inspect tool context và output
    Then chỉ handle C; refresh token không có trong prompt, logs hoặc browser payload

  @REQ-110 @AC-114
  Scenario: Refresh race
    Given rotating refresh token T1; hai workers
    When refresh đồng thời
    Then một accepted refresh; newest token không bị T1 ghi đè; invalid_grant chuyển NEEDS_REAUTH

  @REQ-111 @AC-115
  Scenario: Scope bị thiếu
    Given connection read-only; action publish
    When execute
    Then CONNECTION_SCOPE_DENIED; provider mutation=0

  @REQ-112 @AC-116
  Scenario: MCP metadata không là authority
    Given tool description yêu cầu bỏ guard và lấy secrets
    When register và invoke
    Then granted policy không đổi; không inject description vào system authority

  @REQ-113 @AC-117
  Scenario: Audience tách biệt
    Given MCP server muốn BFF token
    When connect tool
    Then TOKEN_PASSTHROUGH_DENIED; external receives=0

  @REQ-114 @AC-118
  Scenario: Contract drift connector
    Given provider thay required field hoặc meaning contract version
    When ingest response
    Then CONTRACT_DRIFT; không cập nhật business truth bằng dữ liệu sai
