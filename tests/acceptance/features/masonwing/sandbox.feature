@masonwing @F-004 @C1
Feature: Cô lập Wasm, native và remote worker
  Trust class và giới hạn tài nguyên được host áp dụng, không do plugin tự quyết.

  @REQ-019 @AC-019
  Scenario: Không tự nâng trust
    Given tenant uploader gửi native artifact
    When install
    Then TRUST_CLASS_DENIED; process library chưa nạp

  @REQ-020 @AC-020
  Scenario: Capability handles
    Given Wasm plugin chỉ có artifact.read cho artifact A
    When plugin đọc artifact B
    Then RESOURCE_DENIED; nội dung B không có trong result hay logs

  @REQ-021 @AC-021
  Scenario: Ngắt vòng lặp vô hạn
    Given Wasm fixture vòng lặp vô hạn; profile L-WASM
    When invoke
    Then SANDBOX_LIMIT; host health vẫn OK; invocation kết thúc trong deadline profile

  @REQ-022 @AC-022
  Scenario: Host I/O bị treo
    Given mock host HTTP không trả body
    When plugin chờ HTTP
    Then HOST_IO_TIMEOUT trong giới hạn; không giữ worker lease vô hạn

  @REQ-023 @AC-023
  Scenario: Egress chống SSRF — case 1
    Given URL đổi DNS sang loopback hoặc metadata address
    When request qua egress broker
    Then EGRESS_DENIED; TCP tới địa chỉ cấm=0

  @REQ-023 @AC-024
  Scenario: Egress chống SSRF — case 2
    Given redirect HTTPS từ public sang private subnet
    When follow redirect qua broker
    Then EGRESS_DENIED; credentials không chuyển sang origin khác

  @REQ-024 @AC-025
  Scenario: RPC có fence
    Given worker W1 lease=1 hết hạn; W2 có lease=2
    When W1 gửi completion
    Then STALE_FENCE; output W2 không bị ghi đè

  @REQ-025 @AC-026
  Scenario: Dọn sandbox
    Given invocation tạo temp artifact và handle H
    When invocation kết thúc
    Then H không dùng lại được; temp không còn sau cleanup deadline; retained artifact không bị xóa
