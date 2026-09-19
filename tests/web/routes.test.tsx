import { afterEach, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { App } from '../../packages/web-shell/src/App';
import screens from '../../packages/web-shell/src/generated/screens.json';

const session = {
  authenticated: true,
  principal: { type: 'USER', id: 'user-1', issuer: 'https://issuer.local' },
  tenant_ids: ['tenant-a', 'tenant-b'],
  active_tenant_id: 'tenant-a',
  expires_at: '2026-09-17T00:00:00Z',
  step_up_expires_at: null,
  csrf_token: 'csrf-1',
};

function json(body: unknown, status = 200) {
  return { ok: status >= 200 && status < 300, status, json: async () => body, text: async () => JSON.stringify(body) };
}

function emptyFetch() {
  return vi.fn(async (input: RequestInfo | URL) => {
    const path = String(input);
    if (path === '/session') return json(session);
    if (path === '/dev/status') return json({
      service: 'masonwing-host-api', environment: 'LOCAL', external_mutations_enabled: false, live_budget_microunits: 0,
      readiness: { ready: false, status: 'QUALIFICATION_INCOMPLETE' }, operation_catalog_count: 43,
      data_store: 'POSTGRESQL', artifact_store: 'S3', identity: 'OIDC', authorization: 'CEDAR',
    });
    if (path.includes('/resources/') || path.includes('/views/')) return json({ items: [], next_cursor: null, snapshot_at: '2026-09-16T10:00:00Z', data_state: 'NO_DATA', total_visible: 0 });
    throw new Error(`Unexpected fetch ${path}`);
  });
}

function renderApp(path: string) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  const view = render(<QueryClientProvider client={client}><MemoryRouter initialEntries={[path]}><App /></MemoryRouter></QueryClientProvider>);
  return { client, ...view };
}

afterEach(() => vi.unstubAllGlobals());

it.each(screens)('$qualified_id is reachable as an authenticated functional screen', async definition => {
  vi.stubGlobal('fetch', emptyFetch());
  const path = definition.route.replace(':tenantId', 'tenant-a').replace(':brandId', 'default');
  const { client, unmount } = renderApp(path);
  expect(screen.getAllByRole('heading', { level: 1 })).toHaveLength(1);
  expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent(definition.title);
  await screen.findByRole('combobox', { name: 'Workspace' });
  expect(screen.queryByText('HANDLER CHƯA TRIỂN KHAI')).not.toBeInTheDocument();
  for (const operation of definition.actions) {
    expect(screen.getByText(operation, { selector: 'code' })).toBeInTheDocument();
  }
  unmount();
  client.clear();
});

it('preserves a protected deep link in the login return_to', async () => {
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue(json({ ...session, authenticated: false, principal: null, tenant_ids: [], active_tenant_id: null, csrf_token: null })));
  const { client } = renderApp('/workspace/tenant-a/effects?resource=effect-1');
  const login = await screen.findByRole('link', { name: 'Đăng nhập' });
  expect(login).toHaveAttribute('href', `/auth/login?return_to=${encodeURIComponent('/workspace/tenant-a/effects?resource=effect-1')}`);
  client.clear();
});

it('does not issue tenant resource reads when the URL tenant is outside the session', async () => {
  const fetch = emptyFetch();
  vi.stubGlobal('fetch', fetch);
  const { client } = renderApp('/workspace/not-mine/workflow');
  expect(await screen.findByText('Không có quyền truy cập')).toBeInTheDocument();
  expect(fetch.mock.calls.some(([input]) => String(input).includes('/v1/tenants/not-mine/'))).toBe(false);
  client.clear();
});

it('reads resource detail through projection artifact content instead of assuming inline JSON', async () => {
  const fetch = vi.fn(async (input: RequestInfo | URL) => {
    const path = String(input);
    if (path === '/session') return json(session);
    if (path === '/v1/tenants/tenant-a/resources/Run?limit=50') return json({
      items: [{ resource_type: 'Run', resource_id: 'run-7', version: 3 }], next_cursor: null,
      snapshot_at: '2026-09-16T10:00:00Z', data_state: 'FRESH', total_visible: 1,
    });
    if (path === '/v1/tenants/tenant-a/resources/Run/run-7') return json({
      resource: { resource_type: 'Run', resource_id: 'run-7', version: 3 },
      artifact_ref: { artifact_id: 'artifact-run-7', tenant_id: 'tenant-a', digest: 'sha256:run7', schema_version: '1', classification: 'INTERNAL' },
      updated_at: '2026-09-16T10:00:00Z', data_state: 'FRESH',
    });
    if (path === '/v1/tenants/tenant-a/artifacts/artifact-run-7/content') return json({
      run_id: 'run-7', tenant_id: 'tenant-a', workflow_id: 'wf-1', workflow_version: '2', state: 'WAITING_APPROVAL',
      plugin_digest: 'sha256:plugin', grant_id: 'grant-1', version: 3, created_at: '2026-09-16T09:00:00Z', updated_at: '2026-09-16T10:00:00Z',
    });
    throw new Error(`Unexpected fetch ${path}`);
  });
  vi.stubGlobal('fetch', fetch);
  const { client } = renderApp('/workspace/tenant-a/workflow');
  const list = await screen.findByRole('list', { name: 'Runs' });
  fireEvent.click(within(list).getByRole('button', { name: /run-7/ }));
  expect(await screen.findByText('WAITING_APPROVAL')).toBeInTheDocument();
  expect(fetch).toHaveBeenCalledWith('/v1/tenants/tenant-a/resources/Run/run-7', expect.anything());
  expect(fetch).toHaveBeenCalledWith('/v1/tenants/tenant-a/artifacts/artifact-run-7/content', expect.anything());
  client.clear();
});

it('keeps an unavailable API distinguishable from an empty collection', async () => {
  const fetch = vi.fn(async (input: RequestInfo | URL) => {
    const path = String(input);
    if (path === '/session') return json(session);
    if (path.includes('/resources/EffectIntent')) return json({
      code: 'OPERATION_NOT_IMPLEMENTED', message: 'Projection adapter unavailable', correlation_id: 'corr-1',
      effect_state: 'NOT_APPLICABLE', retryable: false, recovery_action: 'CONTACT_OPERATOR', details: [],
    }, 501);
    throw new Error(`Unexpected fetch ${path}`);
  });
  vi.stubGlobal('fetch', fetch);
  const { client } = renderApp('/workspace/tenant-a/effects');
  expect(await screen.findByText('Effects chưa được backend hỗ trợ')).toBeInTheDocument();
  expect(screen.queryByText('Chưa có dữ liệu')).not.toBeInTheDocument();
  client.clear();
});

it('switches tenant with replace navigation after clearing the old tenant cache', async () => {
  vi.stubGlobal('fetch', emptyFetch());
  const { client } = renderApp('/workspace/tenant-a/sessions');
  client.setQueryData(['tenant', 'tenant-a', 'principal', 'user-1', 'epoch', 0, 'sentinel'], 'private-a');
  const switcher = await screen.findByRole('combobox', { name: 'Workspace' });
  fireEvent.change(switcher, { target: { value: 'tenant-b' } });
  await waitFor(() => expect(client.getQueryData(['tenant', 'tenant-a', 'principal', 'user-1', 'epoch', 0, 'sentinel'])).toBeUndefined());
  expect(await screen.findByRole('combobox', { name: 'Workspace' })).toHaveValue('tenant-b');
  client.clear();
});
