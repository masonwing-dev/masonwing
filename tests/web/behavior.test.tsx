import { describe, expect, it, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { QueryClient } from '@tanstack/react-query';
import { ContributionRegistry, recoveryFor, tenantQueryKey } from '../../packages/plugin-sdk/src/index';
import { DataSurface } from '../../packages/ui/src/index';
import { getDevStatus, submitCommand, ApiFailure } from '../../packages/api-client/src/index';
import { clearPreviousTenant } from '../../packages/web-shell/src/tenant-scope';

const contribution = { id: 'fixture:one', plugin: 'fixture', route: '/workspace/:tenant/one', title: 'One', feature: 'F-001', commands: ['fixture.read'] };

describe('UI contribution boundary · MASONWING AC-125/126', () => {
  it('rejects a route collision even when parameter names differ, preserving the first contribution', () => {
    const registry = new ContributionRegistry();
    registry.register(contribution);
    expect(() => registry.register({ ...contribution, id: 'fixture:two', route: '/workspace/:other/one/' })).toThrow('UI_CONTRIBUTION_CONFLICT');
    expect(registry.list()).toEqual([contribution]);
  });
  it('rejects an unowned contribution namespace', () => {
    expect(() => new ContributionRegistry().register({ ...contribution, id: 'another:one' })).toThrow('UI_CONTRIBUTION_SCOPE_DENIED');
  });
  it('takes an immutable copy of the command list', () => {
    const commands = ['fixture.read'];
    const registry = new ContributionRegistry();
    registry.register({ ...contribution, commands });
    commands.push('fixture.publish');
    expect(registry.list()[0].commands).toEqual(['fixture.read']);
  });
});

describe('Tenant cache scope · MASONWING AC-128', () => {
  it('distinguishes both tenant identity and membership epoch', () => {
    expect(tenantQueryKey('A', 1, 'drafts')).not.toEqual(tenantQueryKey('B', 1, 'drafts'));
    expect(tenantQueryKey('A', 1, 'drafts')).not.toEqual(tenantQueryKey('A', 2, 'drafts'));
    expect(() => tenantQueryKey('', 1, 'drafts')).toThrow('TENANT_CONTEXT_REQUIRED');
  });
  it('cancels old reads and removes tenant data before activating a new scope', async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const key = tenantQueryKey('A', 1, 'drafts');
    let complete: (value: string) => void = () => {};
    let signal: AbortSignal | undefined;
    client.setQueryData(['local-dev-status'], 'keep');
    const previous = client.fetchQuery({ queryKey: key, queryFn: (context) => {
      signal = context.signal;
      return new Promise<string>(resolve => { complete = resolve; });
    } }).catch(() => undefined);
    await clearPreviousTenant(client, 'B', 1);
    expect(signal?.aborted).toBe(true);
    complete('tenant A private content');
    await previous;
    expect(client.getQueryData(key)).toBeUndefined();
    expect(client.getQueryData(['local-dev-status'])).toBe('keep');
    client.clear();
  });
});

describe('Observable failure states', () => {
  it('shows a failed read as an error, never as empty data', () => {
    const retry = vi.fn();
    render(<DataSurface state="error" retry={retry} />);
    expect(screen.getByRole('alert')).toHaveTextContent('Không tải được dữ liệu');
    expect(screen.queryByText('Chưa có dữ liệu')).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Tải lại dữ liệu' }));
    expect(retry).toHaveBeenCalledOnce();
  });
  it('does not expose blind retry for an unknown external outcome', () => {
    render(<DataSurface state="outcome-unknown" retry={vi.fn()} />);
    expect(screen.getByRole('status')).toHaveTextContent('Kết quả chưa xác định');
    expect(screen.queryByRole('button')).not.toBeInTheDocument();
    expect(recoveryFor('outcome-unknown')).toEqual({ label: 'Đối soát kết quả', mayRetryMutation: false });
  });
  it('labels unimplemented functionality explicitly', () => {
    render(<DataSurface state="not-implemented" />);
    expect(screen.getByRole('status')).toHaveTextContent('Handler nghiệp vụ chưa được triển khai');
  });
});

describe('Same-origin BFF transport', () => {
  it('refuses an external command destination before any request', async () => {
    const fetch = vi.fn(); vi.stubGlobal('fetch', fetch);
    await expect(submitCommand('https://other.invalid/v1/tenants/A/commands/x', {}, { csrf: 'c', idempotencyKey: 'k' })).rejects.toThrow('COMMAND_TARGET_DENIED');
    expect(fetch).not.toHaveBeenCalled();
  });
  it('does not automatically resend a command after a retryable server error', async () => {
    const fetch = vi.fn().mockResolvedValue({ ok: false, status: 503, json: async () => ({
      code: 'OUTCOME_UNKNOWN', message: 'Unknown', correlation_id: 'c', effect_state: 'UNKNOWN',
      retryable: true, recovery_action: 'RECONCILE', details: [],
    }) });
    vi.stubGlobal('fetch', fetch);
    await expect(submitCommand('/v1/tenants/A/commands/effect.dispatch', {}, { csrf: 'csrf', idempotencyKey: 'key' })).rejects.toBeInstanceOf(ApiFailure);
    expect(fetch).toHaveBeenCalledOnce();
    expect(fetch.mock.calls[0][1].credentials).toBe('same-origin');
    expect(fetch.mock.calls[0][1].headers.Authorization).toBeUndefined();
  });
  it('rejects a malformed dev status instead of displaying false readiness', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: async () => ({ environment: 'LOCAL', scaffold: true, readiness: { writes: true } }) }));
    await expect(getDevStatus()).rejects.toThrow('DEV_STATUS_CONTRACT_INVALID');
  });
});
