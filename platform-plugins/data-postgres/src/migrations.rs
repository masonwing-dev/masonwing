//! Operator-only SQLx migrations. Existing unversioned bootstrap schemas require
//! an explicit, verified adoption; runtime credentials never perform DDL.

use crate::store::database_failure;
use masonwing_kernel::runtime::CommandFailure;
use sqlx::{PgPool, Row, migrate::Migrator};
use std::path::Path;

pub async fn migrate(pool: &PgPool, directory: &Path) -> Result<(), CommandFailure> {
    let role: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(pool)
        .await
        .map_err(database_failure)?;
    if role != "masonwing_migrator" {
        return Err(CommandFailure::denied("MIGRATOR_ROLE_REQUIRED"));
    }
    let migrator = Migrator::new(directory)
        .await
        .map_err(|_| CommandFailure::unavailable("MIGRATION_CATALOG_INVALID"))?;
    let baseline: Option<String> = sqlx::query_scalar("SELECT to_regclass('public.tenants')::text")
        .fetch_one(pool)
        .await
        .map_err(database_failure)?;
    let history: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('public._sqlx_migrations')::text")
            .fetch_one(pool)
            .await
            .map_err(database_failure)?;
    if baseline.is_some() && history.is_none() {
        return Err(CommandFailure::precondition("BOOTSTRAP_ADOPTION_REQUIRED"));
    }
    migrator
        .run(pool)
        .await
        .map_err(|_| CommandFailure::unavailable("MIGRATION_FAILED"))?;
    Ok(())
}

/// Adopt only the known 0000/0001 schema, with operator consent expressed by
/// invoking this function. The SQLx checksums come from the actual migration
/// bytes. Existing tables, data, ownership, RLS and grants remain in place.
pub async fn adopt_bootstrap(pool: &PgPool, directory: &Path) -> Result<(), CommandFailure> {
    let role: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(pool)
        .await
        .map_err(database_failure)?;
    if role != "masonwing_migrator" {
        return Err(CommandFailure::denied("MIGRATOR_ROLE_REQUIRED"));
    }
    let migrator = Migrator::new(directory)
        .await
        .map_err(|_| CommandFailure::unavailable("MIGRATION_CATALOG_INVALID"))?;
    let mut tx = pool.begin().await.map_err(database_failure)?;
    sqlx::query("SELECT pg_advisory_xact_lock(7246732751234)")
        .execute(&mut *tx)
        .await
        .map_err(database_failure)?;
    let history: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('public._sqlx_migrations')::text")
            .fetch_one(&mut *tx)
            .await
            .map_err(database_failure)?;
    if history.is_some() {
        return Err(CommandFailure::conflict("MIGRATION_HISTORY_ALREADY_EXISTS"));
    }
    let rows = sqlx::query("SELECT c.relname,c.relrowsecurity,c.relforcerowsecurity,r.rolname FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace JOIN pg_roles r ON r.oid=c.relowner WHERE n.nspname='public' AND c.relname=ANY($1)")
        .bind(vec!["tenants","memberships","audit_events","outbox_events","inbox_events"])
        .fetch_all(&mut *tx).await.map_err(database_failure)?;
    if rows.len() != 5
        || rows.iter().any(|row| {
            row.get::<String, _>("rolname") != "masonwing_migrator"
                || !row.get::<bool, _>("relrowsecurity")
                || !row.get::<bool, _>("relforcerowsecurity")
        })
    {
        return Err(CommandFailure::precondition("BOOTSTRAP_SCHEMA_MISMATCH"));
    }
    // This manifest uses catalog column/type/nullability checks, not a successful
    // SELECT against a table with a similar name. Extra columns indicate another
    // migration may already have been applied and require operator reconciliation.
    let columns = sqlx::query("SELECT table_name,column_name,data_type,is_nullable FROM information_schema.columns WHERE table_schema='public' AND table_name=ANY($1) ORDER BY table_name,ordinal_position")
        .bind(vec!["tenants","memberships","audit_events","outbox_events","inbox_events"])
        .fetch_all(&mut *tx).await.map_err(database_failure)?;
    let expected = [
        (
            "audit_events",
            "tenant_id:id:principal_id:action:resource_type:resource_id:correlation_id:created_at",
        ),
        (
            "inbox_events",
            "tenant_id:consumer:event_id:aggregate_sequence:received_at",
        ),
        (
            "memberships",
            "tenant_id:principal_id:role:status:membership_epoch:version",
        ),
        (
            "outbox_events",
            "tenant_id:id:aggregate_id:aggregate_version:event_name:artifact_refs:delivery_status:created_at",
        ),
        ("tenants", "id:name:status:version:created_at"),
    ];
    for (table, names) in expected {
        let found: Vec<String> = columns
            .iter()
            .filter(|r| r.get::<String, _>("table_name") == table)
            .map(|r| r.get("column_name"))
            .collect();
        if found.join(":") != names {
            return Err(CommandFailure::precondition("BOOTSTRAP_SCHEMA_MISMATCH"));
        }
    }
    for row in &columns {
        let column: String = row.get("column_name");
        let expected_type = match column.as_str() {
            "tenant_id" | "id" | "principal_id" | "resource_id" | "correlation_id"
            | "aggregate_id" | "event_id" => "uuid",
            "version" | "aggregate_version" => "integer",
            "membership_epoch" | "aggregate_sequence" => "bigint",
            "created_at" | "received_at" => "timestamp with time zone",
            "artifact_refs" => "jsonb",
            _ => "text",
        };
        if row.get::<String, _>("data_type") != expected_type
            || row.get::<String, _>("is_nullable") != "NO"
        {
            return Err(CommandFailure::precondition("BOOTSTRAP_SCHEMA_MISMATCH"));
        }
    }
    let policies: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_policies WHERE schemaname='public' AND tablename=ANY($1) AND policyname='tenant_scope' AND permissive='PERMISSIVE' AND qual LIKE '%app.tenant_id%'")
        .bind(vec!["tenants","memberships","audit_events","outbox_events","inbox_events"])
        .fetch_one(&mut *tx).await.map_err(database_failure)?;
    let runtime =
        sqlx::query("SELECT rolsuper,rolbypassrls FROM pg_roles WHERE rolname='masonwing_app'")
            .fetch_one(&mut *tx)
            .await
            .map_err(database_failure)?;
    if policies != 5 || runtime.get::<bool, _>("rolsuper") || runtime.get::<bool, _>("rolbypassrls")
    {
        return Err(CommandFailure::precondition("BOOTSTRAP_SCHEMA_MISMATCH"));
    }
    let forbidden:bool=sqlx::query_scalar("SELECT has_table_privilege('masonwing_app','audit_events','UPDATE') OR has_table_privilege('masonwing_app','audit_events','DELETE') OR has_table_privilege('masonwing_app','tenants','INSERT')")
        .fetch_one(&mut *tx).await.map_err(database_failure)?;
    if forbidden {
        return Err(CommandFailure::precondition("BOOTSTRAP_SCHEMA_MISMATCH"));
    }
    sqlx::query("CREATE TABLE _sqlx_migrations(version BIGINT PRIMARY KEY,description TEXT NOT NULL,installed_on TIMESTAMPTZ NOT NULL DEFAULT now(),success BOOLEAN NOT NULL,checksum BYTEA NOT NULL,execution_time BIGINT NOT NULL)")
        .execute(&mut *tx).await.map_err(database_failure)?;
    for version in [0_i64, 1] {
        let migration = migrator
            .iter()
            .find(|m| m.version == version)
            .ok_or_else(|| CommandFailure::precondition("BOOTSTRAP_MIGRATION_MISSING"))?;
        sqlx::query("INSERT INTO _sqlx_migrations(version,description,success,checksum,execution_time) VALUES($1,$2,true,$3,0)")
            .bind(migration.version).bind(migration.description.as_ref()).bind(migration.checksum.as_ref())
            .execute(&mut *tx).await.map_err(database_failure)?;
    }
    tx.commit().await.map_err(database_failure)?;
    Ok(())
}
