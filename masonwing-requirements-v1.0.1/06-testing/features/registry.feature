@masonwing @F-002 @C1
Feature: Danh tính plugin và dependency resolution
  Chỉ nạp artifact có provenance, contract và dependency graph hợp lệ.

  @REQ-006 @AC-006
  Scenario: Manifest có phiên bản
    Given manifest fixture hợp lệ và schema 1.0.0
    When verify manifest
    Then kết quả VALID gắn schema version và artifact digest

  @REQ-007 @AC-007
  Scenario: Chu trình dependency
    Given plugin A cần B và B cần A
    When resolve graph
    Then DEPENDENCY_CYCLE kèm đường chu trình; migrations=0; installed registry không đổi

  @REQ-008 @AC-008
  Scenario: Dependency thiếu
    Given plugin A yêu cầu contract x@2; chỉ có x@1
    When resolve A
    Then DEPENDENCY_UNSATISFIED; không tải runtime code

  @REQ-009 @AC-009
  Scenario: Artifact bị tráo
    Given artifact bytes đã sửa sau ký
    When install plugin
    Then ARTIFACT_INTEGRITY; handler calls=0; artifact QUARANTINED

  @REQ-010 @AC-010
  Scenario: Lockfile tái lập
    Given registry có hai phiên bản tương thích
    When resolve rồi resolve với lockfile đã sinh
    Then lần hai chọn cùng digests; không tự lấy bản mới

  @REQ-011 @AC-011
  Scenario: Phạm vi extension point
    Given module A truy cập điểm riêng của B
    When register contribution
    Then EXTENSION_SCOPE_DENIED; B không bị thay handler
