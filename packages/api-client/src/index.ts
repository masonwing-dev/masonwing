import type {
  ArtifactRef,
  CommandReceipt,
  Connection,
  ConnectorAuthorization,
  CostReservation,
  Delegation,
  Error as ContractError,
  EffectIntent,
  EventEnvelope,
  Membership,
  PluginManifest,
  ReadCollection,
  ResourceProjection,
  ResourceRef,
  Run,
  SessionView,
  UploadSession,
} from './generated/masonwing';

export type BaselineResourceType =
  | 'Run'
  | 'Membership'
  | 'Delegation'
  | 'EffectIntent'
  | 'Connection'
  | 'CostReservation'
  | 'PluginManifest'
  | 'UploadSession'
  | 'ConnectorAuthorization';

const baselineResourceTypes = new Set<BaselineResourceType>([
  'Run', 'Membership', 'Delegation', 'EffectIntent', 'Connection', 'CostReservation',
  'PluginManifest', 'UploadSession', 'ConnectorAuthorization',
]);

/** Host extensions use their own route; baseline contract enums stay closed. */
export const runtimeViewTypes = [
  'ProductComposition', 'PluginInstallation', 'PluginInvocation', 'Approval', 'BudgetSettings', 'KillSwitch',
  'ProviderProfile', 'Notification', 'SupportGrant', 'CatalogListing', 'Export', 'ReleaseQualification',
  'ConformanceRun', 'PolicyProposal', 'Schedule', 'DeletionRequest', 'PolicyDecision', 'MembershipInvitation',
  'IdentityConfigurationProposal',
] as const;
export type RuntimeViewType = typeof runtimeViewTypes[number];
export type ResourceType = BaselineResourceType | RuntimeViewType;
const viewTypes = new Set<string>(runtimeViewTypes);

function resourceRoute(type: ResourceType): 'resources' | 'views' {
  if (baselineResourceTypes.has(type as BaselineResourceType)) return 'resources';
  if (viewTypes.has(type)) return 'views';
  throw new Error('RESOURCE_TYPE_INVALID');
}

export type {
  ArtifactRef, CommandReceipt, Connection, ConnectorAuthorization, CostReservation, Delegation, EffectIntent, EventEnvelope,
  Membership, PluginManifest, ReadCollection, ResourceProjection, ResourceRef, Run, SessionView, UploadSession,
};

export type DevStatus = {
  service: string; environment: 'LOCAL'; external_mutations_enabled: false; live_budget_microunits: 0;
  readiness: { ready: false; status: 'QUALIFICATION_INCOMPLETE' };
  operation_catalog_count: number; data_store: 'POSTGRESQL'; artifact_store: 'S3'; identity: 'OIDC'; authorization: 'CEDAR';
};
export type ApiErrorBody = {
  code: string; message: string; correlation_id: string; effect_state: string;
  retryable: boolean; recovery_action: string; details: { field: string; reason: string }[];
};
export class ApiFailure extends Error {
  constructor(readonly status: number, readonly body: ApiErrorBody | null, readonly requestPath?: string) {
    super(body?.message ?? `HTTP ${status}`);
  }
}

function isErrorBody(value: unknown): value is ApiErrorBody {
  if (!value || typeof value !== 'object') return false;
  const body = value as Partial<ContractError>;
  return typeof body.code === 'string' && typeof body.message === 'string'
    && typeof body.correlation_id === 'string' && typeof body.effect_state === 'string'
    && typeof body.retryable === 'boolean' && typeof body.recovery_action === 'string'
    && Array.isArray(body.details) && body.details.every(detail => Boolean(detail)
      && typeof detail === 'object' && typeof detail.field === 'string' && typeof detail.reason === 'string');
}

async function errorBody(response: Response): Promise<ApiErrorBody | null> {
  try {
    const body: unknown = await response.json();
    return isErrorBody(body) ? body : null;
  } catch {
    return null;
  }
}

async function getJson<T>(path: string, signal?: AbortSignal): Promise<T> {
  const response = await fetch(path, { signal, credentials: 'same-origin', cache: 'no-store' });
  if (!response.ok) throw new ApiFailure(response.status, await errorBody(response), path);
  return response.json() as Promise<T>;
}

export async function getSession(signal?: AbortSignal): Promise<SessionView> {
  return getJson<SessionView>('/session', signal);
}

export async function logoutSession(csrf: string | null, signal?: AbortSignal): Promise<void> {
  const response = await fetch('/auth/logout', {
    method: 'POST', credentials: 'same-origin', signal,
    headers: csrf ? { 'X-CSRF-Token': csrf } : {},
  });
  if (!response.ok) throw new ApiFailure(response.status, await errorBody(response), '/auth/logout');
}

const opaque = /^[A-Za-z0-9][A-Za-z0-9._:-]*$/;
function segment(value: string, label: string): string {
  if (!opaque.test(value)) throw new Error(`${label}_INVALID`);
  return encodeURIComponent(value);
}

export async function listResources(
  tenantId: string,
  resourceType: ResourceType,
  options: { cursor?: string | null; limit?: number; signal?: AbortSignal } = {},
): Promise<ReadCollection> {
  const route = resourceRoute(resourceType);
  const params = new URLSearchParams();
  if (options.cursor) params.set('cursor', options.cursor);
  if (options.limit !== undefined) params.set('limit', String(options.limit));
  const query = params.size ? `?${params.toString()}` : '';
  return getJson<ReadCollection>(
    `/v1/tenants/${segment(tenantId, 'TENANT')}/${route}/${segment(resourceType, 'RESOURCE_TYPE')}${query}`,
    options.signal,
  );
}

export async function getResourceProjection(
  tenantId: string,
  resourceType: ResourceType,
  resourceId: string,
  signal?: AbortSignal,
): Promise<ResourceProjection> {
  const route = resourceRoute(resourceType);
  return getJson<ResourceProjection>(
    `/v1/tenants/${segment(tenantId, 'TENANT')}/${route}/${segment(resourceType, 'RESOURCE_TYPE')}/${segment(resourceId, 'RESOURCE')}`,
    signal,
  );
}

export async function getArtifact(tenantId: string, artifactId: string, signal?: AbortSignal): Promise<ArtifactRef> {
  return getJson<ArtifactRef>(
    `/v1/tenants/${segment(tenantId, 'TENANT')}/artifacts/${segment(artifactId, 'ARTIFACT')}`,
    signal,
  );
}

export async function getArtifactContent<T>(tenantId: string, artifactId: string, signal?: AbortSignal): Promise<T> {
  const path = `/v1/tenants/${segment(tenantId, 'TENANT')}/artifacts/${segment(artifactId, 'ARTIFACT')}/content`;
  const response = await fetch(path, { signal, credentials: 'same-origin', cache: 'no-store' });
  if (!response.ok) throw new ApiFailure(response.status, await errorBody(response), path);
  const text = await response.text();
  try {
    return JSON.parse(text) as T;
  } catch {
    throw new Error('ARTIFACT_CONTENT_NOT_JSON');
  }
}

export async function getResource<T>(
  tenantId: string,
  resourceType: ResourceType,
  resourceId: string,
  signal?: AbortSignal,
): Promise<{ projection: ResourceProjection; value: T }> {
  const projection = await getResourceProjection(tenantId, resourceType, resourceId, signal);
  if (projection.resource.resource_type !== resourceType || projection.resource.resource_id !== resourceId
      || projection.artifact_ref.tenant_id !== tenantId) {
    throw new Error('RESOURCE_PROJECTION_MISMATCH');
  }
  const value = await getArtifactContent<T>(tenantId, projection.artifact_ref.artifact_id, signal);
  return { projection, value };
}

export async function getRun(tenantId: string, runId: string, signal?: AbortSignal): Promise<Run> {
  return getJson<Run>(
    `/v1/tenants/${segment(tenantId, 'TENANT')}/runs/${segment(runId, 'RUN')}`,
    signal,
  );
}

export function eventStreamPath(tenantId: string): string {
  return `/v1/tenants/${segment(tenantId, 'TENANT')}/events`;
}

/** Same-origin BFF transport. Tokens are never accepted or persisted by the browser SDK. */
export async function getDevStatus(signal?: AbortSignal): Promise<DevStatus> {
  const response = await fetch('/dev/status', { signal, credentials: 'same-origin', cache: 'no-store' });
  if (!response.ok) throw new ApiFailure(response.status, null);
  const body = await response.json();
  if (body.environment !== 'LOCAL' || typeof body.service !== 'string'
    || body.external_mutations_enabled !== false || body.live_budget_microunits !== 0
    || body.readiness?.ready !== false || body.readiness?.status !== 'QUALIFICATION_INCOMPLETE'
    || !Number.isInteger(body.operation_catalog_count) || body.operation_catalog_count < 1
    || body.data_store !== 'POSTGRESQL' || body.artifact_store !== 'S3'
    || body.identity !== 'OIDC' || body.authorization !== 'CEDAR') {
    throw new Error('DEV_STATUS_CONTRACT_INVALID');
  }
  return body as DevStatus;
}

export async function submitCommand(path: string, input: unknown, options: {
  csrf: string; idempotencyKey: string; version?: number; signal?: AbortSignal;
}): Promise<CommandReceipt> {
  if (!/^\/v1\/tenants\/[A-Za-z0-9._:-]+\/commands\//.test(path)
      || path.includes('..') || path.includes('\\') || path.includes('?') || path.includes('#')) {
    throw new Error('COMMAND_TARGET_DENIED');
  }
  if (!options.csrf || !options.idempotencyKey) throw new Error('COMMAND_PROOF_REQUIRED');
  const response = await fetch(path, {
    method: 'POST', credentials: 'same-origin', signal: options.signal,
    headers: { 'Content-Type': 'application/json', 'X-CSRF-Token': options.csrf,
      'Idempotency-Key': options.idempotencyKey,
      ...(options.version === undefined ? {} : { 'If-Match': String(options.version) }) },
    body: JSON.stringify(input),
  });
  const body: unknown = await response.json();
  if (!response.ok) throw new ApiFailure(response.status, isErrorBody(body) ? body : null, path);
  // Return the receipt as-is. ACCEPTED does not imply successful business effects.
  return body as CommandReceipt;
}
