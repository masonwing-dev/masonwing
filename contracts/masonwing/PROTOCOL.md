# Protocol và serialization

Contract version 1.0.0. `shared-contracts.json` giống byte-for-byte giữa hai gói; `contracts.json` thêm domain types/command inputs. JSON Schema draft2020-12 là wire validation; `openapi.json` là HTTP contract. Các trường business phải đi qua checks cross-field mô tả dưới,không chỉ JSON Schema.

## Identity và primitives

IDs là opaque,server-generated >=128 bits entropy; không thể dùng ID possession thay auth. Fixture aliases `tenant_a`,`asset_a` chỉ là tên dữ liệu test. Timestamps RFC3339 UTC; timestamps provider có original timezone riêng. Money dùng integer microunits cho cost,minor currency units cho CRM outcome; range<=2^53-1 để frontend không mất precision. Không float currency. Digest `sha256:<64 lowercase hex>` tính trên bytes; structured content dùng UTF-8 canonical JSON với ordering deterministically documented. Content digest không chứa mutable timestamp/signature wrapper.

## Commands

Closed operation catalog; không có dynamic SQL/eval endpoint. Tenant từ path và verified membership; actor từ session/service token; không nhận permission claims từ request body. `Idempotency-Key` bắt buộc cho mọi command có tác động,scope=(tenant,principal,operation,key). Fingerprint=canonical operation+input+target version,loại correlation ID. Same key+same fingerprint trả receipt gốc; mismatch409. API command dedupe retention24h là proposed profile; effect logical identity/receipt retained theo business retention,không tự reset sau24h.

Domain writes `expected_version` được kiểm cùng transaction; stale409. Không tự retry optimistic conflict với latest body. Không replay unsafe request khi chưa biết outcome. 202 là accepted,200 là synchronous completion; result cho logical run/effect ở resource projection. Nếu outbox commit đã xảy ra mà response mất,client retry cùng key.

## Error and confidentiality

Error gồm code,message,correlation_id,effect_state,retryable,recovery_action,details. Không trả stack trace,token,raw provider body hoặc cross-tenant identifiers. 400 invalid syntax/schema,401 authentication,403 permission/CSRF,404 absent-or-forbidden cross-tenant,409 stale/idempotency/transition conflict,413 oversized,422 policy precondition,429 capacity,503 critical dependency. Nonexistent and inaccessible object dùng same shape; timing không được quảng cáo constant-time tuyệt đối,cần negative disclosure tests.

## Reads và artifacts

Collection endpoint có cursor opaque bound tenant,principal epoch,filters,sort,snapshot and expiry. Sort mặc định(created_at,id) stable; limit default50,max200. Filters là closed fields được mô tả ở operation/resource catalog; không đưa raw SQL. Invalid cursor400;expired history410 SNAPSHOT_REQUIRED. Count phải là authorized count,không dataset total.

Resource GET trả typed ResourceProjection và authorized ArtifactRef. Artifact document có content-type application/json và schema_name trong metadata; bytes validate đúng schema được versioned. Upload begin cấp upload locator riêng; finalize kiểm bytes size/digest/content-type/malware rồi mới cấp active ref. Confidential/restricted downloads đi proxy `/v1/tenants/{tenant_id}/artifacts/{artifact_id}/content`;public/internal có time-bound locator. Data plane endpoints chi tiết ở `data-plane.json`,không để plugin tự mở bucket.

## Events

At-least-once bus; dedupe(tenant,consumer,event_id); per-aggregate sequence tăng trong transaction; không hứa global ordering. Body chứa artifact reference không raw secret/draft. SSE `id:event_id`,`event:type`,`data:EventEnvelope`; heartbeat15sec;replay window24h proposed; current authorization cho mỗi page replay; gap410 và snapshot refetch. Unrecognized event major => quarantine,không silently drop.

## API evolution

Minor chỉ thêm optional fields/enum khi consumer capability hỗ trợ; command input strict additionalProperties=false được negotiated theo exact schema_version,không gửi new fields vào old server. Required field removal,meaning change,state semantic change=>major. Support current+previous minor major1 là mục tiêu acceptance,không blanket compatible unknown enum. Unknown provider native items giữ raw encrypted envelope nhưng không execute nếu tool type unsupported.

## Cross-field semantic invariants

Expiry>creation;end>=start;money currencies same trong một ledger;settled<=reserved upper bound hoặc explicit overrun incident;source spans end>start và trong snapshot;artifact tenant=host tenant;child grant subset;source rights live chưa expire;claim SUPPORTED phải có source;metric0eligible=>value=null;failed+eligible+ineligible=planned;publication receipt target đúng approved digest/version;unsupported provider capability không auto downgrade.

## Idempotency và transition oracle

`state-machines.json` liệt kê adjacency hợp lệ; guard thiếu vẫn deny dù adjacency đúng. Same-state retry chỉ idempotent khi command key/fingerprint khớp; không phải transition tự do. Illegal transition409 preserves version and no effect. State fixture tests trong testing là **spec model vectors**,không được ghi là runtime state machine đã chạy.

## Typed handshake results

`artifact.begin` returns `CommandReceipt(state=SUCCEEDED,resource.resource_type=UploadSession)`; resource and artifact ID are allocated atomically, run_id/effect_id null. Read the typed UploadSession through ResourceProjection to obtain upload_path. `artifact.finalize` returns ArtifactRef metadata after digest/size validation and uses the exact artifact_id from that session. Repeating begin with the same idempotency key returns the same session. Expired sessions never reopen silently.

`connection.authorize` returns `CommandReceipt(state=SUCCEEDED,resource.resource_type=ConnectorAuthorization)`; use only its prevalidated authorization_url after current authorization. The URL contains an opaque provider authorization state, not a refresh token/client secret. The connector callback is an internal adapter endpoint registered in the exact IdP/provider redirect allowlist; it verifies tenant-bound one-use state+PKCE, updates Connection, and redirects to a prevalidated relative return_to. Never accept a tenant supplied in callback query as authority.

All other asynchronous command receipts return a non-null run_id and ACCEPTED, or a synchronous existing result resource with SUCCEEDED. SUCCEEDED describes the local command, not confirmed external publication. A completed handler's typed result is bound to the output_schema_ref in its pinned HandlerContract/WorkflowDefinition and made available as a ResourceProjection artifact; an absent required output fails the run. No successful response is allowed to leave resource,run_id,effect_id all null except an idempotent no-op documented by its action.
