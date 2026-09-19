import type { QueryClient } from '@tanstack/react-query';
import { tenantQueryKey } from '@masonwing/plugin-sdk';

export function tenantPrincipalQueryKey(
  tenant: string,
  principal: string,
  epoch: number,
  resource: string,
  filters: object = {},
) {
  if (!principal) throw new Error('PRINCIPAL_CONTEXT_REQUIRED');
  const base = tenantQueryKey(tenant, epoch, resource, filters);
  return ['tenant', tenant, 'principal', principal, ...base.slice(2)] as const;
}

/** Call on an authenticated tenant/epoch change before displaying the new workspace. */
export async function clearPreviousTenant(client: QueryClient, tenant: string, epoch: number) {
  tenantQueryKey(tenant, epoch, 'scope');
  await client.cancelQueries({ queryKey: ['tenant'] });
  client.removeQueries({ queryKey: ['tenant'] });
  return { tenant, epoch };
}

export async function clearAuthenticatedScope(client: QueryClient) {
  await client.cancelQueries({ queryKey: ['tenant'] });
  client.removeQueries({ queryKey: ['tenant'] });
}
