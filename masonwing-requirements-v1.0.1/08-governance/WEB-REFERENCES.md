# Tài liệu kỹ thuật được xác minh bổ sung

Checked:2026-09-16. Các URL dưới là nguồn chính thức. Quy tắc triển khai cụ thể do tác giả đề xuất được phân biệt trong ADR/assumptions,không là lời hứa của provider. Source nghiên cứu gốc giữ nguyên,nhưng các citations cũ trong đó không tự được xem là xác minh mới.

| ID | Tài liệu | URL | Phạm vi dùng |
|---|---|---|---|
| WEB-KC | Keycloak OIDC layers | https://www.keycloak.org/securing-apps/oidc-layers | Code flow,client credentials,introspection; không là qualification deployment |
| WEB-OIDC | openidconnect Rust documentation | https://docs.rs/openidconnect/latest/openidconnect/ | OIDC client và HTTP redirect SSRF warning |
| WEB-SESSION | tower-sessions | https://docs.rs/tower-sessions/latest/tower_sessions/ | SessionStore/SQLx và expiry cleanup |
| WEB-CEDAR | Cedar authorization semantics | https://docs.cedarpolicy.com/auth/authorization.html | default deny,forbid overrides,skip-on-error; framework thêm diagnostics-deny |
| WEB-COMPACT | OpenAI compaction | https://developers.openai.com/api/docs/guides/compaction | standalone complete returned window; server-side config |
| WEB-STATE | OpenAI conversation state | https://developers.openai.com/api/docs/guides/conversation-state | preserve output items,phase và reasoning continuation |
| WEB-TEMPORAL | Temporal Rust guide | https://docs.temporal.io/develop/rust | Official Rust workflow/activity API documentation |
| WEB-TEMPORAL-PREVIEW | Temporal Rust public preview notice | https://temporal.io/changelog/rust-sdk-public-preview | Thông báo public preview ngày07/05/2026; không tự suy ra GA hay production qualification |
| WEB-WASM | Wasmtime introduction | https://docs.wasmtime.dev/ | Wasm/WASI/Component runtime |
| WEB-WASM-LIMIT | Wasmtime interruption | https://docs.wasmtime.dev/examples-interrupting-wasm.html | fuel và epoch interruption; host I/O có controls riêng trong proposed architecture |
| WEB-WP | WordPress posts REST reference | https://developer.wordpress.org/rest-api/reference/posts/ | CRUD fields/status; custom CAS/idempotency bridge là thiết kế bổ sung,không WP built-in guarantee |
| WEB-GSC | Search Console Search Analytics query | https://developers.google.com/webmaster-tools/v1/searchanalytics/query | API top-row/internal coverage limitations |
| WEB-PG | PostgreSQL row security | https://www.postgresql.org/docs/current/ddl-rowsecurity.html | owner/superuser/BYPASSRLS exceptions |
| WEB-SHADCN | shadcn monorepo | https://ui.shadcn.com/docs/monorepo | shared UI workspace and monorepo installation |
| WEB-TAILWIND | Tailwind class detection | https://tailwindcss.com/docs/detecting-classes-in-source-files | static class detection and explicit @source |
