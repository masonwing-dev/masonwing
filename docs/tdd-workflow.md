# TDD từ đầu ra trong requirements

Toàn bộ 1.359 ca gốc được collect bởi `tests/acceptance/test_requirements.py`,
giữ nguyên preconditions, steps, expected, evidence_required, suite và REQ/AC IDs.
45 file Gherkin nằm trong `tests/acceptance/features/`. Spec gốc không bị chỉnh
để khớp với giới hạn của scaffold.

## Chọn và làm một ca

```sh
.venv/bin/python -m pytest tests/acceptance --collect-only -q
make tdd CASE='MASONWING@1.0.1:TC-AC-088'
```

Chưa có driver thì test raise `NotImplementedError` và in toàn bộ oracle của ca.
Đây là RED hữu ích cho backlog, không phải một integration test đã hoàn thiện.
Developer vẫn phải viết scenario driver nối Given/When vào application/adapter
thật và chuyển các Then của ca đó thành assertions cụ thể. Không có một parser
tự suy diễn an toàn mọi câu nghiệp vụ tự do thành assertions.

Driver nằm trong module dưới `tests/acceptance/`, đăng ký qua
`@scenario("MASONWING@1.0.1:TC-AC-...")` của `drivers.py`. Import module đăng ký
tại cuối `drivers.py` hoặc trước collection. Mỗi driver chịu trách nhiệm seed
đúng fixture, gọi code thật, assert toàn bộ expected outcomes và evidence của
chính ca. Chỉ giữ driver khi nó fail với hành vi sai trước khi implementation
được sửa. Không đăng ký function rỗng hoặc mock toàn bộ handler rồi trả PASS.

Chạy RED, implement đơn vị nhỏ nhất, chạy GREEN, sau đó refactor và chạy lại bộ
test liên quan. Khi nối adapter, thêm integration test thực; khi nối UX, thêm
browser scenario. Chỉ chuyển trạng thái product case khi bằng chứng của ca đó
đủ. Các acceptance case chưa implement không dùng skip/xfail để làm CI xanh.

## Những output tests đã có thể chạy

| Trường hợp | Output được kiểm tra | Nơi kiểm tra |
| --- | --- | --- |
| Resource khác tenant | Từ chối; tenant trong request không thắng authority | `crates/kernel/src/scope.rs` |
| Scope/expiry delegation | Không mở rộng parent/plugin/run; hết hạn bị chặn | `crates/kernel/src/grants.rs` |
| Approval đổi digest/đã hết hạn | Không đủ điều kiện thực thi | `crates/kernel/src/approval.rs` |
| Provider outcome unknown | Reconcile được yêu cầu, không resend | `crates/kernel/src/effects.rs` |
| Usage unknown | Reservation giữ nguyên | `crates/kernel/src/budget.rs` |
| Run/dependency invalid | Không dispatch sau turn limit; cycle không migrate | `crates/kernel/src/loop_guard.rs`, `registry.rs` |
| SCORE-1 | Known vector = 59; missing factor → null; rights hard block | `products/gleanbird/opportunities/` |
| Protected manual edits | Giữ block cũ; stale revision conflict không ghi đè | `products/gleanbird/evidence-content/` |
| Measurement | 12/24 = 0.5; zero eligible = NO_DATA; Wilson95 đúng vector | `products/gleanbird/measurement-optimization/` |
| Pooled tenant context | Transaction sau không đọc tenant cũ | `tests/integration/test_postgres.py` |
| Atomic commit/rollback | Business + audit + outbox cùng có hoặc cùng không | `tests/integration/test_postgres.py` |
| Provider nhận rồi ngắt kết nối | Lookup tìm receipt; mutation=1, transmit=1 | `tests/integration/test_provider_fixtures.py` |
| 16 request đồng thời cùng key | Chỉ 1 mutation; đổi fingerprint → 409 | `tests/integration/test_provider_fixtures.py` |
| Read error / unknown outcome | Không giả empty; không cung cấp blind retry | `tests/web/behavior.test.tsx` |
| Đổi tenant khi request cũ còn chạy | Cancel + remove cache; response cũ không xuất hiện | `tests/web/behavior.test.tsx` |

608 state vectors chạy qua state machines và kiểm tra version không đổi khi
transition bị cấm. Đây là adjacency tests; guard conditions có test riêng.
Không suy ra persistence, current authorization hoặc live transport từ việc
toàn bộ state vectors qua.

## Dịch vụ fault fixtures

Provider local tại `http://localhost:39859` dùng namespace `/cases/<unique-id>`.
POST `/effects` nhận `key`, `mode`, `content_digest`. Modes đang implement:
`accept`, `accept_then_disconnect`, `reject`, `rate_limit`, `unknown_cost`.
GET `/lookup/<key>` và `/counters` cung cấp observation để assert.

Các case tách namespace nên chạy lặp không reset dữ liệu của ca khác. SQLite lưu
operation markers và counters qua restart. Fake provider không truyền request ra
ngoài và luôn gắn `provider_mode=MOCK`. Các fault modes khác trong baseline cần
thêm adapter/fixture và test đúng loại trước khi claim conformance đầy đủ.

PostgreSQL, S3 và OpenBao tests dùng synthetic tenant A/B, IDs riêng cho mỗi lần
chạy. OpenBao token test bị revoke trong finally; các S3 test versions tự dọn
đúng objects của test. DB audit/inbox/outbox synthetic được giữ như append-only
evidence; không có global reset hoặc xóa volume.

## Cách đọc kết quả

`make test` kiểm tra những cơ chế scaffold đã thực sự triển khai. `make tdd`
kiểm tra product case. Schema validation chỉ xác nhận hình dạng dữ liệu; một
provider fixture test chỉ xác nhận fixture; HTTP 401 chỉ xác nhận fail-closed
entry gate. Không cộng chúng thành tỷ lệ hoàn thành product.

Production gates còn đòi real adapters, end-to-end flow evidence, live provider
qualification, performance/DR, legal/source rights và reviewer sign-off theo
release partition. `.dev/evidence/verification.json` luôn ghi loại bằng chứng
`SCAFFOLD_VERIFICATION` và `product_acceptance=NOT_ACCEPTED`.
