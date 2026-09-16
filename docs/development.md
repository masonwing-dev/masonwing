# Local development

The workspace runs under Compose project **masonwing-dev**. The checked-in image
references pin registry digests. The scripts operate on this project only and
never remove volumes. `.env.example` contains **synthetic local credentials**;
`scripts/dev.py` copies it to a mode-0600 `.env` only when `.env` is missing.

## Start and inspect

```sh
make bootstrap
make dev-up
make dev-status
make test-integration
```

`make dev-up` checks every published port before building, builds one shared Rust
image plus the web and fixture images, starts the stack, waits for liveness,
then seeds the private versioned S3 bucket and tenant-scoped OpenBao fixtures.
An occupied port from another process is an error; the script does not kill it
or silently select another port.

| Host endpoint | Service | Purpose |
| --- | --- | --- |
| http://localhost:39850 | React/Vite web | Developer shell and all 45 feature routes |
| http://localhost:39851/dev/status | Rust BFF scaffold | Actual local application status |
| 127.0.0.1:39852 | PostgreSQL | Business database `masonwing` |
| http://localhost:39853 | Keycloak | Imported `masonwing` realm |
| 127.0.0.1:39854 | Temporal gRPC | `masonwing-local`, `gleanbird-local` namespaces |
| http://localhost:39855 | Temporal UI | Workflow namespace/history inspection |
| http://localhost:39856 | S3 API | `masonwing-artifacts-local` bucket |
| http://localhost:39857 | Storage console | Browse synthetic local artifacts |
| http://localhost:39858/ui | OpenBao | Local secret fixtures and policies |
| http://localhost:39859/health | Provider fixtures | Synthetic provider fault harness |

All ten host bindings use `127.0.0.1`. A fixed-destination HAProxy TCP gateway
publishes them; application and dependency containers remain on an internal
Docker network. Only the gateway also joins the edge network. It is not a
general forward proxy. This preserves the local port contract on the inspected
OrbStack engine, where ports on an internal-only container were not published.
The gateway's private health port 8404 is not published on the host.

The worker, component runner and remote runner run on private container ports.
They currently serve status and handle shutdown; they do not yet execute
Temporal tasks, Wasm components, browser jobs or media renders. Container
liveness is not evidence that these adapters have been implemented.

## Local accounts and data

Keycloak administrator: `admin` / `masonwing-local-admin` by default. The realm
contains `developer` / `masonwing-local-developer`. The confidential client
`masonwing-local-bff` requires authorization code + S256 PKCE and permits only
`http://localhost:39850/auth/callback`. Password grants and implicit flow are off.
The Rust BFF login/session adapter remains unimplemented; these identity fixtures
do not constitute a working application login or grant application ownership.

Database runtime role: `masonwing_app` / `masonwing-local-app`; migration owner:
`masonwing_migrator` / `masonwing-local-migrator`. The runtime role is not a table
owner, superuser or BYPASSRLS role. Fixtures map `tenant_a` to
`00000000-0000-4000-8000-00000000000a` and `tenant_b` to the same UUID ending `b`.
The `local-dev` and `local-brand` route parameters are developer catalog labels,
not authenticated tenant/brand selections.

Storage credentials default to `masonwing-local` / `masonwing-local-storage`.
OpenBao dev root token defaults to `masonwing-local-root`; fixture reader policies
allow only their tenant's path. Root storage/Bao credentials are not passed to
the application runtime. Production credentials must never reuse these values.

PostgreSQL, S3, the synthetic provider's SQLite state and Temporal's local SQLite
state persist in named project volumes. **OpenBao runs in dev mode and its state
is in memory.** After recreating/restarting OpenBao run `make dev-seed`.
Temporal is the real OSS engine launched by its local-development CLI; this
configuration is not a production Temporal cluster or qualification of its Rust
adapter. The database initialization SQL applies only to a fresh database volume;
see `infra/migrations/README.md` before introducing later migrations.

## Edit, rebuild and test

The four browser package `src` directories are mounted read-only into Vite for
hot reload. Dependencies are installed inside the image, not mounted from macOS.
After changing JavaScript dependencies rebuild web. Rust edits require rebuilding
the shared runtime and recreating its services:

```sh
docker compose --project-name masonwing-dev build api
docker compose --project-name masonwing-dev up -d --no-build api worker component-runner remote-runner
make check
make test
make test-integration
pnpm exec playwright install chromium
make test-e2e
```

The browser suite uses the already-running Compose web; it does not spawn a
second dev server or take another host port. `python3 scripts/verify.py
--integration --browser` records commands, exit codes, durations and logs under
`.dev/evidence/`. Test output is local scaffold evidence, not product release
approval. Development commands can be run from the native project directory.

```sh
make dev-logs
make dev-down
```

`dev-down` stops only this Compose project and retains its data. There is no
automatic volume reset command. Existing other projects/services are not stopped.

## Health semantics

`/health/live` is 200 while the Rust process is alive. `/health/ready` is
deliberately **503**, with `writes=false` and
`critical_adapters_qualified=false`, until real adapters and their required
evidence are implemented. `/dev/status` explicitly reports `NOT_IMPLEMENTED`,
no external mutations and a zero live budget. No command returns a fabricated
successful business receipt. Fake bearer values are not authenticated users.

These distinctions are shown in the web shell and asserted in the HTTP tests.
The local services can all be healthy while product write readiness is false.
