# Tracking adoption — không tự khởi tạo

Bộ ZIP nguồn phân biệt SRS create và track-setup. Yêu cầu hiện tại tạo hai requirements packs,không phải cấp quyền khởi tạo ledger,change branch protection hoặc deploy CI. Vì vậy gói này có work-package **kế hoạch**,test designs,trace matrix và run-evidence schema,nhưng không active progress ledger,không VERIFIED claims,không fake review.

Full source skill/templates được giữ nguyên trong `vendor/requirements-spec/`. Khi được cấp quyền triển khai tracking,hãy tìm tracker hiện hữu và tiếp tục nó;không reseed lịch sử. Chỉ chọn một work source authoritative. Adopt pinned policy bằng change-control,không vì global skill đã đổi mà cập nhật policy đang chạy.

## Phải giữ các gate semantics

G-READY:exact criteria,owner/reviewer/coordinator,confirmed decisions và authority. G-TRACE:unique IDs,partition/backlinks,source snapshots. G-IMPACT:changed files mapped,test selection kể cả shared paths. G-VERIFY:matching evidence,actual assertions,independent review,no blockers. G-MERGE:exact candidate tree và authority. G-RELEASE:selected scope gồm NFR,live/security/restore/perf và deployment approval.

Any target empty,zero tests,missing mapping,missing artifacts,stale tree,renamed author-as-reviewer,local self-consistent fake hashes hoặc unknown authority => not verified. Current fail defeats old pass. Work completion khác evidence verification. N/A hoặc scope removal phải có approved change request. Publication concurrent writers cần serialized head/CAS và immutable history;không dựng fake head chain trong documents để giả enforce.

## Deliverables khi track-setup được authorized

Use full `tracking.md`, `tracking-contract.md`, `tracking-gates.md`;seed exact source IDs/counts,status unverified;record proposed vs accepted policy;keep independent review pending if unavailable. Templates trong vendor là ví dụ,không thuộc product trace inventory. Không tính fixture good.md của validator là SRS sản phẩm.
