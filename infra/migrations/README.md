# Database scaffold and migration ownership

The fresh-volume bootstrap imports `0000_identity.sql` and
`0001_audit_outbox.sql` as `masonwing_migrator`, then seeds two synthetic tenants.
The runtime role has FORCE RLS and transaction-local tenant policies, append-only
audit grants and inbox/outbox constraints. Actual PostgreSQL tests verify these
database mechanisms. They do not prove that application handlers already use the
required transaction or recheck current authority.

This is an initial DDL scaffold, **not an implemented SQLx migration runner**.
Existing volumes are never modified automatically by changing the bootstrap
files. Before the first incremental SQLx migration, adopt this initial schema
into the SQLx migration history with verified checksums, inspect any existing
synthetic data and test upgrades from both a fresh volume and this baseline.
Do not reset a developer's volume to make a migration pass.

| Baseline migration | Current ownership / next slice |
| --- | --- |
| M000 identity/tenant/core contracts | Initial tenant DDL present; maintained OIDC/tower-sessions store is adapter work |
| M001 audit/outbox/inbox/artifact metadata | Initial audit/outbox/inbox DDL present; artifact metadata and transactional repository remain adapter work |
| M002 memberships/grants/policy | Membership DDL bootstrapped early for RLS tests; grants and policy persistence remain unimplemented |
| M003 runs, approvals, effects, budgets | Version/fence/state persistence and atomic dispatch reservation under workflow/effect owners |
| M004 plugin registry/catalog | Registry, installation, pinned artifacts and catalog ownership |
| M100 Gleanbird | Brand/rights/source |
| M101 Gleanbird | VOC/opportunity/inventory |
| M102 Gleanbird | Evidence/content/proposals |
| M103 Gleanbird | Measurements/outcomes/experiments |
| M104 Gleanbird | Media/distribution |

The exact order, descriptions and preconditions must be checked against the
immutable data model and work packages before implementing each migration. This
table groups delivery ownership and does not replace their schema contracts.
Use additive SQLx migrations, explicit non-owner runtime roles, schema ownership
per plugin and tested rollback/restore plans. Never infer permission from a
browser-provided tenant or retain `SET` tenant state across pooled transactions.
