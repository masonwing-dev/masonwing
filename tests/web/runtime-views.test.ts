import { afterEach, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { getDevStatus, getResource, listResources, runtimeViewTypes, type ResourceType } from '../../packages/api-client/src/index';

const status = {
  service: 'masonwing-host-api', environment: 'LOCAL', readiness: { ready: false, status: 'QUALIFICATION_INCOMPLETE' },
  external_mutations_enabled: false, live_budget_microunits: 0, operation_catalog_count: 43,
  data_store: 'POSTGRESQL', artifact_store: 'S3', identity: 'OIDC', authorization: 'CEDAR',
};
const response = (body: unknown) => ({ ok: true, status: 200, json: async () => body, text: async () => JSON.stringify(body) });
afterEach(() => vi.unstubAllGlobals());

it('reads the implemented BFF status and refuses an unsafe readiness/configuration claim', async () => {
  const fetch = vi.fn().mockResolvedValue(response(status));
  vi.stubGlobal('fetch', fetch);
  expect(await getDevStatus()).toEqual(status);
  for (const invalid of [
    { ...status, external_mutations_enabled: true }, { ...status, live_budget_microunits: 1 },
    { ...status, readiness: { ready: true, status: 'READY' } }, { ...status, operation_catalog_count: -1 },
  ]) {
    fetch.mockResolvedValueOnce(response(invalid));
    await expect(getDevStatus()).rejects.toThrow('DEV_STATUS_CONTRACT_INVALID');
  }
});

it('keeps host extension read routes aligned without modifying the baseline enum', () => {
  const source = readFileSync('platform-plugins/data-postgres/src/queries.rs', 'utf8');
  const declaration = source.match(/pub const EXTENSION_RESOURCE_TYPES:[^=]+=\s*&\[([^;]+)\];/)?.[1];
  expect(declaration).toBeDefined();
  const backend = Array.from(declaration!.matchAll(/"([A-Za-z]+)"/g), match => match[1]);
  expect([...runtimeViewTypes].sort()).toEqual(backend.sort());
  expect(runtimeViewTypes).not.toContain('Run');
});

it('loads baseline resources and host views from their distinct routes', async () => {
  const fetch = vi.fn().mockResolvedValue(response({ items: [], next_cursor: null, data_state: 'NO_DATA' }));
  vi.stubGlobal('fetch', fetch);
  await listResources('tenant-a', 'Run', { limit: 50 });
  await listResources('tenant-a', 'PluginInstallation', { limit: 20, cursor: 'cursor-2' });
  expect(fetch.mock.calls.map(call => call[0])).toEqual([
    '/v1/tenants/tenant-a/resources/Run?limit=50', '/v1/tenants/tenant-a/views/PluginInstallation?cursor=cursor-2&limit=20',
  ]);
  await expect(listResources('tenant-a', 'UndeclaredView' as ResourceType)).rejects.toThrow('RESOURCE_TYPE_INVALID');
  expect(fetch).toHaveBeenCalledTimes(2);
});

it('resolves an invocation view through its tenant-bound artifact and never reads a foreign projection', async () => {
  const projection = { resource: { resource_type: 'PluginInvocation', resource_id: 'invocation-1', version: 1 },
    artifact_ref: { artifact_id: 'result-1', tenant_id: 'tenant-a', digest: `sha256:${'a'.repeat(64)}`, classification: 'CONFIDENTIAL', schema_version: '1.0.0' },
    data_state: 'FRESH', updated_at: '2026-09-16T10:00:00Z' };
  const value = { invocation_id: 'invocation-1', plugin_id: 'checksum', state: 'SUCCEEDED' };
  const fetch = vi.fn().mockResolvedValueOnce(response(projection)).mockResolvedValueOnce(response(value));
  vi.stubGlobal('fetch', fetch);
  expect(await getResource('tenant-a', 'PluginInvocation', 'invocation-1')).toEqual({ projection, value });
  expect(fetch.mock.calls.map(call => call[0])).toEqual([
    '/v1/tenants/tenant-a/views/PluginInvocation/invocation-1', '/v1/tenants/tenant-a/artifacts/result-1/content',
  ]);
  fetch.mockResolvedValueOnce(response({ ...projection, artifact_ref: { ...projection.artifact_ref, tenant_id: 'tenant-b' } }));
  await expect(getResource('tenant-a', 'PluginInvocation', 'invocation-1')).rejects.toThrow('RESOURCE_PROJECTION_MISMATCH');
  expect(fetch).toHaveBeenCalledTimes(3);
});
