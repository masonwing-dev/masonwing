@masonwing @F-021 @C1
Feature: Backup, deploy, migration và supply chain
  Đưa bản build theo digest qua môi trường; phục hồi phải đối soát tác động bên ngoài.

  @REQ-140 @AC-144
  Scenario: Build provenance
    Given CI build candidate
    When publish candidate to staging registry
    Then SBOM và provenance tham chiếu cùng digest; không dùng mutable latest trong deployment manifest

  @REQ-141 @AC-145
  Scenario: Tách môi trường
    Given staging service token và secret path
    When request production data
    Then ENVIRONMENT_DENIED; data returned=0

  @REQ-142 @AC-146
  Scenario: Migration preflight
    Given destructive migration thiếu backup receipt
    When migrate
    Then MIGRATION_PREFLIGHT_FAILED; schema không đổi

  @REQ-143 @AC-147
  Scenario: Restore kiểm toàn vẹn
    Given backup Postgres/object store/history khác watermark
    When restore
    Then write readiness=false đến khi repair/reconcile hoàn tất; dangling refs được report

  @REQ-144 @AC-148
  Scenario: Health đúng dependency
    Given policy hoặc secret broker unavailable
    When GET /health/ready
    Then non-200 write readiness; cached public static page vẫn có thể phục vụ

  @REQ-145 @AC-149
  Scenario: License policy
    Given SBOM có license bị cấm hoặc unknown
    When release gate
    Then LICENSE_REVIEW_REQUIRED; không promote; không tự khẳng định OSS

  @REQ-146 @AC-150
  Scenario: Restore không retry effect đã gửi
    Given backup có PREPARED nhưng provider đã xử lý sau backup
    When resume restore
    Then OUTCOME_UNKNOWN/RECONCILING; không resend từ trạng thái cũ
