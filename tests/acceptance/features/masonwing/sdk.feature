@masonwing @F-023 @C2
Feature: SDK, conformance và developer experience
  Tác giả plugin khác xây được mà không sửa core hoặc biết internals.

  @REQ-153 @AC-157
  Scenario: Scaffold plugin
    Given clean workspace; plugin namespace com.example.review
    When CLI init
    Then generated project validate thành công; không chứa marketing import

  @REQ-154 @AC-158
  Scenario: Contract compatibility
    Given host major1; plugin requires major2
    When load
    Then CONTRACT_INCOMPATIBLE có supported range; handler calls=0

  @REQ-155 @AC-159
  Scenario: Pack deterministic
    Given hai clean builds cùng lock/source/config
    When pack hai lần
    Then unsigned content digests giống nhau; timestamp/signature envelope không lẫn payload reproducibility

  @REQ-156 @AC-160
  Scenario: Conformance report có identity
    Given artifact D, suite version S
    When run conformance
    Then report D/S/env; NOT_RUN/skipped không được tính PASS

  @REQ-157 @AC-161
  Scenario: Dev profile không production
    Given auth mock hoặc unsigned plugin=true; env production
    When start
    Then UNSAFE_CONFIGURATION; readiness=false

  @REQ-158 @AC-162
  Scenario: Adapter thay thế không fork
    Given fake IdP và storage adapter dùng cùng contracts
    When run portability suite
    Then domain plugin digests không đổi; identity/data assertions giống nhau
