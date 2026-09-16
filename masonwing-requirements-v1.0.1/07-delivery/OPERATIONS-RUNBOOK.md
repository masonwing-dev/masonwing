# Deployment, vận hành và recovery runbook

## Environments

LOCAL:mock providers,synthetic data,network egress disabled by default. INTEGRATION:real local OSS services;all external effects routed to stub. STAGING_LIVE:approved service accounts,test namespaces,low budgets,no uncontrolled public send. PRODUCTION:separate credentials,namespace,IdP clients,keys,storage and review authority. Không dùng staging token trên production.

Candidate stack:API2 replicas 4vCPU/8GiB mỗi replica;worker2 replicas tương tự;PostgreSQL4vCPU/16GiB;20GiB seeded data;local object storage;Temporal isolated DB/namespace;Keycloak dedicated DB role. Đây là benchmark profile đề xuất,không phải sizing đã đo. K8s không bắt buộc:container manifests với health checks,rolling deploy và secrets refs đủ cho baseline;production capacity cần qualification.

## Startup gate

Validate product manifest,plugin lock,contract versions,required secrets refs,IdP issuer,policy digest,db migrations and region/retention profile. Production rejects mock auth,unsigned plugins,debug secret logging,unrestricted egress and missing evidence for activated external features. Liveness process khác readiness writes;policy/DB/secret failure makes write readiness false.

## Incident playbooks

AUTH/tenant incident:activate tenant/global switch;revoke compromised sessions/grants/credentials;preserve redacted audit;identify scope;rotate keys;test negative isolation;owner-reviewed resume.

OUTCOME_UNKNOWN:do not resend. Use effect ID,provider operation key/remote revision;collect receipt evidence;confirmed present=>record success;proven absence=>fresh authorization before retry;unresolvable=>manual exception. Không click retry vì UI spinner lâu.

Budget overrun/unknown:pause billable dispatch,retain reservations,fetch usage receipts,explain cost classes;do not zero unknown balance to unblock.

Connector drift:quarantine affected operations;retain raw encrypted response;keep last successful watermark;update contract via CR;run old/new fixtures and live qualification before re-enable.

Plugin compromise:revoke artifact/signer,block new install and next host call;keep unaffected plugins running;drain/kill remote workers with outcome reconciliation;publish fixed digest after review.

## Backup và restore

Daily full backup +continuous WAL/managed equivalent;artifact manifest and immutable object versions;durable engine recovery plan;secrets backed up separately with keys not in data dump. Proposed RPO15min/RTO4h,monthly drill. Verify credentials,key access,backup integrity,restore to isolated environment,apply migrations matching snapshot,replay deletion tombstones,check referential/audit consistency,reconcile unknown external effects,switch to read-only smoke,then authorize writes. Do not pretend a restored PREPARED effect was never transmitted.

## Upgrade

Build once,promote by digest;lock/SBOM/vulnerability/license/eval reports attach exact candidate. Additive migration and backfill before switching reads;new runs to new plugin digest,old runs drained. Canary5% proposed;expand only after metrics/denial/regression review. Destructive schema step needs snapshot,forward-repair plan and explicit authorization. Rollback gate rejects old code if data cannot be read safely.

## Owner roles

Ops owns alerts/DR;Security owns incident/credential policy;Platform Lead owns runtime/schema;Product Owner owns scope/business policy;Privacy/Legal owner owns source rights/retention/jurisdiction. These are responsibility roles,not invented named staff assignments. Actual on-call and independent reviewers must be assigned before release.
