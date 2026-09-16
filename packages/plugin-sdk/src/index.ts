export type SurfaceState = 'loading' | 'empty' | 'error' | 'offline' | 'stale' | 'forbidden' | 'success' | 'not-implemented' | 'outcome-unknown';
export type Contribution = Readonly<{
  id: string; plugin: string; route: string; title: string; feature: string;
  commands: readonly string[];
}>;

/** Host-owned route registry. A rejected contribution never replaces an existing one. */
export class ContributionRegistry {
  private readonly entries = new Map<string, Contribution>();
  private readonly routes = new Set<string>();
  register(contribution: Contribution): void {
    const route = contribution.route.replace(/\/+$/, '') || '/';
    // Parameter names do not create distinct router paths.
    const canonical = route.replace(/:[A-Za-z][A-Za-z0-9_]*/g, ':param');
    if (this.entries.has(contribution.id) || this.routes.has(canonical)) {
      throw new Error('UI_CONTRIBUTION_CONFLICT');
    }
    if (!contribution.id.startsWith(`${contribution.plugin}:`)
      || !/^\/(workspace|brands)\/[A-Za-z0-9._:-]+\/[A-Za-z0-9/_-]+$/.test(route)) {
      throw new Error('UI_CONTRIBUTION_SCOPE_DENIED');
    }
    this.entries.set(contribution.id, Object.freeze({ ...contribution, route,
      commands: Object.freeze([...contribution.commands]) }));
    this.routes.add(canonical);
  }
  list(): readonly Contribution[] { return [...this.entries.values()]; }
}

export function tenantQueryKey(tenant: string, epoch: number, resource: string, filters: object = {}) {
  if (!tenant || !Number.isSafeInteger(epoch) || epoch < 0) throw new Error('TENANT_CONTEXT_REQUIRED');
  return ['tenant', tenant, 'epoch', epoch, resource, filters] as const;
}

export function recoveryFor(state: SurfaceState): { label: string; mayRetryMutation: boolean } {
  if (state === 'outcome-unknown') return { label: 'Đối soát kết quả', mayRetryMutation: false };
  if (state === 'not-implemented') return { label: 'Xem yêu cầu triển khai', mayRetryMutation: false };
  if (state === 'forbidden') return { label: 'Kiểm tra quyền truy cập', mayRetryMutation: false };
  return { label: state === 'offline' ? 'Kiểm tra kết nối' : 'Tải lại dữ liệu', mayRetryMutation: false };
}
