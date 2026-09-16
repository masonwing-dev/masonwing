@masonwing @F-015 @C1
Feature: Provider adapter, OpenAI Responses và compaction
  Bảo toàn provider-native state; không coi OpenAI-compatible là cùng mọi capability.

  @REQ-099 @AC-103
  Scenario: Capability negotiation
    Given task cần Responses compact; adapter chỉ Chat Completions
    When plan task
    Then CAPABILITY_UNSUPPORTED; không downgrade ngầm sang plain text

  @REQ-100 @AC-104
  Scenario: Native envelope fidelity
    Given fixture chứa message.phase, reasoning, tool call, compaction và unknown opaque item
    When persist rồi reload
    Then canonical native payload giữ nguyên các fields/items; không leak native content vào public logs

  @REQ-101 @AC-105
  Scenario: Standalone compact window
    Given compact response có 3 ordered items gồm opaque compaction
    When continue conversation
    Then next input chứa đủ 3 items đúng thứ tự; không chỉ dùng compact item

  @REQ-102 @AC-106
  Scenario: Server compaction mode
    Given profile chọn manual history với server compaction
    When gửi hai lượt liên tiếp
    Then không trộn previous_response_id với duplicate manual history; native compaction output còn được giữ

  @REQ-103 @AC-107
  Scenario: Không thay business truth
    Given xóa provider session sau khi run đã có approval/evidence
    When đọc business records
    Then approval/evidence/receipt vẫn tồn tại; không reconstruct từ model summary

  @REQ-104 @AC-108
  Scenario: Đổi provider
    Given opaque state provider A; chuyển B
    When continue
    Then B không nhận opaque blob A; context có source refs hợp lệ và thông báo continuity boundary

  @REQ-105 @AC-109
  Scenario: Structured output lỗi
    Given tool arguments malformed hoặc output thiếu required field
    When consume output
    Then MODEL_OUTPUT_INVALID; tool calls=0; repair attempts giới hạn

  @REQ-106 @AC-110
  Scenario: Truncated stream
    Given SSE bị cắt giữa JSON tool call
    When process stream
    Then INCOMPLETE; tool dispatch=0; cost UNKNOWN hoặc usage thực có bằng chứng

  @REQ-107 @AC-111
  Scenario: Provider privacy gate
    Given restricted source; provider chưa có approved retention/region policy
    When invoke
    Then DATA_POLICY_DENIED; request bytes sent=0
