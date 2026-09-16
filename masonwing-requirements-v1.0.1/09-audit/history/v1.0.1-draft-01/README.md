# Bản kiểm tra trước khi chốt định dạng thời gian

Lượt kiểm tra đầu của bản đổi tên phát hiện `Exported` dùng offset `+00:00`, trong khi validator nguồn yêu cầu hậu tố `Z`. Đã đổi riêng định dạng timestamp sang UTC `Z`; không đổi requirement, validator hoặc test oracle. Kết quả hiện hành ở `../../check-results.json`.
