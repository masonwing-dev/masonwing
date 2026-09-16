# Data model, ownership và storage

Typed scalar columns theo data-dictionary;arrays/snapshots có JSON Schema dùng JSONB khi appropriate cho immutable aggregate,không schema-less EAV toàn hệ thống. Business identifiers,sources,versions,state,tenant,time,currency/index keys là columns có type. Raw content/media trong artifact store;PostgreSQL chứa refs/digests.

| Entity | Owner module | Key/index | Retention/lifecycle |
|---|---|---|---|
| Run | platform owner | primary opaque ID;tenant+state+updated_at;unique logical key per contract | immutable versions or audited optimistic updates;rights/deletion policy applies |
| Membership | platform owner | primary opaque ID;tenant+state+updated_at;unique logical key per contract | immutable versions or audited optimistic updates;rights/deletion policy applies |
| Delegation | platform owner | primary opaque ID;tenant+state+updated_at;unique logical key per contract | immutable versions or audited optimistic updates;rights/deletion policy applies |
| EffectIntent | platform owner | primary opaque ID;tenant+state+updated_at;unique logical key per contract | immutable versions or audited optimistic updates;rights/deletion policy applies |
| EffectReceipt | platform owner | primary opaque ID;tenant+state+updated_at;unique logical key per contract | immutable versions or audited optimistic updates;rights/deletion policy applies |
| CostReservation | platform owner | primary opaque ID;tenant+state+updated_at;unique logical key per contract | immutable versions or audited optimistic updates;rights/deletion policy applies |
| Connection | platform owner | primary opaque ID;tenant+state+updated_at;unique logical key per contract | immutable versions or audited optimistic updates;rights/deletion policy applies |
| PluginManifest | platform owner | primary opaque ID;tenant+state+updated_at;unique logical key per contract | immutable versions or audited optimistic updates;rights/deletion policy applies |
| ProviderProfile | platform owner | primary opaque ID;tenant+state+updated_at;unique logical key per contract | immutable versions or audited optimistic updates;rights/deletion policy applies |
| ProviderNativeEnvelope | platform owner | primary opaque ID;tenant+state+updated_at;unique logical key per contract | immutable versions or audited optimistic updates;rights/deletion policy applies |
| ApprovalBinding | platform owner | primary opaque ID;tenant+state+updated_at;unique logical key per contract | immutable versions or audited optimistic updates;rights/deletion policy applies |
| EventEnvelope | platform owner | primary opaque ID;tenant+state+updated_at;unique logical key per contract | immutable versions or audited optimistic updates;rights/deletion policy applies |

## Relational invariants

Tenant-scoped tables reference tenant identity;cross-resource references require same tenant at command validation and DB composite key where ownership permits. Unique `(tenant_id,operation,idempotency_key,principal_id)`;effect fingerprint immutable. Unique inbox consumer+event and aggregate sequence. Approval has exact proposal version/digest;effect references approval,reservation,connection and target;receipt immutable references effect. Membership unique tenant+principal;last-owner removal transaction locks tenant membership set. Plugin artifact digest globally immutable;tenant plugin-installation record separate from global artifact metadata.

Marketing:SourceItem revision unique(source_id,external_id,revision);PainCluster membership(source_item_revision) distinct;ClaimEvidence edges pin artifact+span;ContentRevision unique(asset_id,revision);block IDs unique per revision;URL reservation unique(brand,locale,normalized canonical);Publication references immutable ContentRevision;OutcomeEvent unique(tenant,provider,event_id,revision);currency never implicit. Measurements carry panel/model/config/window/surface so incompatible snapshots cannot be merged accidentally.

## Migration plan

M000 identity/tenant/core contracts;M001 audit,outbox,inbox,artifact metadata;M002 memberships/grants/policy;M003 run/effect/approval/budget;M004 plugin registry/catalog;marketing M100 brand/rights/source;M101 VOC/opportunity/inventory;M102 evidence/content/proposals;M103 measurements/outcomes/experiments;M104 media/distribution. Each migration has checksum,owner,forward/backward compatibility and lock strategy. Third-party session/IdP/Temporal migrations owned by their adapter,not rewritten manually.

Runtime no DDL privilege. Installer validates manifest migration permissions;backup/restore prerequisites before destructive step. Additive columns nullable/backfilled before constraint;old and new worker compatibility tested. Wrong-tenant insert and pooled connection leak are separate tests. Immutable records do not prevent lawful deletion:approved retention erasure uses tombstone/redacted proof under privileged audited procedure,not silent update from normal API.

## Data invariants validation

JSON Schema validates serialization,not foreign-key validity or rights. Cross-field semantic assertions from PROTOCOL.md must run inside authoritative transaction for mutation constraints. Domain retrieval cannot trust only cached schema-valid data. Dirty restored/projection rows cause quarantine or rebuild,not manufactured success.
