# Chiến lược kiểm thử và acceptance

## Phân biệt artifacts

`test-cases.json`: ca kiểm thử sản phẩm được thiết kế,**NOT_RUN**. `suites.json`: grouping và execution profile. `traceability.csv`: scope->feature->REQ->AC->TC->suite. `state-vectors.json`: toàn bộ cặp trạng thái hợp lệ/trái phép trong mô hình hữu hạn. `pairwise-vectors.json`: phủ cặp của các yếu tố đã liệt kê; không phải mọi tổ hợp mọi đầu vào. `reference-vectors.json`: data mẫu có oracle. `spec-check-results.json`: kết quả kiểm tra tài liệu thực tế; tuyệt đối không là product acceptance.

## Pyramid

Unit/property cho policy wrappers,versions,cost arithmetic,normalization,metrics and state guards. Integration với PostgreSQL/Keycloak/Temporal/object store thật trong local controlled stack. Contract tests cho every operation/schema/event/provider adapter. E2E UI bằng browser thật; failure injection trước/sau từng PONR. Security tests actor/tenant/data rights matrix; load và DR theo named profiles. Live tests chỉ trong account authorized và namespace test; không gửi email/social/content public thật từ test mặc định.

Rust tooling candidates: cargo-nextest,cargo-llvm-cov,proptest,wiremock,testcontainers. Web: Vitest,Testing Library,Playwright,axe-core. API schema fuzz candidate Schemathesis;load k6;dependency cargo-deny/cargo-audit+OSV+SBOM scanner. Các tool chưa được cài/activate như CI bởi yêu cầu này; implementation team pin/test compatible versions tại bootstrap.

## Fixture rules

Clock injectable UTC; fixture aliases resolve theo `fixtures/seed.json`. Tenant A/B,Owner/Editor/Reviewer/Publisher/Viewer/Service/Support/Attacker; all credentials synthetic. Provider stub có counters accepted_requests,transmitted_requests,mutation_count,lookup_count,usage receipts. Một negative AC phải assert response **và absence** state/effect/provider counter. Crash matrix:before intent commit,after commit,before network,after acceptance,before response,before receipt commit,after receipt,before outbox ack,after restore.

## Required product run evidence

run_id,source_tree_digest,build artifact digest,test binary/version,tests collected,names/assertion IDs,profile,seed/clock,fixture hash,config/toolchain/lock digests,start/end/exit code,actual result,logs and screenshots digests,effect counters,reviewer identity. Evidence captures được redact nhưng vẫn đủ chứng minh assertion. Zero collected tests,skip,error,cancel,timeout và missing artifact không được count PASS. Không tự tạo sample PASS JSON.

## Acceptance gates

G-SPEC: structural + semantic self audit; no dangling scope. G-LOCAL: required unit/integration/contract/E2E tests thực với evidence. G-SEC: threat/tenant/secret negative suites. G-PERF: measured named profiles. G-DR: actual restore drill. G-LIVE: authorized nonempty provider/connector reports. G-REVIEW: independent semantic/evidence review. G-RELEASE: exact release partition passes all prior gates và owner deployment authorization. Source candidate thay đổi làm impact triage,không carry old PASS tự động.

## Coverage boundary

Yêu cầu 100% REQ→AC và AC→TC có thể kiểm cơ học. Không thể suy từ đó rằng 100% bugs hoặc mọi input tương lai được loại bỏ. Bổ sung boundary values,mutation testing,property fuzz,race schedules và regression sau mọi defect. Mọi production claim phải dựa evidence thực,không dựa số test cases trong ZIP.
