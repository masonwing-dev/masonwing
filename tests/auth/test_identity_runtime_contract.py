"""Structural contract for WP005-WP007 auth storage; not product acceptance."""

from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MIGRATION = ROOT / "infra/migrations/0002_identity_runtime.sql"


def sql() -> str:
    return MIGRATION.read_text().lower()


def test_oidc_identity_key_is_issuer_subject_and_email_cannot_merge_accounts():
    source = sql()
    assert "unique (issuer, subject)" in source
    assert "unique (email" not in source
    assert "oidc_identity_key_immutable" in source


def test_membership_authority_has_stable_id_and_monotonic_permission_epoch():
    source = sql()
    # The migration must initialize existing rows without an UPDATE that FORCE
    # RLS could silently hide from the migration role. PostgreSQL materializes
    # both ADD COLUMN expressions for existing rows; permission_epoch is then
    # converted to a normal mutable column while preserving membership_epoch.
    assert "add column membership_id uuid not null default gen_random_uuid()" in source
    assert "add column permission_epoch bigint generated always as (membership_epoch) stored" in source
    assert "alter column permission_epoch drop expression" in source
    assert "alter column permission_epoch set default 1" in source
    assert "update memberships\nset membership_id" not in source
    assert "memberships_bump_authority_epochs" in source
    assert "old.membership_epoch + 1" in source
    assert "old.permission_epoch + 1" in source


def test_oidc_transaction_and_tower_session_storage_are_server_side():
    source = sql()
    assert "create table oidc_auth_transactions" in source
    assert "pkce_verifier text not null" in source
    assert "browser_binding text not null" in source
    assert "create table if not exists tower_sessions.session" in source
    assert "data bytea not null" in source


def test_cedar_current_policy_is_tenant_scoped_and_version_rows_are_immutable():
    source = sql()
    assert "alter table authorization_policies force row level security" in source
    assert "authorization_policies_one_current_per_tenant" in source
    assert "authorization_policy_versions_immutable" in source
    assert "policy_epoch bigint not null" in source
