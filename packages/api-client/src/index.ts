export type DevStatus = {
  service: string; environment: 'LOCAL'; scaffold: true; implementation_status: string;
  external_mutation_enabled: false; live_budget_microunits: 0;
  readiness: { writes: false; critical_adapters_qualified: false }; contract_version: string;
};
export type ApiErrorBody = {
  code: string; message: string; correlation_id: string; effect_state: string;
  retryable: boolean; recovery_action: string; details: unknown[];
};
export class ApiFailure extends Error {
  constructor(readonly status: number, readonly body: ApiErrorBody | null) {
    super(body?.message ?? `HTTP ${status}`);
  }
}

/** Same-origin BFF transport. Tokens are never accepted or persisted by the browser SDK. */
export async function getDevStatus(signal?: AbortSignal): Promise<DevStatus> {
  const response = await fetch('/dev/status', { signal, credentials: 'same-origin', cache: 'no-store' });
  if (!response.ok) throw new ApiFailure(response.status, null);
  const body = await response.json();
  if (body.environment !== 'LOCAL' || body.scaffold !== true || typeof body.service !== 'string'
    || body.external_mutation_enabled !== false || body.live_budget_microunits !== 0
    || body.readiness?.writes !== false || body.readiness?.critical_adapters_qualified !== false) {
    throw new Error('DEV_STATUS_CONTRACT_INVALID');
  }
  return body as DevStatus;
}

export async function submitCommand(path: string, input: unknown, options: {
  csrf: string; idempotencyKey: string; version?: number; signal?: AbortSignal;
}): Promise<unknown> {
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
  const body = await response.json();
  if (!response.ok) throw new ApiFailure(response.status, body);
  // Return the receipt as-is. ACCEPTED does not imply successful business effects.
  return body;
}
