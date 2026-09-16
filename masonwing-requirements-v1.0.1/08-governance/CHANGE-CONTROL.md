# Change control

ID stable trong mỗi product namespace. Fully qualified form `MASONWING@1.0.1:REQ-001` / `GLEANBIRD@1.0.1:REQ-001`; local REQ-001 không được dùng xuyên products thiếu namespace. New baseline IDs dense; update chỉ append,không renumber.

CR bắt buộc chứa requested delta,source authority,affected scope/REQ/AC/NFR/contracts/data/privacy/effects,alternatives,cost/risk,compatibility,migration,test impact,owner decision,reviewer và effective version. Proposed không đổi thành Accepted chỉ vì file được sinh. Threshold thay đổi là requirement/control change,không test maintenance đơn giản.

Phân loại:editorial không đổi semantics;compatible additive;breaking;security emergency. Emergency có thể disable để an toàn nhưng re-enable/new permission cần evidence và authority. Schema/contract meaning đổi=>major khi phá client;fresh evidence cho affected criteria. Keep history,prior failures and stale results.

Sau freeze,pack hash làm baseline reference. SHA xác nhận bytes,không tự xác nhận tác giả độc lập hoặc stakeholder consent. Hai contract shared digests phải update phối hợp:Masonwing release,consumer compatibility,Gleanbird upgrade. Không commit/push/apply external changes chỉ từ việc có file kế hoạch.


## CR-BRAND-001 (16/09/2026)

Quyết định tên đã được người dùng xác nhận; bản v1.0.1 chỉ cập nhật nhận diện và namespaced references. Xem `BRAND-MIGRATION.md` và `brand-aliases.json`. Giữ nguyên local IDs và mọi acceptance/production gate chưa hoàn tất.
