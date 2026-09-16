@masonwing @F-010 @C1
Feature: Artifact, nguồn gốc và vòng đời dữ liệu
  Evidence, receipts và dữ liệu riêng tư có version, rights và chính sách xóa rõ ràng.

  @REQ-061 @AC-064
  Scenario: Artifact bất biến
    Given upload bytes B rồi finalize
    When đọc và thử ghi lại cùng artifact ID
    Then digest=SHA256(B); overwrite trả IMMUTABLE_ARTIFACT

  @REQ-062 @AC-065
  Scenario: Signed URL có scope
    Given user được đọc A, không B
    When request download A
    Then locator chỉ đọc A; expires theo L-SIGNEDURL; không liệt kê bucket

  @REQ-063 @AC-066
  Scenario: Chống nội dung upload nguy hiểm
    Given polyglot HTML/image hoặc malware fixture
    When finalize upload
    Then QUARANTINED; preview, model retrieval, public link=0

  @REQ-064 @AC-067
  Scenario: Retraction propagation
    Given source S được dùng bởi derived D
    When revoke S
    Then D marked INVALIDATED; retrieval không trả D; active workflows chuyển BLOCKED

  @REQ-065 @AC-068
  Scenario: Xóa tới dữ liệu dẫn xuất
    Given subject data có cache, index, export, external publication
    When execute deletion
    Then active cache/index bị xóa; external references ghi pending action; không báo xóa toàn bộ khi chưa chứng minh

  @REQ-066 @AC-069
  Scenario: Legal hold
    Given retention hết hạn nhưng hold active
    When retention job
    Then HELD; dữ liệu không bị xóa; audit thể hiện hold owner

  @REQ-067 @AC-070
  Scenario: Backup không hồi sinh dữ liệu xóa
    Given backup trước yêu cầu xóa S; tombstone sau đó
    When restore
    Then S không đọc được trước readiness; restore report ghi tombstone replay

  @REQ-068 @AC-071
  Scenario: Mã hóa payload riêng tư
    Given provider conversation/token blob có synthetic secret; database backup được tạo
    When inspect backup without secret-manager key
    Then không giải mã được secret; key material không có trong app database dump
