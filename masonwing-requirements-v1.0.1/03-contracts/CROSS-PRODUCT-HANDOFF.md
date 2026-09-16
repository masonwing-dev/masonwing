# Ranh giới giao hàng giữa Masonwing và Gleanbird

Hai bộ độc lập về ownership nhưng không độc lập về runtime. Gleanbird phụ thuộc shared contracts 1.0.0 và implementation Masonwing tương ứng. Cả hai gói chứa cùng byte-level shared-contracts và cùng Masonwing Platform OpenAPI; kiểm tra SHA256 trước khi tích hợp. `cross-product-map.json` dùng ID đầy đủ `PRODUCT@VERSION:REQ-ID`, không nhầm REQ-001 của hai dự án.

## Thứ tự tích hợp

C1 walking skeleton cung cấp contracts/SDK types cục bộ, identity, tenant, artifacts và một deterministic workflow. Nhóm marketing có thể viết domain models/UI/test fixtures trên SDK ngay trong C1; external effects/model-live bị khóa cho đến khi các capability thực tế được qualification. C2 đóng gói SDK ra ngoài repository, thử nhà phát triển thứ hai và catalog public không thanh toán. Không chờ marketplace mới được viết plugin marketing.

Shared OpenAPI được sao chép trong Gleanbird để implementer có tài liệu đầy đủ; đây không phải chỉ thị triển khai lại các endpoint core. Chỉ core sở hữu handlers của `/auth`, `/session`, `/v1/.../artifacts`, `/runs` và các platform commands. Marketing đăng ký resource projections và actions qua SDK. `openapi.json` trong marketing là phần domain; `masonwing-platform.openapi.json` là hợp đồng dependency.

Mọi thay đổi breaking vào shared contract cần ADR, version mới, migration và consumer conformance. Không sửa bản copy trong marketing để tạm chữa integration. Khi source chưa có implementation, mock adapter chỉ là test double và phải gắn environment=MOCK, không phải provider qualification.
