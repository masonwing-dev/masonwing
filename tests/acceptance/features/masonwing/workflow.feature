@masonwing @F-011 @C1
Feature: Durable workflow, checkpoint và bounded agent loop
  Workflow replay không lặp I/O tùy tiện; thời gian và vòng lặp có giới hạn.

  @REQ-069 @AC-072
  Scenario: Bắt đầu run có idempotency — case 1
    Given idempotency key K và fingerprint P chưa tồn tại
    When POST /runs hai lần với K,P
    Then cùng run ID; một root execution

  @REQ-069 @AC-073
  Scenario: Bắt đầu run có idempotency — case 2
    Given K đã dùng với fingerprint P1
    When POST /runs với K,P2
    Then 409 IDEMPOTENCY_CONFLICT; không tạo run mới

  @REQ-070 @AC-074
  Scenario: Phục hồi sau crash
    Given worker chết sau completed checkpoint và trước bước kế
    When restart worker
    Then bước hoàn tất không bị ghi thành pending; bước kế được điều phối một lần theo logical ID

  @REQ-071 @AC-075
  Scenario: Chờ phê duyệt không giữ worker
    Given 100 run WAITING_APPROVAL
    When đọc capacity và approve một run
    Then 100 durable waits; worker slot không bị chiếm bởi idle waits; đúng run được resume

  @REQ-072 @AC-076
  Scenario: Giới hạn vòng lặp
    Given profile max_turns=8; model liên tục xin thêm tool
    When thực thi lượt vượt giới hạn
    Then LIMIT_REACHED; model/tool calls sau limit=0

  @REQ-073 @AC-077
  Scenario: Transition trái phép
    Given run SUCCEEDED
    When submit resume
    Then 409 ILLEGAL_TRANSITION; state SUCCEEDED không đổi

  @REQ-074 @AC-078
  Scenario: Cancel phân biệt đã gửi
    Given remote request đã qua point of no return
    When cancel run
    Then CANCEL_REQUESTED; không báo effect undone; reconciliation vẫn được phép

  @REQ-075 @AC-079
  Scenario: Replay deterministic
    Given v2 replay history v1 không tương thích
    When resume
    Then REPLAY_INCOMPATIBLE; transmit=0; operator có history reference

  @REQ-076 @AC-080
  Scenario: Workflow definition validation
    Given workflow gọi handler không tồn tại hoặc cycle không có loop bound
    When install workflow definition
    Then WORKFLOW_INVALID; created runs=0; no model/provider calls

  @REQ-077 @AC-081
  Scenario: Workflow input/output validation
    Given node A trả object thiếu field cho node B
    When complete A
    Then NODE_OUTPUT_INVALID; B chưa dispatch; invalid result retained as quarantined diagnostic
