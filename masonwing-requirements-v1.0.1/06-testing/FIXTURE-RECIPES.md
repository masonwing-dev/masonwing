# Fixture setup và oracle theo criterion

Các recipe là kế hoạch test cụ thể,không phải seed sản phẩm đã được chạy. Labels A/B là alias trong fixtures/seed.json.

## F-001 — Lắp ghép sản phẩm, không fork kernel

Owner: QA + implementer `composition`. Shared fixtures: tenant_a/tenant_b,clock,provider counters.

### AC-001 / TC-AC-001

Setup: product A bật document-review; marketing không được cài

Trigger duy nhất: khởi động product A

Oracle: route document-review tồn tại; route marketing trả 404; không tạo bảng marketing

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-002 / TC-AC-002

Setup: kernel digest K và hai fixture document-review, marketing

Trigger duy nhất: build hai sản phẩm dùng K

Oracle: kernel digest giống nhau; cả hai fixture hoàn thành workflow được khai báo

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-003 / TC-AC-003

Setup: fixture checksum chỉ có input artifact và deterministic handler

Trigger duy nhất: thực thi workflow

Oracle: output hash đúng fixture; provider calls=0; không yêu cầu model key

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-004 / TC-AC-004

Setup: manifest khai báo capability không có trong registry

Trigger duy nhất: activate sản phẩm

Oracle: CONFIG_UNSUPPORTED; readiness=false; tenant routes chưa được phục vụ

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-005 / TC-AC-005

Setup: sample app bên ngoài monorepo dùng package phát hành

Trigger duy nhất: build và chạy conformance smoke

Oracle: không có source fork; compatible contract hoạt động; digest package có trong lockfile

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

## F-002 — Danh tính plugin và dependency resolution

Owner: QA + implementer `registry`. Shared fixtures: tenant_a/tenant_b,clock,provider counters.

### AC-006 / TC-AC-006

Setup: manifest fixture hợp lệ và schema 1.0.0

Trigger duy nhất: verify manifest

Oracle: kết quả VALID gắn schema version và artifact digest

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-007 / TC-AC-007

Setup: plugin A cần B và B cần A

Trigger duy nhất: resolve graph

Oracle: DEPENDENCY_CYCLE kèm đường chu trình; migrations=0; installed registry không đổi

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-008 / TC-AC-008

Setup: plugin A yêu cầu contract x@2; chỉ có x@1

Trigger duy nhất: resolve A

Oracle: DEPENDENCY_UNSATISFIED; không tải runtime code

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-009 / TC-AC-009

Setup: artifact bytes đã sửa sau ký

Trigger duy nhất: install plugin

Oracle: ARTIFACT_INTEGRITY; handler calls=0; artifact QUARANTINED

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-010 / TC-AC-010

Setup: registry có hai phiên bản tương thích

Trigger duy nhất: resolve rồi resolve với lockfile đã sinh

Oracle: lần hai chọn cùng digests; không tự lấy bản mới

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-011 / TC-AC-011

Setup: module A truy cập điểm riêng của B

Trigger duy nhất: register contribution

Oracle: EXTENSION_SCOPE_DENIED; B không bị thay handler

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

## F-003 — Vòng đời, nâng cấp và thu hồi plugin

Owner: QA + implementer `lifecycle`. Shared fixtures: tenant_a/tenant_b,clock,provider counters.

### AC-012 / TC-AC-012

Setup: manifest đã verify; migration additive thành công

Trigger duy nhất: commit install

Oracle: state INSTALLED_DISABLED; handler chưa nhận việc

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-013 / TC-AC-013

Setup: plugin xin read và publish; admin chỉ cấp read

Trigger duy nhất: enable rồi gọi publish

Oracle: read hoạt động; publish CAPABILITY_DENIED; provider mutation=0

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-014 / TC-AC-014

Setup: v1 đang active; tạo run R; sau đó cài v2

Trigger duy nhất: resume R

Oracle: R dùng digest v1; run mới dùng v2 theo rollout policy

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-015 / TC-AC-015

Setup: v1 có một run đang chạy

Trigger duy nhất: disable v1

Oracle: new run bị PLUGIN_DRAINING; run cũ tiếp tục theo cancellation policy

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-016 / TC-AC-016

Setup: run v1 đã pin nhưng signer hoặc digest bị revoke

Trigger duy nhất: gọi host action kế tiếp

Oracle: PLUGIN_REVOKED; external calls=0 sau điểm enforcement; run chuyển BLOCKED

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-017 / TC-AC-017

Setup: run hoàn tất còn liên kết evidence thuộc plugin

Trigger duy nhất: uninstall với chế độ retain

Oracle: UI plugin biến mất; audit và evidence vẫn truy xuất bởi auditor được phép

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-018 / TC-AC-018

Setup: migration ghi dữ liệu không tương thích v1

Trigger duy nhất: operator yêu cầu rollback v1

Oracle: ROLLBACK_UNSAFE; không deploy v1; cung cấp forward-repair runbook

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

## F-004 — Cô lập Wasm, native và remote worker

Owner: QA + implementer `sandbox`. Shared fixtures: tenant_a/tenant_b,clock,provider counters.

### AC-019 / TC-AC-019

Setup: tenant uploader gửi native artifact

Trigger duy nhất: install

Oracle: TRUST_CLASS_DENIED; process library chưa nạp

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-020 / TC-AC-020

Setup: Wasm plugin chỉ có artifact.read cho artifact A

Trigger duy nhất: plugin đọc artifact B

Oracle: RESOURCE_DENIED; nội dung B không có trong result hay logs

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-021 / TC-AC-021

Setup: Wasm fixture vòng lặp vô hạn; profile L-WASM

Trigger duy nhất: invoke

Oracle: SANDBOX_LIMIT; host health vẫn OK; invocation kết thúc trong deadline profile

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-022 / TC-AC-022

Setup: mock host HTTP không trả body

Trigger duy nhất: plugin chờ HTTP

Oracle: HOST_IO_TIMEOUT trong giới hạn; không giữ worker lease vô hạn

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-023 / TC-AC-023

Setup: URL đổi DNS sang loopback hoặc metadata address

Trigger duy nhất: request qua egress broker

Oracle: EGRESS_DENIED; TCP tới địa chỉ cấm=0

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-024 / TC-AC-024

Setup: redirect HTTPS từ public sang private subnet

Trigger duy nhất: follow redirect qua broker

Oracle: EGRESS_DENIED; credentials không chuyển sang origin khác

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-025 / TC-AC-025

Setup: worker W1 lease=1 hết hạn; W2 có lease=2

Trigger duy nhất: W1 gửi completion

Oracle: STALE_FENCE; output W2 không bị ghi đè

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-026 / TC-AC-026

Setup: invocation tạo temp artifact và handle H

Trigger duy nhất: invocation kết thúc

Oracle: H không dùng lại được; temp không còn sau cleanup deadline; retained artifact không bị xóa

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

## F-005 — Đăng nhập OIDC qua Rust BFF

Owner: QA + implementer `oidc`. Shared fixtures: tenant_a/tenant_b,clock,provider counters.

### AC-027 / TC-AC-027

Setup: browser chưa có session

Trigger duy nhất: GET /auth/login với return_to nội bộ

Oracle: redirect tới issuer đã allowlist; state, nonce, PKCE transaction dùng một lần

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-028 / TC-AC-028

Setup: mock issuer trả token đúng issuer, audience, nonce; code chưa dùng

Trigger duy nhất: GET /auth/callback

Oracle: session mới gắn issuer+sub; cookie HttpOnly Secure; tokens không xuất hiện trong response body

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-029 / TC-AC-029

Setup: state mismatch, nonce mismatch, issuer mismatch hoặc code replay

Trigger duy nhất: gửi callback cho từng fixture

Oracle: 401 AUTH_INVALID_CALLBACK; session count không tăng; không lộ token

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-030 / TC-AC-030

Setup: return_to=https://attacker.example hoặc //attacker.example

Trigger duy nhất: start login

Oracle: 400 RETURN_TARGET_DENIED; không redirect ra origin lạ

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-031 / TC-AC-031

Setup: request username/password tới BFF

Trigger duy nhất: gửi password-grant request

Oracle: AUTH_FLOW_UNSUPPORTED; Keycloak grant calls=0; mật khẩu không ghi log

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-032 / TC-AC-032

Setup: discovery có issuer khác hoặc endpoint private ngoài allowlist

Trigger duy nhất: activate identity adapter

Oracle: ISSUER_TRUST_FAILED; không tải JWKS qua endpoint lạ

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-033 / TC-AC-033

Setup: IdP A/sub1 và B/sub2 cùng email

Trigger duy nhất: đăng nhập lần lượt

Oracle: hai identity records; membership không chuyển qua email match

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-034 / TC-AC-034

Setup: user ở login/account settings

Trigger duy nhất: open recovery hoặc MFA enrollment

Oracle: đến verified IdP flow; app không thu password reset token hoặc tự đổi credential database

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

## F-006 — Session, membership và tổ chức

Owner: QA + implementer `sessions`. Shared fixtures: tenant_a/tenant_b,clock,provider counters.

### AC-035 / TC-AC-035

Setup: session guest có ID S

Trigger duy nhất: login hoặc hoàn tất step-up

Oracle: ID mới khác S; S không dùng được để gọi API

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-036 / TC-AC-036

Setup: attacker origin hoặc token thiếu

Trigger duy nhất: POST request ghi dữ liệu

Oracle: 403 CSRF_REJECTED; state version không đổi

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-037 / TC-AC-037

Setup: virtual clock đúng expiry hoặc vượt expiry

Trigger duy nhất: GET /session

Oracle: 401 SESSION_EXPIRED; không gia hạn phiên đã hết hạn

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-038 / TC-AC-038

Setup: session S đang active

Trigger duy nhất: POST /auth/logout

Oracle: S bị revoke; API dùng S trả 401

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-039 / TC-AC-039

Setup: logout token có sid hợp lệ đúng issuer/audience

Trigger duy nhất: POST /auth/backchannel-logout

Oracle: các phiên matching bị revoke; unrelated sessions còn active

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-040 / TC-AC-040

Setup: logout token sai chữ ký hoặc replay jti

Trigger duy nhất: gửi logout event

Oracle: sai chữ ký bị từ chối; replay idempotent; không revoke unrelated subject

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-041 / TC-AC-041

Setup: invite đúng tenant/email đã verify; chưa hết hạn

Trigger duy nhất: hai request accept đồng thời

Oracle: một membership; response replay trả cùng membership; không thêm role khác

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-042 / TC-AC-042

Setup: tenant có đúng một owner

Trigger duy nhất: remove hoặc demote owner

Oracle: 409 LAST_OWNER; membership không đổi

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-043 / TC-AC-043

Setup: browser đang mở tenant A với SSE và cache

Trigger duy nhất: admin revoke membership

Oracle: SSE tenant A đóng theo L-REVOKE; cache A bị xóa; deep link trả 404

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

## F-007 — Cedar, tenant isolation và quyền theo tài nguyên

Owner: QA + implementer `authorization`. Shared fixtures: tenant_a/tenant_b,clock,provider counters.

### AC-044 / TC-AC-044

Setup: user có membership nhưng action chưa được grant

Trigger duy nhất: gọi action

Oracle: 403 AUTHZ_DENIED; mutation=0

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-045 / TC-AC-045

Setup: policy permit publish và forbid frozen-tenant

Trigger duy nhất: publish trong frozen tenant

Oracle: 403 AUTHZ_DENIED; provider calls=0

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-046 / TC-AC-046

Setup: permit khớp; forbid thiếu attribute gây diagnostics

Trigger duy nhất: gọi action

Oracle: AUTHZ_EVALUATION_ERROR; không chạy effect; audit ghi diagnostic code không chứa secrets

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-047 / TC-AC-047

Setup: user tenant A; ID thật của B và ID không tồn tại

Trigger duy nhất: GET từng ID

Oracle: cùng 404 RESOURCE_NOT_FOUND shape; không lộ tên, existence, signed URL hay result count

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-048 / TC-AC-048

Setup: approval còn hạn nhưng publisher bị revoke trước dispatch

Trigger duy nhất: worker dispatch

Oracle: 403 AUTHZ_DENIED; mutation transmit=0

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-049 / TC-AC-049

Setup: body tenant_id=B nhưng route/session thuộc A

Trigger duy nhất: submit command

Oracle: TENANT_CONTEXT_MISMATCH; không chọn B theo body

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-050 / TC-AC-050

Setup: owner có session thường; action secret.rotate

Trigger duy nhất: request action

Oracle: STEP_UP_REQUIRED; sau verified MFA dưới L-STEPUP thì được đánh giá quyền tiếp

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

## F-008 — Machine identity và ủy quyền agent

Owner: QA + implementer `delegation`. Shared fixtures: tenant_a/tenant_b,clock,provider counters.

### AC-051 / TC-AC-051

Setup: token staging dùng trên API production

Trigger duy nhất: worker gửi command

Oracle: 401 TOKEN_AUDIENCE_INVALID; command chưa enqueue

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-052 / TC-AC-052

Setup: run chỉ được draft article A; plugin có publish rộng hơn

Trigger duy nhất: tool publish A

Oracle: DELEGATION_DENIED; external calls=0

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-053 / TC-AC-053

Setup: creator đã logout; automation grant còn active

Trigger duy nhất: scheduler tick

Oracle: run gắn grant ID và tenant; không sử dụng cookie creator

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-054 / TC-AC-054

Setup: clock=expires_at; queued action chưa transmit

Trigger duy nhất: dispatch

Oracle: DELEGATION_EXPIRED; transmit=0

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-055 / TC-AC-055

Setup: tool arguments chứa role=owner và tenant_id khác

Trigger duy nhất: execute tool proposal

Oracle: CAPABILITY_ESCALATION_DENIED; granted set không đổi

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-056 / TC-AC-056

Setup: parent chỉ read A; child xin read A,B

Trigger duy nhất: spawn child

Oracle: child grant không có B; request vượt scope bị từ chối

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

## F-009 — Quyền sở hữu dữ liệu và atomic audit

Owner: QA + implementer `data`. Shared fixtures: tenant_a/tenant_b,clock,provider counters.

### AC-057 / TC-AC-057

Setup: plugin A không có query contract B

Trigger duy nhất: A đọc record nội bộ B

Oracle: DATA_SCOPE_DENIED; không trả raw SQL handle

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-058 / TC-AC-058

Setup: inject audit write failure trong mutation

Trigger duy nhất: submit update

Oracle: transaction rollback; không tồn tại business update thiếu audit

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-059 / TC-AC-059

Setup: resource version 2; request If-Match=1

Trigger duy nhất: update

Oracle: 409 STALE_VERSION; resource giữ v2

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-060 / TC-AC-060

Setup: hai writer cùng base version=2

Trigger duy nhất: gửi hai update đồng thời

Oracle: một update thành công v3; bên thua 409; không merge ngầm

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-061 / TC-AC-061

Setup: RLS fixture A/B; runtime DB role không owner/BYPASSRLS

Trigger duy nhất: chạy negative isolation suite

Oracle: không đọc hoặc ghi record B từ tenant A kể cả connection reuse

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-062 / TC-AC-062

Setup: process bị kill ngay sau commit mutation

Trigger duy nhất: restart dispatcher

Oracle: business event được giao; không mất acknowledged intent

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-063 / TC-AC-063

Setup: unknown field, payload quá lớn hoặc số ngoài range

Trigger duy nhất: gửi command cho từng vector

Oracle: 400 hoặc 413 theo error catalog; mutation=0

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

## F-010 — Artifact, nguồn gốc và vòng đời dữ liệu

Owner: QA + implementer `artifacts`. Shared fixtures: tenant_a/tenant_b,clock,provider counters.

### AC-064 / TC-AC-064

Setup: upload bytes B rồi finalize

Trigger duy nhất: đọc và thử ghi lại cùng artifact ID

Oracle: digest=SHA256(B); overwrite trả IMMUTABLE_ARTIFACT

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-065 / TC-AC-065

Setup: user được đọc A, không B

Trigger duy nhất: request download A

Oracle: locator chỉ đọc A; expires theo L-SIGNEDURL; không liệt kê bucket

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-066 / TC-AC-066

Setup: polyglot HTML/image hoặc malware fixture

Trigger duy nhất: finalize upload

Oracle: QUARANTINED; preview, model retrieval, public link=0

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-067 / TC-AC-067

Setup: source S được dùng bởi derived D

Trigger duy nhất: revoke S

Oracle: D marked INVALIDATED; retrieval không trả D; active workflows chuyển BLOCKED

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-068 / TC-AC-068

Setup: subject data có cache, index, export, external publication

Trigger duy nhất: execute deletion

Oracle: active cache/index bị xóa; external references ghi pending action; không báo xóa toàn bộ khi chưa chứng minh

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-069 / TC-AC-069

Setup: retention hết hạn nhưng hold active

Trigger duy nhất: retention job

Oracle: HELD; dữ liệu không bị xóa; audit thể hiện hold owner

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-070 / TC-AC-070

Setup: backup trước yêu cầu xóa S; tombstone sau đó

Trigger duy nhất: restore

Oracle: S không đọc được trước readiness; restore report ghi tombstone replay

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-071 / TC-AC-071

Setup: provider conversation/token blob có synthetic secret; database backup được tạo

Trigger duy nhất: inspect backup without secret-manager key

Oracle: không giải mã được secret; key material không có trong app database dump

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

## F-011 — Durable workflow, checkpoint và bounded agent loop

Owner: QA + implementer `workflow`. Shared fixtures: tenant_a/tenant_b,clock,provider counters.

### AC-072 / TC-AC-072

Setup: idempotency key K và fingerprint P chưa tồn tại

Trigger duy nhất: POST /runs hai lần với K,P

Oracle: cùng run ID; một root execution

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-073 / TC-AC-073

Setup: K đã dùng với fingerprint P1

Trigger duy nhất: POST /runs với K,P2

Oracle: 409 IDEMPOTENCY_CONFLICT; không tạo run mới

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-074 / TC-AC-074

Setup: worker chết sau completed checkpoint và trước bước kế

Trigger duy nhất: restart worker

Oracle: bước hoàn tất không bị ghi thành pending; bước kế được điều phối một lần theo logical ID

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-075 / TC-AC-075

Setup: 100 run WAITING_APPROVAL

Trigger duy nhất: đọc capacity và approve một run

Oracle: 100 durable waits; worker slot không bị chiếm bởi idle waits; đúng run được resume

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-076 / TC-AC-076

Setup: profile max_turns=8; model liên tục xin thêm tool

Trigger duy nhất: thực thi lượt vượt giới hạn

Oracle: LIMIT_REACHED; model/tool calls sau limit=0

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-077 / TC-AC-077

Setup: run SUCCEEDED

Trigger duy nhất: submit resume

Oracle: 409 ILLEGAL_TRANSITION; state SUCCEEDED không đổi

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-078 / TC-AC-078

Setup: remote request đã qua point of no return

Trigger duy nhất: cancel run

Oracle: CANCEL_REQUESTED; không báo effect undone; reconciliation vẫn được phép

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-079 / TC-AC-079

Setup: v2 replay history v1 không tương thích

Trigger duy nhất: resume

Oracle: REPLAY_INCOMPATIBLE; transmit=0; operator có history reference

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-080 / TC-AC-080

Setup: workflow gọi handler không tồn tại hoặc cycle không có loop bound

Trigger duy nhất: install workflow definition

Oracle: WORKFLOW_INVALID; created runs=0; no model/provider calls

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-081 / TC-AC-081

Setup: node A trả object thiếu field cho node B

Trigger duy nhất: complete A

Oracle: NODE_OUTPUT_INVALID; B chưa dispatch; invalid result retained as quarantined diagnostic

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

## F-012 — Phê duyệt bất biến và race resolution

Owner: QA + implementer `approval`. Shared fixtures: tenant_a/tenant_b,clock,provider counters.

### AC-082 / TC-AC-082

Setup: proposal P hash H target A v2

Trigger duy nhất: approve P

Oracle: approval record chứa H,A,v2 và actor; không cấp blanket permission

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-083 / TC-AC-083

Setup: approved H1; draft hiện tại H2

Trigger duy nhất: dispatch

Oracle: APPROVAL_STALE; transmit=0

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-084 / TC-AC-084

Setup: approval expires_at=T; authoritative time=T

Trigger duy nhất: dispatch

Oracle: APPROVAL_EXPIRED; transmit=0

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-085 / TC-AC-085

Setup: approval PENDING version1

Trigger duy nhất: approve và cancel đồng thời

Oracle: một terminal decision; loser 409 DECISION_CONFLICT; audit ghi winner

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-086 / TC-AC-086

Setup: policy requires distinct approver; author A

Trigger duy nhất: A approve

Oracle: 403 FOUR_EYES_REQUIRED; proposal còn PENDING

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-087 / TC-AC-087

Setup: proposal có before/after và target domain

Trigger duy nhất: mở approval UI

Oracle: UI hiển thị cùng digests trong API; không dùng latest draft để thay preview

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

## F-013 — Effect broker và đối soát kết quả chưa biết

Owner: QA + implementer `effects`. Shared fixtures: tenant_a/tenant_b,clock,provider counters.

### AC-088 / TC-AC-088

Setup: broker có authorized effect

Trigger duy nhất: dispatch với fault trước network

Oracle: effect intent tồn tại trước bất kỳ provider acceptance nào

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-089 / TC-AC-089

Setup: provider nhận request rồi cắt TCP trước response

Trigger duy nhất: dispatch

Oracle: OUTCOME_UNKNOWN; resend count=0; reconciliation queued

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-090 / TC-AC-090

Setup: remote operation ID đã tồn tại sau timeout

Trigger duy nhất: reconcile

Oracle: SUCCEEDED cùng remote ID/version/hash; không suy ra thành công từ HTTP retry count

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-091 / TC-AC-091

Setup: provider không có key hoặc lookup đáng tin

Trigger duy nhất: reconcile

Oracle: MANUAL_REVIEW; không gửi lại; UI nói kết quả chưa xác định

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-092 / TC-AC-092

Setup: K đã gắn content H1

Trigger duy nhất: submit K,H2

Oracle: 409 IDEMPOTENCY_CONFLICT; intent gốc không đổi

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-093 / TC-AC-093

Setup: effect đã authorized nhưng chưa gửi; switch enabled

Trigger duy nhất: dispatch

Oracle: KILL_SWITCH_ACTIVE; transmit=0

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-094 / TC-AC-094

Setup: connect failure before bytes sent; budget còn

Trigger duy nhất: schedule retry

Oracle: attempt dùng cùng effect identity; backoff hợp lệ; vượt budget vào DEAD_LETTER

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-095 / TC-AC-095

Setup: bài đã publish và user yêu cầu revert

Trigger duy nhất: submit compensation

Oracle: effect mới có approval riêng; original receipt giữ nguyên; UI không nói lịch sử đã bị xóa

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

## F-014 — Ngân sách, quota và fairness

Owner: QA + implementer `budget`. Shared fixtures: tenant_a/tenant_b,clock,provider counters.

### AC-096 / TC-AC-096

Setup: budget còn 100 units; request upper_bound=20

Trigger duy nhất: dispatch

Oracle: reservation 20 tồn tại trước provider call; balance khả dụng 80

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-097 / TC-AC-097

Setup: balance=100; hai request cùng đòi 80

Trigger duy nhất: reserve đồng thời

Oracle: một reservation 80; một BUDGET_EXCEEDED; tổng reserved<=100

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-098 / TC-AC-098

Setup: provider timeout sau acceptance

Trigger duy nhất: settle attempt

Oracle: cost_state UNKNOWN; không giải phóng reservation như zero; reconcile queued

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-099 / TC-AC-099

Setup: usage 12 cho reservation20

Trigger duy nhất: gửi receipt usage hai lần

Oracle: charged12; released8; lần replay không trừ thêm

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-100 / TC-AC-100

Setup: model profile không có output ceiling hoặc tool price cap

Trigger duy nhất: admit

Oracle: PRICE_BOUND_UNKNOWN; billable calls=0

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-101 / TC-AC-101

Setup: tenant A đầy slots; B còn share

Trigger duy nhất: submit A và B

Oracle: A queued hoặc 429 theo backlog; B vẫn được dispatch

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-102 / TC-AC-102

Setup: BYOK profile; platform daily limit hết

Trigger duy nhất: dispatch

Oracle: QUOTA_EXCEEDED; customer provider calls=0

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

## F-015 — Provider adapter, OpenAI Responses và compaction

Owner: QA + implementer `models`. Shared fixtures: tenant_a/tenant_b,clock,provider counters.

### AC-103 / TC-AC-103

Setup: task cần Responses compact; adapter chỉ Chat Completions

Trigger duy nhất: plan task

Oracle: CAPABILITY_UNSUPPORTED; không downgrade ngầm sang plain text

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-104 / TC-AC-104

Setup: fixture chứa message.phase, reasoning, tool call, compaction và unknown opaque item

Trigger duy nhất: persist rồi reload

Oracle: canonical native payload giữ nguyên các fields/items; không leak native content vào public logs

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-105 / TC-AC-105

Setup: compact response có 3 ordered items gồm opaque compaction

Trigger duy nhất: continue conversation

Oracle: next input chứa đủ 3 items đúng thứ tự; không chỉ dùng compact item

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-106 / TC-AC-106

Setup: profile chọn manual history với server compaction

Trigger duy nhất: gửi hai lượt liên tiếp

Oracle: không trộn previous_response_id với duplicate manual history; native compaction output còn được giữ

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-107 / TC-AC-107

Setup: xóa provider session sau khi run đã có approval/evidence

Trigger duy nhất: đọc business records

Oracle: approval/evidence/receipt vẫn tồn tại; không reconstruct từ model summary

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-108 / TC-AC-108

Setup: opaque state provider A; chuyển B

Trigger duy nhất: continue

Oracle: B không nhận opaque blob A; context có source refs hợp lệ và thông báo continuity boundary

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-109 / TC-AC-109

Setup: tool arguments malformed hoặc output thiếu required field

Trigger duy nhất: consume output

Oracle: MODEL_OUTPUT_INVALID; tool calls=0; repair attempts giới hạn

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-110 / TC-AC-110

Setup: SSE bị cắt giữa JSON tool call

Trigger duy nhất: process stream

Oracle: INCOMPLETE; tool dispatch=0; cost UNKNOWN hoặc usage thực có bằng chứng

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-111 / TC-AC-111

Setup: restricted source; provider chưa có approved retention/region policy

Trigger duy nhất: invoke

Oracle: DATA_POLICY_DENIED; request bytes sent=0

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

## F-016 — OAuth connector, secret broker và MCP

Owner: QA + implementer `connectors`. Shared fixtures: tenant_a/tenant_b,clock,provider counters.

### AC-112 / TC-AC-112

Setup: OAuth state gắn tenant A, provider account X

Trigger duy nhất: callback connector

Oracle: connection thuộc A/X; không tự tạo membership hay login session

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-113 / TC-AC-113

Setup: writer gọi tool với connection C

Trigger duy nhất: inspect tool context và output

Oracle: chỉ handle C; refresh token không có trong prompt, logs hoặc browser payload

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-114 / TC-AC-114

Setup: rotating refresh token T1; hai workers

Trigger duy nhất: refresh đồng thời

Oracle: một accepted refresh; newest token không bị T1 ghi đè; invalid_grant chuyển NEEDS_REAUTH

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-115 / TC-AC-115

Setup: connection read-only; action publish

Trigger duy nhất: execute

Oracle: CONNECTION_SCOPE_DENIED; provider mutation=0

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-116 / TC-AC-116

Setup: tool description yêu cầu bỏ guard và lấy secrets

Trigger duy nhất: register và invoke

Oracle: granted policy không đổi; không inject description vào system authority

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-117 / TC-AC-117

Setup: MCP server muốn BFF token

Trigger duy nhất: connect tool

Oracle: TOKEN_PASSTHROUGH_DENIED; external receives=0

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-118 / TC-AC-118

Setup: provider thay required field hoặc meaning contract version

Trigger duy nhất: ingest response

Oracle: CONTRACT_DRIFT; không cập nhật business truth bằng dữ liệu sai

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

## F-017 — Scheduler, event bus và replay

Owner: QA + implementer `events`. Shared fixtures: tenant_a/tenant_b,clock,provider counters.

### AC-119 / TC-AC-119

Setup: schedule timezone America/New_York qua DST; profile skip-gap/run-once-fold

Trigger duy nhất: advance clock qua các boundary

Oracle: số occurrence đúng schedule policy; không có run kép do clock fold

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-120 / TC-AC-120

Setup: schedule coalesce_latest; run trước chưa xong

Trigger duy nhất: ba tick tiếp theo

Oracle: chỉ một occurrence pending mới nhất; missed_count=3 được audit

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-121 / TC-AC-121

Setup: same event_id gửi hai lần

Trigger duy nhất: consume

Oracle: inbox có một receipt; logical business mutation một lần

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-122 / TC-AC-122

Setup: subscriber trả 503 hết retries

Trigger duy nhất: process budget cuối

Oracle: DEAD_LETTER cùng reason/attempt history; operator có replay action được auth

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-123 / TC-AC-123

Setup: client trước có quyền A, nay bị revoke; Last-Event-ID cũ

Trigger duy nhất: connect SSE

Oracle: không replay tenant A events; AUTHZ_DENIED hoặc stream close

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-124 / TC-AC-124

Setup: cursor trước retention window

Trigger duy nhất: reconnect

Oracle: 410 SNAPSHOT_REQUIRED; client refetch; không giả stream đầy đủ

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

## F-018 — React shell và UI plugin

Owner: QA + implementer `ui`. Shared fixtures: tenant_a/tenant_b,clock,provider counters.

### AC-125 / TC-AC-125

Setup: hai domain plugins dùng UI SDK

Trigger duy nhất: đi qua routes và mở dialog

Oracle: một h1/route; shared components; không thêm global header thứ hai

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-126 / TC-AC-126

Setup: hai plugin khai báo cùng route /settings

Trigger duy nhất: load contributions

Oracle: UI_CONTRIBUTION_CONFLICT; không overwrite route im lặng

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-127 / TC-AC-127

Setup: iframe origin khác; fixture cố đọc host DOM và gửi forged message

Trigger duy nhất: open contribution

Oracle: browser chặn DOM; bridge từ chối origin/schema; privileged calls=0

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-128 / TC-AC-128

Setup: Editor được downgrade Viewer khi đang mở editor

Trigger duy nhất: nhận permission refresh

Oracle: Publish/Edit bị disable hoặc ẩn; cached data tenant khác không hiện

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-129 / TC-AC-129

Setup: mỗi state fixture trong UI matrix

Trigger duy nhất: render route

Oracle: title/recovery control đúng state; không hiển thị giả dữ liệu khi error

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-130 / TC-AC-130

Setup: chưa login; deep link resource A

Trigger duy nhất: login thành công

Oracle: đến A sau auth; nếu không có quyền trả 404 thay vì landing giả

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-131 / TC-AC-131

Setup: editor dirty; autosave failed

Trigger duy nhất: navigate away

Oracle: stay/discard dialog; cancel giữ draft; không tự publish hoặc ghi đè server

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

## F-019 — Tìm kiếm, thông báo và export có quyền

Owner: QA + implementer `search-export`. Shared fixtures: tenant_a/tenant_b,clock,provider counters.

### AC-132 / TC-AC-132

Setup: index chứa A/B cùng keyword

Trigger duy nhất: A search

Oracle: B không có trong hits, facets, snippets hay counts

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-133 / TC-AC-133

Setup: nhiều records cùng timestamp; cursor page1

Trigger duy nhất: request page2

Oracle: không lặp/bỏ record trong snapshot; cursor đổi filters trả CURSOR_MISMATCH

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-134 / TC-AC-134

Setup: auditor export 100 records

Trigger duy nhất: start rồi cập nhật source

Oracle: export thể hiện snapshot đã pin; manifest có schema/digest và data classification

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-135 / TC-AC-135

Setup: cell bắt đầu =,+,-,@,tab,CR

Trigger duy nhất: export CSV

Oracle: mở fixture không thực thi formula; raw value có trong safe JSON export được auth

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-136 / TC-AC-136

Setup: event có secret canary và restricted draft

Trigger duy nhất: send notification

Oracle: chỉ summary được phép và deep link; canary=0 trong payload

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-137 / TC-AC-137

Setup: notification liên kết approval pending

Trigger duy nhất: mark read

Oracle: approval vẫn PENDING; publish calls=0

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

## F-020 — Quan sát, audit và support access

Owner: QA + implementer `audit`. Shared fixtures: tenant_a/tenant_b,clock,provider counters.

### AC-138 / TC-AC-138

Setup: run R có trace T

Trigger duy nhất: invoke remote step

Oracle: trace chứa parent relation R/T và plugin digest; không dùng tenant text làm metric label

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-139 / TC-AC-139

Setup: runtime token; event E tồn tại

Trigger duy nhất: update/delete E

Oracle: AUDIT_IMMUTABLE; original digest không đổi

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-140 / TC-AC-140

Setup: canary ở token, error body, header, tool args

Trigger duy nhất: inject fault qua các sinks

Oracle: canary không xuất hiện trong logs, traces, metrics, exports, screenshots

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-141 / TC-AC-141

Setup: tenant owner cấp 15 phút chỉ đọc metadata

Trigger duy nhất: support đọc content ngoài scope

Oracle: SUPPORT_SCOPE_DENIED; metadata hợp lệ có audit; expiry chặn toàn access

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-142 / TC-AC-142

Setup: audit datastore lỗi write

Trigger duy nhất: publish request

Oracle: AUDIT_UNAVAILABLE; transmit=0; không success toast

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-143 / TC-AC-143

Setup: DLQ vượt L-ALERT threshold

Trigger duy nhất: evaluate alert

Oracle: alert có correlation, owner Ops, runbook ref và side-effect state

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

## F-021 — Backup, deploy, migration và supply chain

Owner: QA + implementer `operations`. Shared fixtures: tenant_a/tenant_b,clock,provider counters.

### AC-144 / TC-AC-144

Setup: CI build candidate

Trigger duy nhất: publish candidate to staging registry

Oracle: SBOM và provenance tham chiếu cùng digest; không dùng mutable latest trong deployment manifest

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-145 / TC-AC-145

Setup: staging service token và secret path

Trigger duy nhất: request production data

Oracle: ENVIRONMENT_DENIED; data returned=0

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-146 / TC-AC-146

Setup: destructive migration thiếu backup receipt

Trigger duy nhất: migrate

Oracle: MIGRATION_PREFLIGHT_FAILED; schema không đổi

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-147 / TC-AC-147

Setup: backup Postgres/object store/history khác watermark

Trigger duy nhất: restore

Oracle: write readiness=false đến khi repair/reconcile hoàn tất; dangling refs được report

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-148 / TC-AC-148

Setup: policy hoặc secret broker unavailable

Trigger duy nhất: GET /health/ready

Oracle: non-200 write readiness; cached public static page vẫn có thể phục vụ

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-149 / TC-AC-149

Setup: SBOM có license bị cấm hoặc unknown

Trigger duy nhất: release gate

Oracle: LICENSE_REVIEW_REQUIRED; không promote; không tự khẳng định OSS

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-150 / TC-AC-150

Setup: backup có PREPARED nhưng provider đã xử lý sau backup

Trigger duy nhất: resume restore

Oracle: OUTCOME_UNKNOWN/RECONCILING; không resend từ trạng thái cũ

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

## F-022 — Catalog plugin, curation và marketplace

Owner: QA + implementer `marketplace`. Shared fixtures: tenant_a/tenant_b,clock,provider counters.

### AC-151 / TC-AC-151

Setup: publisher đã verify; artifact digest D

Trigger duy nhất: submit listing

Oracle: listing PENDING_REVIEW chứa D, capabilities, license, compatibility và support contact

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-152 / TC-AC-152

Setup: listing chưa đủ reviews

Trigger duy nhất: request public install

Oracle: LISTING_NOT_APPROVED; artifact không được activated

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-153 / TC-AC-153

Setup: v1 read; v2 thêm publish

Trigger duy nhất: update theo auto-update policy

Oracle: UPDATE_APPROVAL_REQUIRED; v1 còn hoạt động; publish không tự được cấp

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-154 / TC-AC-154

Setup: digest D đang listed và revoked

Trigger duy nhất: install D từ cache hoặc catalog

Oracle: ARTIFACT_REVOKED; không bypass bằng offline cache

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-155 / TC-AC-155

Setup: tenant có license plugin nhưng user không có publish role

Trigger duy nhất: publish

Oracle: AUTHZ_DENIED; paid entitlement không cấp quyền nghiệp vụ

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-156 / TC-AC-156

Setup: catalog mở; commerce capability disabled

Trigger duy nhất: gọi checkout endpoint

Oracle: COMMERCE_DISABLED; charge/payout=0; free install vẫn theo policy

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

## F-023 — SDK, conformance và developer experience

Owner: QA + implementer `sdk`. Shared fixtures: tenant_a/tenant_b,clock,provider counters.

### AC-157 / TC-AC-157

Setup: clean workspace; plugin namespace com.example.review

Trigger duy nhất: CLI init

Oracle: generated project validate thành công; không chứa marketing import

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-158 / TC-AC-158

Setup: host major1; plugin requires major2

Trigger duy nhất: load

Oracle: CONTRACT_INCOMPATIBLE có supported range; handler calls=0

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-159 / TC-AC-159

Setup: hai clean builds cùng lock/source/config

Trigger duy nhất: pack hai lần

Oracle: unsigned content digests giống nhau; timestamp/signature envelope không lẫn payload reproducibility

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-160 / TC-AC-160

Setup: artifact D, suite version S

Trigger duy nhất: run conformance

Oracle: report D/S/env; NOT_RUN/skipped không được tính PASS

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-161 / TC-AC-161

Setup: auth mock hoặc unsigned plugin=true; env production

Trigger duy nhất: start

Oracle: UNSAFE_CONFIGURATION; readiness=false

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.

### AC-162 / TC-AC-162

Setup: fake IdP và storage adapter dùng cùng contracts

Trigger duy nhất: run portability suite

Oracle: domain plugin digests không đổi; identity/data assertions giống nhau

Teardown:rollback test namespace;reset stub counters;clear tenant cache;retain evidence immutable by candidate ID.
