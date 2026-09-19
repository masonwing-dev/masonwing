import { afterEach, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { MemoryRouter } from 'react-router-dom';
import { ProductScreen } from '../../packages/web-shell/src/ProductScreen';

function json(body: unknown, status = 200) {
  return { ok: status >= 200 && status < 300, status, json: async () => body, text: async () => JSON.stringify(body) };
}

function renderBudget(fetch: ReturnType<typeof vi.fn>) {
  vi.stubGlobal('fetch', fetch);
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  render(<QueryClientProvider client={client}><MemoryRouter initialEntries={['/workspace/tenant-a/budget']}><ProductScreen
    screen={{ qualified_id: 'MASONWING@1.0.1:SCREEN-BUDGET', feature_id: 'F-014', title: 'Budget', actions: ['budget.configure'] }}
    tenantId="tenant-a" principalId="USER:issuer:user-1" epoch={2} csrf="csrf-1" />
  </MemoryRouter></QueryClientProvider>);
  return client;
}

function fillBudget() {
  fireEvent.change(screen.getByLabelText(/Currency/), { target: { value: 'USD' } });
  fireEvent.change(screen.getByLabelText(/Limit \(microunits\)/), { target: { value: '250000' } });
  fireEvent.change(screen.getByLabelText(/Chu kỳ/), { target: { value: 'DAY' } });
  fireEvent.change(screen.getByLabelText(/Expected version/), { target: { value: '4' } });
}

afterEach(() => vi.unstubAllGlobals());

it('reuses the idempotency key only for an explicit retry of the same payload', async () => {
  let writes = 0;
  const fetch = vi.fn(async (input: RequestInfo | URL, _init?: RequestInit) => {
    const path = String(input);
    if (path.includes('/resources/CostReservation')) return json({ items: [], next_cursor: null, snapshot_at: '2026-09-16T10:00:00Z', data_state: 'NO_DATA', total_visible: 0 });
    if (path.endsWith('/commands/budget.configure')) {
      writes += 1;
      if (writes === 1) return json({ code: 'TEMPORARY_UNAVAILABLE', message: 'Not sent', correlation_id: 'c1', effect_state: 'NOT_SENT', retryable: true, recovery_action: 'NONE', details: [] }, 503);
      return json({ command_id: 'cmd-1', state: 'ACCEPTED', resource: null, run_id: null, effect_id: null, correlation_id: 'c2', accepted_at: '2026-09-16T10:01:00Z' }, 202);
    }
    throw new Error(`Unexpected fetch ${path}`);
  });
  const client = renderBudget(fetch);
  fillBudget();
  fireEvent.click(screen.getByRole('button', { name: 'Gửi thao tác' }));
  const retry = await screen.findByRole('button', { name: 'Thử lại cùng request' });
  fireEvent.click(retry);
  expect(await screen.findByText('Command đã được nhận')).toBeInTheDocument();
  const writesCalls = fetch.mock.calls.filter(([input]) => String(input).endsWith('/commands/budget.configure'));
  expect(writesCalls).toHaveLength(2);
  const firstHeaders = writesCalls[0][1]?.headers as Record<string, string>;
  const secondHeaders = writesCalls[1][1]?.headers as Record<string, string>;
  expect(firstHeaders['Idempotency-Key']).toBeTruthy();
  expect(secondHeaders['Idempotency-Key']).toBe(firstHeaders['Idempotency-Key']);
  client.clear();
});

it('changes the idempotency key when the payload changes after a failed attempt', async () => {
  const fetch = vi.fn(async (input: RequestInfo | URL, _init?: RequestInit) => {
    const path = String(input);
    if (path.includes('/resources/CostReservation')) return json({ items: [], next_cursor: null, snapshot_at: '2026-09-16T10:00:00Z', data_state: 'NO_DATA', total_visible: 0 });
    if (path.endsWith('/commands/budget.configure')) return json({ code: 'INVALID', message: 'Fix payload', correlation_id: 'c', effect_state: 'NOT_SENT', retryable: false, recovery_action: 'NONE', details: [] }, 422);
    throw new Error(`Unexpected fetch ${path}`);
  });
  const client = renderBudget(fetch);
  fillBudget();
  fireEvent.click(screen.getByRole('button', { name: 'Gửi thao tác' }));
  await screen.findByText('Command không hoàn tất');
  fireEvent.change(screen.getByLabelText(/Limit \(microunits\)/), { target: { value: '300000' } });
  fireEvent.click(screen.getByRole('button', { name: 'Gửi thao tác' }));
  await waitFor(() => expect(fetch.mock.calls.filter(([input]) => String(input).endsWith('/commands/budget.configure'))).toHaveLength(2));
  const calls = fetch.mock.calls.filter(([input]) => String(input).endsWith('/commands/budget.configure'));
  expect((calls[0][1]?.headers as Record<string, string>)['Idempotency-Key'])
    .not.toBe((calls[1][1]?.headers as Record<string, string>)['Idempotency-Key']);
  client.clear();
});

it('turns a 409 into review flow without automatically replaying the command', async () => {
  const fetch = vi.fn(async (input: RequestInfo | URL, _init?: RequestInit) => {
    const path = String(input);
    if (path.includes('/resources/CostReservation')) return json({ items: [], next_cursor: null, snapshot_at: '2026-09-16T10:00:00Z', data_state: 'NO_DATA', total_visible: 0 });
    if (path.endsWith('/commands/budget.configure')) return json({ code: 'VERSION_CONFLICT', message: 'Budget changed', correlation_id: 'c', effect_state: 'NOT_SENT', retryable: false, recovery_action: 'REVIEW_CONFLICT', details: [] }, 409);
    throw new Error(`Unexpected fetch ${path}`);
  });
  const client = renderBudget(fetch);
  fillBudget();
  fireEvent.click(screen.getByRole('button', { name: 'Gửi thao tác' }));
  const review = await screen.findByRole('button', { name: 'Tải trạng thái mới để review' });
  expect(screen.getByText('Có conflict cần review')).toBeInTheDocument();
  fireEvent.click(review);
  await waitFor(() => expect(fetch.mock.calls.filter(([input]) => String(input).endsWith('/commands/budget.configure'))).toHaveLength(1));
  client.clear();
});

it('does not offer blind retry when the effect outcome is unknown', async () => {
  const fetch = vi.fn(async (input: RequestInfo | URL, _init?: RequestInit) => {
    const path = String(input);
    if (path.includes('/resources/CostReservation')) return json({ items: [], next_cursor: null, snapshot_at: '2026-09-16T10:00:00Z', data_state: 'NO_DATA', total_visible: 0 });
    if (path.endsWith('/commands/budget.configure')) return json({ code: 'OUTCOME_UNKNOWN', message: 'Provider result unknown', correlation_id: 'c', effect_state: 'UNKNOWN', retryable: true, recovery_action: 'RECONCILE', details: [] }, 503);
    throw new Error(`Unexpected fetch ${path}`);
  });
  const client = renderBudget(fetch);
  fillBudget();
  fireEvent.click(screen.getByRole('button', { name: 'Gửi thao tác' }));
  expect(await screen.findByText('Kết quả chưa xác định')).toBeInTheDocument();
  expect(screen.queryByRole('button', { name: 'Thử lại cùng request' })).not.toBeInTheDocument();
  client.clear();
});
