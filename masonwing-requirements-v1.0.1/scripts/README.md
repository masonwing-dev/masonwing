# Chạy lại kiểm tra tài liệu

Yêu cầu Python >=3.10. Tạo virtualenv rồi cài `python -m pip install -r scripts/requirements-validation.txt` từ nguồn package được tổ chức tin cậy. Sau đó chạy:

```sh
sh scripts/run-checks.sh
```

Lệnh mặc định không ghi lại báo cáo niêm phong. `--report 09-audit/new-check-results.json` chỉ dùng khi tạo phiên bản bundle mới, vì thay đổi file sẽ ảnh hưởng manifest. Script không gọi model/provider, không tạo database, không chạy UI, không deploy, không bật CI/tracking.

Bao gồm validator nguyên bản từ ZIP người dùng, positive/negative self-test bắt 15 lớp lỗi, kiểm REQ/AC/TC/suite/phase/work package, JSON Schema + positive/negative wire fixtures, OpenAPI local refs/path parameters, ma trận trạng thái, pairwise reference model, arithmetic oracle và source/checksum. Không thay một product integration suite bằng các kiểm tra này.

OpenAPI được kiểm tra cấu trúc và schema bằng các công cụ có sẵn; dedicated external OpenAPI validator chưa chạy vì dependency không tải được trong môi trường soạn thảo. Khi khởi tạo repository, chạy một OpenAPI 3.1 validator trong toolchain đã khóa phiên bản rồi giữ receipt. Không được ghi mục này đã PASS trước khi thực thi.
