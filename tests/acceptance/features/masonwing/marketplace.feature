@masonwing @F-022 @C2
Feature: Catalog plugin, curation và marketplace
  Catalog riêng trước; marketplace công khai dùng cùng contracts và kiểm soát supply chain.

  @REQ-147 @AC-151
  Scenario: Catalog metadata
    Given publisher đã verify; artifact digest D
    When submit listing
    Then listing PENDING_REVIEW chứa D, capabilities, license, compatibility và support contact

  @REQ-148 @AC-152
  Scenario: Duyệt trước public
    Given listing chưa đủ reviews
    When request public install
    Then LISTING_NOT_APPROVED; artifact không được activated

  @REQ-149 @AC-153
  Scenario: Permission diff khi update
    Given v1 read; v2 thêm publish
    When update theo auto-update policy
    Then UPDATE_APPROVAL_REQUIRED; v1 còn hoạt động; publish không tự được cấp

  @REQ-150 @AC-154
  Scenario: Thu hồi phiên bản marketplace
    Given digest D đang listed và revoked
    When install D từ cache hoặc catalog
    Then ARTIFACT_REVOKED; không bypass bằng offline cache

  @REQ-151 @AC-155
  Scenario: Entitlement không là authorization
    Given tenant có license plugin nhưng user không có publish role
    When publish
    Then AUTHZ_DENIED; paid entitlement không cấp quyền nghiệp vụ

  @REQ-152 @AC-156
  Scenario: Không tự thu tiền
    Given catalog mở; commerce capability disabled
    When gọi checkout endpoint
    Then COMMERCE_DISABLED; charge/payout=0; free install vẫn theo policy
