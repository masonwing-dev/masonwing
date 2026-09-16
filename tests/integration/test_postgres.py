"""Actual PostgreSQL role, RLS, transaction and append-only behavior; synthetic rows only."""
from uuid import uuid4

import psycopg
import pytest

pytestmark = pytest.mark.integration
A = "00000000-0000-4000-8000-00000000000a"
B = "00000000-0000-4000-8000-00000000000b"


def tenant(db, value):
    db.execute("SELECT set_config('app.tenant_id', %s, true)", (value,))


def test_runtime_role_cannot_bypass_rls_or_own_business_tables(db):
    assert db.execute("SELECT rolsuper, rolbypassrls FROM pg_roles WHERE rolname=current_user").fetchone() == (False, False)
    assert db.execute("SELECT tableowner <> current_user FROM pg_tables WHERE tablename='tenants' AND schemaname='public'").fetchone() == (True,)
    assert db.execute("SELECT relrowsecurity, relforcerowsecurity FROM pg_class WHERE oid='public.tenants'::regclass").fetchone() == (True, True)


def test_tenant_setting_cannot_leak_when_connection_is_reused(db):
    assert db.execute("SELECT id FROM tenants").fetchall() == []
    with db.transaction():
        tenant(db, A)
        assert db.execute("SELECT id::text FROM tenants").fetchall() == [(A,)]
    assert db.execute("SELECT id FROM tenants").fetchall() == []
    with db.transaction():
        tenant(db, B)
        assert db.execute("SELECT id::text FROM tenants").fetchall() == [(B,)]
    assert db.execute("SELECT id FROM tenants").fetchall() == []


def test_cross_tenant_insert_is_rejected_and_leaves_no_row(db):
    principal = uuid4()
    with pytest.raises(psycopg.errors.InsufficientPrivilege):
        with db.transaction():
            tenant(db, A)
            db.execute("INSERT INTO memberships (tenant_id,principal_id,role,status,membership_epoch) VALUES (%s,%s,'VIEWER','ACTIVE',1)", (B, principal))
    with db.transaction():
        tenant(db, B)
        assert db.execute("SELECT 1 FROM memberships WHERE principal_id=%s", (principal,)).fetchall() == []


def write_unit(db, identity):
    db.execute("INSERT INTO memberships (tenant_id,principal_id,role,status,membership_epoch) VALUES (%s,%s,'VIEWER','ACTIVE',1)", (A, identity))
    db.execute("INSERT INTO audit_events (tenant_id,id,principal_id,action,resource_type,resource_id,correlation_id) VALUES (%s,%s,%s,'fixture.created','membership',%s,%s)", (A, identity, identity, identity, identity))
    db.execute("INSERT INTO outbox_events (tenant_id,id,aggregate_id,aggregate_version,event_name) VALUES (%s,%s,%s,1,'fixture.created')", (A, identity, identity))


def test_transaction_rollback_cannot_leave_business_audit_or_outbox_partial(db):
    identity = uuid4()
    with pytest.raises(RuntimeError, match="injected"):
        with db.transaction():
            tenant(db, A)
            write_unit(db, identity)
            raise RuntimeError("injected before commit")
    with db.transaction():
        tenant(db, A)
        assert db.execute("SELECT 1 FROM memberships WHERE principal_id=%s", (identity,)).fetchall() == []
        assert db.execute("SELECT 1 FROM audit_events WHERE id=%s", (identity,)).fetchall() == []
        assert db.execute("SELECT 1 FROM outbox_events WHERE id=%s", (identity,)).fetchall() == []


def test_committed_unit_has_all_three_records_and_audit_cannot_be_rewritten(db):
    identity = uuid4()
    with db.transaction():
        tenant(db, A)
        write_unit(db, identity)
    with db.transaction():
        tenant(db, A)
        assert db.execute("SELECT 1 FROM memberships WHERE principal_id=%s", (identity,)).fetchone() == (1,)
        assert db.execute("SELECT 1 FROM audit_events WHERE id=%s", (identity,)).fetchone() == (1,)
        assert db.execute("SELECT 1 FROM outbox_events WHERE id=%s", (identity,)).fetchone() == (1,)
    with pytest.raises(psycopg.errors.InsufficientPrivilege):
        with db.transaction():
            tenant(db, A)
            db.execute("UPDATE audit_events SET action='rewritten' WHERE id=%s", (identity,))
    with db.transaction():
        tenant(db, A)
        assert db.execute("SELECT action FROM audit_events WHERE id=%s", (identity,)).fetchone() == ("fixture.created",)


def test_inbox_duplicate_event_cannot_be_inserted_twice(db):
    event = uuid4()
    with db.transaction():
        tenant(db, A)
        db.execute("INSERT INTO inbox_events (tenant_id,consumer,event_id,aggregate_sequence) VALUES (%s,'synthetic-conformance',%s,1)", (A, event))
    with pytest.raises(psycopg.errors.UniqueViolation):
        with db.transaction():
            tenant(db, A)
            db.execute("INSERT INTO inbox_events (tenant_id,consumer,event_id,aggregate_sequence) VALUES (%s,'synthetic-conformance',%s,1)", (A, event))
