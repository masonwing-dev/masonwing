# Authentication, authorization và delegation

## OIDC BFF

Keycloak giữ password,passkeys,MFA,SSO và recovery. Rust BFF dùng `openidconnect`; session dùng `tower-sessions` và SQLx store. Dùng Code+PKCE S256, random state/nonce dùng một lần, issuer+client audience allowlist, JWKS alg allowlist và expiry/nbf/skew validation. Không nhận password để proxy token grant; không implicit flow.

`GET /auth/login?return_to=/...`: chỉ relative local allowlisted path, chống `//`, encoded slash/backslash,CRLF,open redirect. Tạo auth transaction server-side 5 phút và cookie binding; final session cookie khác auth transaction ID. `GET /auth/callback?code=&state=`: token exchange backend, validate issuer/aud/azp khi applicable/nonce/exp/iat/signature, consume auth transaction atomically, rotate session ID. Provider error callback trả lỗi sanitized và xóa transaction; query OAuth code không vào access logs.

Cookie: `__Host-masonwing_session`, Secure,HttpOnly,Path=/,không Domain,SameSite=Lax cho redirect GET flow. Dev loopback dùng profile rõ riêng; production không cho insecure cookie. Session idle30 phút,absolute12 giờ là proposed defaults; step-up5 phút. Tokens giữ ngoài browser; session record chỉ IDs và secret refs. Cleanup expired SQLx sessions chạy60 giây; expiry enforcement kiểm từng request, không chờ cleanup.

Cookie unsafe requests phải cùng configured Origin và synchronizer CSRF token; validate content-type, reject unsupported form bodies. Không xem CORS là CSRF control. BFF same-origin với React; allowlist CORS nếu phải khác origin,không `*` kèm credentials. Trusted proxy headers cấu hình cố định; không trust X-Forwarded-* từ public.

## Token lifecycle

JWKS cache theo issuer; unknown kid chỉ bounded refresh và fail closed nếu không validate được. Không algorithm confusion hoặc token audience substitution. Refresh-token rotation serialize theo connection/session version; latest committed token không bị stale worker overwrite. `invalid_grant` -> NEEDS_REAUTH,không retry vô hạn. Secret refs không chứa token bytes.

Local logout invalidates session ngay; Keycloak logout là follow-up riêng. Backchannel token phải verify signature,issuer,audience,events,sid/sub và jti replay; duplicate đúng token idempotent. Khóa user/membership cần invalidation hook/polling profile và current write checks; chưa có IdP webhook không được hứa tức thì cho mọi external account change.

## Tenant/membership

Một realm cho deployment, client tách sản phẩm/environment; một realm mỗi tenant **không** là mặc định. Tenant memberships nằm platform; Keycloak organizations không tự thay thế data authorization. Account identity=issuer+sub; không auto-link email. Invitation one-use,verified email eligibility,TTL; không cấp owner qua domain email suffix. Last-owner demotion bị chặn; ownership transfer giao dịch rõ và step-up. Provisioning tenants chỉ admin/owner đúng action,không self signup mặc định.

Permissions check server trên mỗi command,tool,export,replay,signed locator. Resource B từ A trả same404 như absent,bao gồm search/facets/counts. Cache key gồm tenant,principal/permission epoch,query và schema version. Kết nối DB SET LOCAL tenant trong transaction; luôn reset/pool tests. Role runtime không table owner,superuser hoặc BYPASSRLS; FORCE RLS là defense-in-depth [WEB-PG].

## Cedar

Host constructs principal/action/resource/context từ records tin cậy; plugin không gửi entity bag có quyền tự tạo. Default deny;forbid overrides; **any evaluation diagnostic => deny** trong adapter của framework, vì native Cedar có skip-on-error [WEB-CEDAR]. Validate policies trước publication; policy versions immutable; snapshot phục vụ audit nhưng current deny có hiệu lực trước write. Không có matching permit ->deny; policy unavailable->503 +no mutation.

## Machine và run grants

Client Credentials xác thực service ở đúng audience/environment. Machine token không trao mọi tenant. Dispatch cần RunContext gắn grant với tenant,principal,parent grant,actions,resource set,expiry và fence. Child grant chỉ subset. Scheduled automation dùng owner-approved automation grant,không kéo dài browser session. Agent không thay policy,authorize chính nó hoặc tự approve proposal. Reviewer quyền phê duyệt khác Publisher quyền thực thi; high-risk có separation-of-duties khi active policy yêu cầu.

## Secrets và OAuth connectors

Login Google không cấp Search Console scopes. Connector consent có OAuth state và token store riêng,tenant/external account/scopes riêng. Dùng oauth2 crate,OpenBao/approved manager. Domain plugin chỉ connection handle; connector executor được tin cậy đọc secret vừa đủ để gửi. Audience token không được passthrough tới MCP server. Secret values không vào provider prompts,traces,crash dumps/screenshots; backup keys tách database.

## Revocation semantics

Protected writes đọc current authority ngay trước broker transmit; cached permit không đủ. Read lease tối đa15 giây là proposed target; SSE đóng và tenant cache purge theo epoch. Bytes người dùng đã tải không thể bị thu hồi. Confidential/restricted artifact tải qua authorized proxy,không direct pre-signed URL; public/internal locator TTL60 giây tối đa và không quảng cáo immediate revocation. Logged-out browser clear app caches; downloaded files nằm ngoài control phải nêu rõ.

## Support/admin

Break-glass không có password universal. Support grant scope,time bound,owner approval và audit; default metadata-only,15 phút đề xuất. Signer/release operator không mặc nhiên đọc tenant content. Admin UI không là permission bypass. Unsafe dev auth mock,unsigned plugin và permissive policy bị startup production validator chặn.
