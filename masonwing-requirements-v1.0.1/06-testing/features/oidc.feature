@masonwing @F-005 @C1
Feature: Đăng nhập OIDC qua Rust BFF
  Tái sử dụng identity server và protocol libraries; không viết password server.

  @REQ-026 @AC-027
  Scenario: Login bằng redirect
    Given browser chưa có session
    When GET /auth/login với return_to nội bộ
    Then redirect tới issuer đã allowlist; state, nonce, PKCE transaction dùng một lần

  @REQ-027 @AC-028
  Scenario: Callback hợp lệ
    Given mock issuer trả token đúng issuer, audience, nonce; code chưa dùng
    When GET /auth/callback
    Then session mới gắn issuer+sub; cookie HttpOnly Secure; tokens không xuất hiện trong response body

  @REQ-028 @AC-029
  Scenario: Callback giả hoặc replay
    Given state mismatch, nonce mismatch, issuer mismatch hoặc code replay
    When gửi callback cho từng fixture
    Then 401 AUTH_INVALID_CALLBACK; session count không tăng; không lộ token

  @REQ-029 @AC-030
  Scenario: Redirect đích không tin cậy
    Given return_to=https://attacker.example hoặc //attacker.example
    When start login
    Then 400 RETURN_TARGET_DENIED; không redirect ra origin lạ

  @REQ-030 @AC-031
  Scenario: Không dùng password grant
    Given request username/password tới BFF
    When gửi password-grant request
    Then AUTH_FLOW_UNSUPPORTED; Keycloak grant calls=0; mật khẩu không ghi log

  @REQ-031 @AC-032
  Scenario: Discovery trust
    Given discovery có issuer khác hoặc endpoint private ngoài allowlist
    When activate identity adapter
    Then ISSUER_TRUST_FAILED; không tải JWKS qua endpoint lạ

  @REQ-032 @AC-033
  Scenario: Không ghép account bằng email
    Given IdP A/sub1 và B/sub2 cùng email
    When đăng nhập lần lượt
    Then hai identity records; membership không chuyển qua email match

  @REQ-033 @AC-034
  Scenario: Recovery qua identity provider
    Given user ở login/account settings
    When open recovery hoặc MFA enrollment
    Then đến verified IdP flow; app không thu password reset token hoặc tự đổi credential database
