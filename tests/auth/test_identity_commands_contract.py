"""Structural checks for the WP005-WP007 identity-command migration candidate.

These checks guard schema/security properties only. They do not replace the Rust
handler tests or PostgreSQL runtime/concurrency probes.
"""

from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MIGRATION = ROOT / "infra/migrations/0005_identity_commands.sql"


def sql() -> str:
    return MIGRATION.read_text().lower()


def test_role_backfill_keeps_force_rls_and_removes_migrator_escape_hatch():
    source = sql()
    assert "create table membership_roles" in source
    assert "membership_roles_migration_backfill" in source
    assert "for select to masonwing_migrator using (true)" in source
    assert "drop policy membership_roles_migration_backfill on memberships" in source
    assert "disable row level security" not in source
    assert "alter table membership_roles force row level security" in source


def test_invites_store_digest_and_metadata_without_raw_bearer_secret():
    source = sql()
    invite_table = source.split("create table membership_invites", 1)[1].split(
        "create index membership_invites_expiry", 1
    )[0]
    assert "token_digest text not null unique" in invite_table
    assert "secret_retrieved_at" in invite_table
    assert "consumed_at" in invite_table
    assert "invite_token" not in invite_table
    assert "raw_token" not in invite_table
    assert "grant select, insert, update on membership_invites to masonwing_app" in source
    assert "membership_invites_integrity" in source
    assert "membership invitation authority fields are immutable" in source
    assert "membership invitation secret retrieval is one-use" in source
    assert "membership invitation consumption is immutable" in source


def test_role_and_invite_tables_are_tenant_force_rls_scoped():
    source = sql()
    for table in ("membership_roles", "membership_invites"):
        assert f"alter table {table} enable row level security" in source
        assert f"alter table {table} force row level security" in source
    assert source.count("nullif(current_setting('app.tenant_id', true), '')::uuid") >= 4
    for table in (
        "identity_configuration_proposals",
        "policy_configuration_state",
        "policy_proposals",
    ):
        assert f"'{table}'" in source
    assert "execute format('alter table %i force row level security'" in source


def test_policy_and_identity_proposals_cannot_activate_runtime_authority():
    source = sql()
    assert "create table policy_proposals" in source
    assert "create table identity_configuration_proposals" in source
    assert "default 'pending'" in source
    assert "default 'unqualified'" in source
    assert "grant select, insert on policy_proposals to masonwing_app" in source
    assert "grant select, insert on identity_configuration_proposals to masonwing_app" in source
    assert "grant select, insert, update on identity_configuration_proposals" not in source
    assert "update authorization_policies" not in source
    assert "insert into authorization_policies" not in source


def test_policy_version_counter_is_separate_from_published_policy_rows():
    source = sql()
    assert "create table policy_configuration_state" in source
    assert "grant select, insert, update on policy_configuration_state to masonwing_app" in source
    proposal = source.split("create table policy_proposals", 1)[1]
    assert "base_policy_version" in proposal
    assert "base_policy_epoch" in proposal
