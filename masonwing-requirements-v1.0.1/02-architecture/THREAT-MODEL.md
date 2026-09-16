# Threat model và trust boundaries

Assets: tenant business data,evidence,credentials,private drafts,approval intent,budget ledger,publisher identity,plugin supply chain. Actors: tenant user,agent,external source,plugin author,service worker,operator,provider và attacker. Các trust boundaries: browser/BFF; IdP/BFF; tenant/data broker; plugin/host; host/provider; CI/signing; backups/restore; support/tenant.

| Attack | Biện pháp bắt buộc | Oracle âm tính |
|---|---|---|
| IDOR/tenant substitution | Scope từ authenticated context,current policy,RLS | unauthorized body,hits,counts,URLs=0 |
| Stale grants/TOCTOU | recheck current authority trước PONR,version/fence | transmit=0 sau revoke trước PONR |
| Prompt injection | source là data,capability whitelist,writer bounded inputs | no new tools,secrets,grants |
| Malicious plugin | signature+review,sandbox,egress broker,quotas | host sống; private file/network calls=0 |
| UI plugin XSS | different-origin iframe or declarative renderer,schema bridge | host DOM/cookies unavailable |
| SSRF/DNS rebinding | validate initial/redirect DNS,IP ranges,ports,SNI/TLS | no TCP tới forbidden address |
| Confused deputy OAuth | state binding,audience separation,scopes per connection | token passthrough=0 |
| CSV/HTML/SVG injection | export escaping,sanitization,content-type validation | no formula/script execution |
| Retry duplicate publish | intent key,provider evidence,unknown state | no blind retransmission |
| Replay after backup | deletion tombstone,remote reconcile before writes | no resurrected access or duplicate send |
| Cost exhaustion | reservation upper bounds,fan-out caps,fairness | admitted<=budget; no unknown=zero |
| Supply chain substitution | signed digest,lockfile,SBOM,publisher revoke | artifact quarantined before execute |
| Fake acceptance evidence | exact tree/hash/profile,nonempty assertions,independent review | no product PASS from spec checker |

## Privacy lifecycle

Classify at acquisition; purpose/rights before AI request; minimize raw data; prompt logging off by default. Every export/artifact has classification and expiry. Private data không tái sử dụng cross-tenant cho training/benchmark. Data retention values trong config là proposed test profile,không phải diễn giải pháp luật. Production policy phải chỉ định region,lawful basis,DPA,retention,backup horizon,legal hold và responsible owner. Xóa third-party copies cần receipt hoặc trạng thái chưa giải quyết; không tự hứa hoàn tác email đã gửi.

## Residual risks

Sandbox hoặc signature không chứng minh plugin hoàn toàn an toàn. Model may hallucinate; validators và human review giảm rủi ro,không chứng minh không thể sai. Provider có thể thay API/ToS; capability quarantine cần manual action. External publications có thể bị cache hoặc copied. Compromised operator/IdP/secret manager vẫn là high-trust risk; cần access review và production security assessment độc lập.
