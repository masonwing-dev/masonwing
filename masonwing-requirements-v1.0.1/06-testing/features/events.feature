@masonwing @F-017 @C1
Feature: Scheduler, event bus và replay
  At-least-once delivery, timezone và thời hạn được thể hiện rõ.

  @REQ-115 @AC-119
  Scenario: Schedule timezone
    Given schedule timezone America/New_York qua DST; profile skip-gap/run-once-fold
    When advance clock qua các boundary
    Then số occurrence đúng schedule policy; không có run kép do clock fold

  @REQ-116 @AC-120
  Scenario: Overlap schedule
    Given schedule coalesce_latest; run trước chưa xong
    When ba tick tiếp theo
    Then chỉ một occurrence pending mới nhất; missed_count=3 được audit

  @REQ-117 @AC-121
  Scenario: Event dedupe
    Given same event_id gửi hai lần
    When consume
    Then inbox có một receipt; logical business mutation một lần

  @REQ-118 @AC-122
  Scenario: Dead letter
    Given subscriber trả 503 hết retries
    When process budget cuối
    Then DEAD_LETTER cùng reason/attempt history; operator có replay action được auth

  @REQ-119 @AC-123
  Scenario: SSE replay authorization
    Given client trước có quyền A, nay bị revoke; Last-Event-ID cũ
    When connect SSE
    Then không replay tenant A events; AUTHZ_DENIED hoặc stream close

  @REQ-120 @AC-124
  Scenario: Replay cursor quá cũ
    Given cursor trước retention window
    When reconnect
    Then 410 SNAPSHOT_REQUIRED; client refetch; không giả stream đầy đủ
