import type { QueryClient } from '@tanstack/react-query';
import { tenantQueryKey } from '@masonwing/plugin-sdk';

/** Call on an authenticated tenant/epoch change before displaying the new workspace. */
export async function clearPreviousTenant(client: QueryClient, tenant: string, epoch: number) {
  tenantQueryKey(tenant, epoch, 'scope');
  await client.cancelQueries({ queryKey: ['tenant'] });
  client.removeQueries({ queryKey: ['tenant'] });
  return { tenant, epoch };
}
