\set ON_ERROR_STOP on
CREATE ROLE masonwing_migrator LOGIN NOSUPERUSER NOBYPASSRLS PASSWORD 'masonwing-local-migrator';
CREATE ROLE masonwing_app LOGIN NOSUPERUSER NOBYPASSRLS PASSWORD 'masonwing-local-app';
CREATE ROLE masonwing_keycloak LOGIN NOSUPERUSER NOBYPASSRLS PASSWORD 'masonwing-local-keycloak';
CREATE DATABASE masonwing OWNER masonwing_migrator;
CREATE DATABASE masonwing_keycloak OWNER masonwing_keycloak;
REVOKE ALL ON DATABASE masonwing FROM PUBLIC;
GRANT CONNECT ON DATABASE masonwing TO masonwing_app;
\connect masonwing
REVOKE ALL ON SCHEMA public FROM PUBLIC;
ALTER SCHEMA public OWNER TO masonwing_migrator;
SET ROLE masonwing_migrator;
\i /migrations/0000_identity.sql
\i /migrations/0001_audit_outbox.sql
RESET ROLE;
INSERT INTO tenants (id, name) VALUES
  ('00000000-0000-4000-8000-00000000000a', 'Synthetic tenant A'),
  ('00000000-0000-4000-8000-00000000000b', 'Synthetic tenant B');
