import { test, expect } from '@playwright/test';
import { createHash, randomUUID } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';

// This exercises real loopback Keycloak/BFF/PostgreSQL/MinIO/ClamAV. OAuth codes,
// cookies and CSRF values must never be retained in a trace or attached response.
test.use({ trace: 'off', video: 'off', screenshot: 'off' });

const origin = 'http://localhost:39850';
const issuer = 'http://localhost:39853/realms/masonwing';

function uuid(value: unknown): string {
  if (typeof value !== 'string' || !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value)) {
    throw new Error('Expected an opaque UUID from the verified local identity');
  }
  return value;
}

function operatorSql(statement: string): void {
  execFileSync('docker', ['compose', '--project-name', 'masonwing-dev', 'exec', '-T', 'postgres',
    'psql', '-U', 'masonwing_migrator', '-d', 'masonwing', '-v', 'ON_ERROR_STOP=1', '-q'], {
    input: statement, timeout: 10_000, stdio: ['pipe', 'ignore', 'pipe'],
  });
}

function provisionTenant(tenant: string, principal: string): void {
  uuid(tenant); uuid(principal);
  const membership = randomUUID();
  operatorSql(`BEGIN;
    SELECT set_config('app.tenant_id','${tenant}',true);
    INSERT INTO tenants(id,name) VALUES('${tenant}','Ephemeral browser integration');
    INSERT INTO memberships(tenant_id,principal_id,membership_id,role,status,membership_epoch,permission_epoch)
      VALUES('${tenant}','${principal}','${membership}','OWNER','ACTIVE',1,1);
    INSERT INTO membership_roles(tenant_id,membership_id,role) VALUES('${tenant}','${membership}','OWNER');
    INSERT INTO authorization_policies(tenant_id,policy_version,policy_epoch,cedar_source,is_current)
      VALUES('${tenant}','1.0.0',1,'permit(principal, action, resource); forbid(principal, action == Action::"effect.dispatch", resource);',true);
    COMMIT;`);
}

function removeTenant(tenant: string): void {
  uuid(tenant);
  // Only this test's random tenant. The verified identity and unrelated accounts,
  // infrastructure services, volumes and immutable object versions are retained.
  const tables = ['resource_projections', 'read_cursors', 'artifact_lineage', 'artifact_rights', 'uploads',
    'command_receipts', 'audit_events', 'outbox_events', 'artifacts', 'authorization_policies', 'membership_roles', 'memberships'];
  operatorSql(`BEGIN; SELECT set_config('app.tenant_id','${tenant}',true);
    ${tables.map(table => `DELETE FROM ${table} WHERE tenant_id='${tenant}';`).join('\n')}
    DELETE FROM tenants WHERE id='${tenant}'; COMMIT;`);
}

test('real OIDC session, tenant admission, CSRF and verified upload round trip', async ({ page, context }, testInfo) => {
  test.setTimeout(120_000);
  const tenant = randomUUID();
  let provisioned = false;
  let csrf = '';
  let report: Record<string, unknown> | undefined;
  try {
    await page.goto('/');
    await expect(page.getByRole('link', { name: 'Đăng nhập', exact: true })).toBeVisible();
    expect((await context.request.get('/session')).status()).toBe(401);
    await page.getByRole('link', { name: 'Đăng nhập', exact: true }).click();
    await expect.poll(() => new URL(page.url()).origin).toBe('http://localhost:39853');
    await page.locator('#username').fill('developer');
    await page.locator('#password').fill('masonwing-local-developer');
    await page.locator('#kc-login').click();
    await expect.poll(() => new URL(page.url()).origin === origin && new URL(page.url()).pathname === '/', { timeout: 30_000 }).toBe(true);

    const sessionResponse = await context.request.get('/session');
    expect(sessionResponse.status()).toBe(200);
    const session = await sessionResponse.json();
    expect(session.authenticated === true).toBe(true);
    expect(session.principal?.issuer === issuer).toBe(true);
    const principal = uuid(session.principal?.id);
    csrf = typeof session.csrf_token === 'string' ? session.csrf_token : '';
    expect(csrf.length >= 32).toBe(true);
    const cookies = (await context.cookies(origin)).filter(cookie => cookie.httpOnly);
    expect(cookies.some(cookie => cookie.path === '/' && cookie.sameSite === 'Lax')).toBe(true);
    expect((await page.evaluate(() => document.cookie)).includes(csrf)).toBe(false);

    // Provisioning is explicit operator setup, not authority claimed by an ID token.
    provisionTenant(tenant, principal);
    provisioned = true;
    await page.reload();
    await expect(page.getByRole('combobox', { name: 'Workspace', exact: true })).toBeVisible();
    const current = await (await context.request.get('/session')).json();
    expect(Array.isArray(current.tenant_ids) && current.tenant_ids.includes(tenant)).toBe(true);

    const headers = { Origin: origin, 'X-CSRF-Token': csrf, 'Content-Type': 'application/json', 'Idempotency-Key': randomUUID() };
    const beginUrl = `/v1/tenants/${tenant}/commands/artifact.begin`;
    const bytes = Buffer.from('Masonwing verified browser upload\n');
    const digest = `sha256:${createHash('sha256').update(bytes).digest('hex')}`;
    const body = { classification: 'CONFIDENTIAL', content_type: 'text/plain', size_bytes: bytes.length, expected_digest: digest };

    expect((await context.request.post(beginUrl, { headers: { ...headers, Origin: 'https://foreign.example.test' }, data: body })).status()).toBe(403);
    const withoutCsrf = { Origin: origin, 'Content-Type': 'application/json', 'Idempotency-Key': randomUUID() };
    expect((await context.request.post(beginUrl, { headers: withoutCsrf, data: body })).status()).toBe(403);
    const wrongTenant = await context.request.post(`/v1/tenants/${randomUUID()}/commands/artifact.begin`, { headers, data: '{invalid-json' });
    expect(wrongTenant.status()).toBe(404); // Admission occurs before JSON decoding.
    const stepUp = await context.request.post(`/v1/tenants/${tenant}/commands/plugin.install`, { headers, data: '{invalid-json' });
    expect(stepUp.status()).toBe(403);
    expect((await stepUp.json()).code).toBe('STEP_UP_REQUIRED');

    const begun = await context.request.post(beginUrl, { headers, data: body });
    expect(begun.status()).toBe(200);
    const receipt = await begun.json();
    expect(receipt.state).toBe('SUCCEEDED');
    const replay = await context.request.post(beginUrl, { headers, data: body });
    expect(replay.status()).toBe(200);
    expect(await replay.json()).toEqual(receipt);
    const uploadId = uuid(receipt.resource?.resource_id);
    const projectionResponse = await context.request.get(`/v1/tenants/${tenant}/resources/UploadSession/${uploadId}`);
    expect(projectionResponse.status()).toBe(200);
    const projection = await projectionResponse.json();
    expect(projection.artifact_ref.classification).toBe('CONFIDENTIAL');
    const metadataResponse = await context.request.get(`/v1/tenants/${tenant}/artifacts/${uuid(projection.artifact_ref.artifact_id)}/content`);
    expect(metadataResponse.status()).toBe(200);
    const upload = await metadataResponse.json();
    const artifact = uuid(upload.artifact_id);
    expect(upload.state).toBe('OPEN');
    expect(upload.upload_path).toBe(`/v1/tenants/${tenant}/uploads/${uploadId}`);
    expect((await context.request.get(`/v1/tenants/${tenant}/artifacts/${artifact}/content`)).status()).toBe(404);

    const uploaded = await context.request.put(upload.upload_path, { headers: { Origin: origin, 'X-CSRF-Token': csrf, 'Content-Type': 'text/plain' }, data: bytes });
    expect(uploaded.status()).toBe(204);
    const finalizeHeaders = { ...headers, 'Idempotency-Key': randomUUID() };
    const finalized = await context.request.post(`/v1/tenants/${tenant}/commands/artifact.finalize`, {
      headers: finalizeHeaders, data: { artifact_id: artifact, observed_digest: digest },
    });
    expect(finalized.status()).toBe(200);
    expect((await finalized.json()).state).toBe('SUCCEEDED');
    const content = await context.request.get(`/v1/tenants/${tenant}/artifacts/${artifact}/content`);
    expect(content.status()).toBe(200);
    expect(await content.body()).toEqual(bytes);
    expect((await context.request.put(upload.upload_path, { headers: { Origin: origin, 'X-CSRF-Token': csrf, 'Content-Type': 'text/plain' }, data: bytes })).status()).toBe(409);

    report = { kind: 'BROWSER_HTTP_INTEGRATION', result: 'PASS', observed_at: new Date().toISOString(),
      authentication: 'real Authorization Code + PKCE against local Keycloak', tenant_id: tenant,
      denied: ['wrong-origin', 'missing-csrf', 'foreign-tenant-before-body', 'step-up-before-body', 'pending-artifact-read', 'immutable-upload-rewrite'],
      upload: { bytes: bytes.length, digest, classification: 'CONFIDENTIAL', scanner: 'real local ClamAV', storage: 'real local MinIO', receipt_replay_equal: true },
      external_provider_mutations: 0, product_acceptance: 'NOT_ACCEPTED',
      cleanup: 'own ephemeral tenant records removed; unreachable immutable object versions await GC' };
    expect((await context.request.post('/auth/logout', { headers: { Origin: origin, 'X-CSRF-Token': csrf } })).status()).toBe(204);
    csrf = '';
    expect((await context.request.get('/session')).status()).toBe(401);
    await page.goto('/');
    await expect(page.getByRole('link', { name: 'Đăng nhập', exact: true })).toBeVisible();
    await page.screenshot({ path: testInfo.outputPath('runtime-logged-out.png'), fullPage: true });
  } finally {
    if (csrf) await context.request.post('/auth/logout', { headers: { Origin: origin, 'X-CSRF-Token': csrf } }).catch(() => {});
    if (provisioned) removeTenant(tenant);
  }
  mkdirSync('.evidence/plugin-runtime', { recursive: true });
  writeFileSync('.evidence/plugin-runtime/browser-flow.json', JSON.stringify(report, null, 2));
});
