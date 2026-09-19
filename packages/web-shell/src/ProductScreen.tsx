import { useMemo, useRef, useState, type FormEvent, type ReactNode } from 'react';
import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useSearchParams } from 'react-router-dom';
import { AlertTriangle, ArrowRight, CheckCircle2, ChevronRight, CircleDot, RefreshCw, ShieldAlert } from 'lucide-react';
import {
  ApiFailure,
  getResource,
  listResources,
  submitCommand,
  type CommandReceipt,
  type Connection,
  type ConnectorAuthorization,
  type CostReservation,
  type Delegation,
  type EffectIntent,
  type Membership,
  type PluginManifest,
  type ResourceType,
  type Run,
  type UploadSession,
} from '@masonwing/api-client';
import { Badge, Button, Card, DataSurface } from '@masonwing/ui';
import { advancedOperationNotes, commandForms, featureConfig, resourceLabels, type CommandFormSpec, type FieldSpec } from './product-config';
import { tenantPrincipalQueryKey } from './tenant-scope';

type ScreenDefinition = Readonly<{
  qualified_id: string;
  feature_id: string;
  title: string;
  actions: readonly string[];
}>;

type KnownResource = Run | Membership | Delegation | EffectIntent | Connection | CostReservation | PluginManifest | UploadSession | ConnectorAuthorization | Record<string, unknown>;

type ProductScreenProps = {
  screen: ScreenDefinition;
  tenantId: string;
  principalId: string;
  epoch: number;
  csrf: string;
};

function unsupported(error: unknown) {
  if (!(error instanceof ApiFailure)) return false;
  const code = error.body?.code.toUpperCase() ?? '';
  return error.status === 501 || code.includes('NOT_IMPLEMENTED') || code.includes('UNSUPPORTED') || code === 'OPERATION_NOT_FOUND';
}

function failureState(error: unknown): 'error' | 'offline' | 'forbidden' | 'outcome-unknown' {
  if (error instanceof ApiFailure) {
    if (error.status === 403) return 'forbidden';
    if (error.body?.effect_state === 'UNKNOWN' || error.body?.recovery_action === 'RECONCILE') return 'outcome-unknown';
    return 'error';
  }
  return error instanceof TypeError ? 'offline' : 'error';
}

export function ProductScreen({ screen, tenantId, principalId, epoch, csrf }: ProductScreenProps) {
  const config = featureConfig[screen.feature_id] ?? { description: 'Mô-đun Masonwing.', resources: [] };
  const guided = screen.actions.map(operation => commandForms[operation]).filter(Boolean);
  const advanced = screen.actions.filter(operation => !commandForms[operation]);
  const queryClient = useQueryClient();

  return <div className="product-screen">
    <Card className="module-summary">
      <div className="section-row">
        <div><Badge>{screen.feature_id}</Badge><p>{config.description}</p></div>
        <Badge tone="positive">LIVE BFF CONTRACT</Badge>
      </div>
      {config.readNote ? <p className="contract-note">{config.readNote}</p> : null}
    </Card>

    {config.resources.length ? <section className="workspace-section" aria-labelledby="resource-heading">
      <div className="section-heading"><div><p className="eyebrow">Dữ liệu hiện hành</p><h2 id="resource-heading">Resource browser</h2></div>
        <p>Danh sách và detail đọc từ tenant-scoped BFF projection. Không suy diễn dữ liệu chưa trả về.</p></div>
      <div className="resource-columns">
        {config.resources.map(resourceType => <ResourceBrowser key={resourceType} tenantId={tenantId} principalId={principalId} epoch={epoch} resourceType={resourceType} />)}
      </div>
    </section> : <Card className="read-model-note">
      <CircleDot size={20} aria-hidden="true" />
      <div><strong>Không có generic read collection cho feature này</strong><p>{config.readNote ?? 'Action vẫn đi qua command contract của BFF.'}</p></div>
    </Card>}

    <section className="workspace-section" aria-labelledby="actions-heading">
      <div className="section-heading"><div><p className="eyebrow">Thao tác</p><h2 id="actions-heading">Guided actions</h2></div>
        <p>Form chỉ thu các field typed mà command contract yêu cầu. Mỗi submit có idempotency key riêng.</p></div>
      {guided.length ? <div className="action-grid">
        {guided.map(spec => <CommandForm key={spec.operation} spec={spec} tenantId={tenantId} csrf={csrf}
          onChanged={() => void queryClient.invalidateQueries({ queryKey: ['tenant', tenantId, 'principal', principalId] })} />)}
      </div> : <Card><DataSurface state="not-implemented" /></Card>}
    </section>

    {advanced.length ? <section className="workspace-section" aria-labelledby="catalog-heading">
      <div className="section-heading"><div><p className="eyebrow">Advanced</p><h2 id="catalog-heading">Contract catalog</h2></div>
        <p>Các command cần context hoặc object phức hợp được khởi tạo từ entity/SDK tương ứng, không có ô nhập JSON tự do.</p></div>
      <Card className="advanced-catalog">{advanced.map(operation => <div className="catalog-row" key={operation}>
        <div><code>{operation}</code><p>{advancedOperationNotes[operation] ?? 'Command contract có trong OpenAPI nhưng chưa có guided form an toàn cho surface này.'}</p></div>
        <Badge>CONTRACT ONLY</Badge>
      </div>)}</Card>
    </section> : null}
  </div>;
}

function ResourceBrowser({ tenantId, principalId, epoch, resourceType }: {
  tenantId: string; principalId: string; epoch: number; resourceType: ResourceType;
}) {
  const [params, setParams] = useSearchParams();
  const selected = params.get('type') === resourceType ? params.get('resource') : null;
  const [filter, setFilter] = useState('');
  const list = useInfiniteQuery({
    queryKey: tenantPrincipalQueryKey(tenantId, principalId, epoch, `resources:${resourceType}`, { limit: 50 }),
    queryFn: ({ signal, pageParam }) => listResources(tenantId, resourceType, { limit: 50, cursor: pageParam, signal }),
    initialPageParam: null as string | null,
    getNextPageParam: page => page.next_cursor ?? undefined,
  });
  const detail = useQuery({
    queryKey: tenantPrincipalQueryKey(tenantId, principalId, epoch, `resource:${resourceType}`, { id: selected }),
    queryFn: ({ signal }) => getResource<KnownResource>(tenantId, resourceType, selected!, signal),
    enabled: Boolean(selected),
  });

  const items = list.data?.pages.flatMap(page => page.items) ?? [];
  const visibleItems = items.filter(item => item.resource_id.toLocaleLowerCase().includes(filter.trim().toLocaleLowerCase()));
  const latestPage = list.data?.pages.at(-1);

  const selectResource = (resourceId: string | null) => {
    const next = new URLSearchParams(params);
    if (resourceId) { next.set('type', resourceType); next.set('resource', resourceId); }
    else { next.delete('type'); next.delete('resource'); }
    setParams(next, { replace: false });
  };

  return <Card className="resource-browser">
    <div className="section-row"><div><h3>{resourceLabels[resourceType]}</h3><p className="muted">{resourceType}</p></div>
      {latestPage ? <Badge tone={latestPage.data_state === 'FRESH' ? 'positive' : 'warning'}>{latestPage.data_state}</Badge> : null}</div>
    {list.isPending ? <DataSurface state="loading" />
      : list.isError ? unsupported(list.error) ? <Unavailable title={`${resourceLabels[resourceType]} chưa được backend hỗ trợ`} error={list.error} />
        : <DataSurface state={failureState(list.error)} retry={() => void list.refetch()} />
      : items.length === 0 ? <DataSurface state={latestPage?.data_state === 'STALE' ? 'stale' : 'empty'} retry={latestPage?.data_state === 'STALE' ? () => void list.refetch() : undefined} />
      : <>
        <label className="resource-filter">Tìm trong dữ liệu đã tải<input type="search" value={filter} onChange={event => setFilter(event.target.value)} placeholder="Nhập resource ID" /></label>
        {visibleItems.length ? <ul className="resource-list" aria-label={resourceLabels[resourceType]}>
          {visibleItems.map(item => <li key={`${item.resource_type}:${item.resource_id}`}>
            <button type="button" className={selected === item.resource_id ? 'resource-row selected' : 'resource-row'} onClick={() => selectResource(item.resource_id)}>
              <span><strong>{item.resource_id}</strong><small>Version {item.version}</small></span><ChevronRight size={17} aria-hidden="true" />
            </button>
          </li>)}
        </ul> : <div className="filtered-empty" role="status">Không có resource đã tải nào khớp “{filter}”.</div>}
        {list.hasNextPage ? <Button className="load-more" disabled={list.isFetchingNextPage} onClick={() => void list.fetchNextPage()}>{list.isFetchingNextPage ? 'Đang tải…' : 'Tải thêm'}</Button> : null}
      </>}
    {selected ? <div className="resource-detail" aria-live="polite">
      <div className="section-row"><h3>Chi tiết</h3><Button variant="ghost" onClick={() => selectResource(null)}>Đóng</Button></div>
      {detail.isPending ? <DataSurface state="loading" />
        : detail.isError ? unsupported(detail.error) ? <Unavailable title="Resource detail chưa được backend hỗ trợ" error={detail.error} />
          : <DataSurface state={failureState(detail.error)} retry={() => void detail.refetch()} />
        : <TypedResourceDetail type={resourceType} value={detail.data.value} updatedAt={detail.data.projection.updated_at} dataState={detail.data.projection.data_state} />}
    </div> : null}
  </Card>;
}

function TypedResourceDetail({ type, value, updatedAt, dataState }: { type: ResourceType; value: KnownResource; updatedAt: string; dataState: string }) {
  const fields = useMemo(() => detailFields(type, value), [type, value]);
  return <div>
    <div className="detail-status"><Badge tone={dataState === 'FRESH' ? 'positive' : 'warning'}>{dataState}</Badge><span>Cập nhật {formatTime(updatedAt)}</span></div>
    <dl className="detail-grid">{fields.map(([label, content]) => <div key={label}><dt>{label}</dt><dd>{content}</dd></div>)}</dl>
  </div>;
}

function detailFields(type: ResourceType, value: KnownResource): [string, ReactNode][] {
  const list = (items: readonly string[] | undefined) => items?.length ? items.join(', ') : '—';
  switch (type) {
    case 'Run': {
      const v = value as Run; return [['Run ID', v.run_id], ['Trạng thái', v.state], ['Workflow', `${v.workflow_id} · ${v.workflow_version}`], ['Grant', v.grant_id], ['Version', v.version], ['Cập nhật', formatTime(v.updated_at)]];
    }
    case 'Membership': {
      const v = value as Membership; return [['Membership ID', v.membership_id], ['Principal', v.principal_id], ['Roles', list(v.roles)], ['Trạng thái', v.state], ['Version', v.version]];
    }
    case 'Delegation': {
      const v = value as Delegation; return [['Grant ID', v.grant_id], ['Principal', v.principal.id], ['Actions', list(v.actions)], ['Hết hạn', formatTime(v.expires_at)], ['Trạng thái', v.state], ['Version', v.version]];
    }
    case 'EffectIntent': {
      const v = value as EffectIntent; return [['Effect ID', v.effect_id], ['Trạng thái', v.state], ['Action', v.action], ['Connection', v.connection_id], ['Run', v.run_id], ['Approval', v.approval_id ?? 'Không yêu cầu'], ['Version', v.version]];
    }
    case 'Connection': {
      const v = value as Connection; return [['Connection ID', v.connection_id], ['Provider', v.provider], ['Tài khoản', v.external_account], ['Scopes', list(v.scopes)], ['Capabilities', list(v.capabilities)], ['Trạng thái', v.state], ['Verified', v.verified_at ? formatTime(v.verified_at) : 'Chưa xác minh'], ['Version', v.version]];
    }
    case 'CostReservation': {
      const v = value as CostReservation; return [['Reservation ID', v.reservation_id], ['Run', v.run_id], ['Trạng thái', v.state], ['Upper bound', `${v.upper_bound_microunits} ${v.currency} µ`], ['Settled', v.settled_microunits == null ? 'Chưa settle' : `${v.settled_microunits} ${v.currency} µ`], ['Price profile', v.price_profile], ['Version', v.version]];
    }
    case 'PluginManifest': {
      const v = value as PluginManifest; return [['Plugin ID', v.id], ['Version', v.version], ['Publisher', v.publisher_id], ['Execution', v.execution_class], ['Capabilities', list(v.requested_capabilities)], ['License', v.license_expression], ['Handlers', v.handlers.length], ['Workflows', v.workflows.length]];
    }
    case 'UploadSession': {
      const v = value as UploadSession; return [['Upload ID', v.upload_id], ['Artifact', v.artifact_id], ['Trạng thái', v.state], ['Content type', v.content_type], ['Expected bytes', v.expected_size_bytes], ['Hết hạn', formatTime(v.expires_at)]];
    }
    case 'ConnectorAuthorization': {
      const v = value as ConnectorAuthorization; return [['Authorization ID', v.authorization_id], ['Connection', v.connection_id], ['Trạng thái', v.state], ['Hết hạn', formatTime(v.expires_at)], ['Return path', v.return_to], ['Authorization URL', <a href={v.authorization_url} rel="noreferrer">Mở authorization<ArrowRight size={14} aria-hidden="true" /></a>]];
    }
    default: {
      const fields: Record<string, readonly (readonly [string, string])[]> = {
        ProductComposition: [['Product', 'product_id'], ['Plugin', 'enabled_plugins'], ['Phiên bản', 'version'], ['Lock digest', 'plugin_lock_ref']],
        PluginInstallation: [['Plugin', 'plugin_id'], ['Trạng thái', 'state'], ['Quyền đã cấp', 'granted_capabilities'], ['Artifact digest', 'artifact_digest'], ['Phiên bản', 'version']],
        PluginInvocation: [['Plugin', 'plugin_id'], ['Handler', 'handler'], ['Trạng thái', 'state'], ['Đầu vào', 'input_ref'], ['Kết quả', 'output_ref'], ['Grant', 'grant_id'], ['Phiên bản', 'version']],
        BudgetSettings: [['Tiền tệ', 'currency'], ['Chu kỳ', 'period'], ['Giới hạn (µ)', 'limit_microunits'], ['Phiên bản', 'version']],
        KillSwitch: [['Phạm vi', 'scope'], ['Đích', 'target_id'], ['Trạng thái', 'state'], ['Lý do', 'reason'], ['Phiên bản', 'version']],
        MembershipInvitation: [['Email', 'email'], ['Vai trò', 'roles'], ['Trạng thái', 'state'], ['Hết hạn', 'expires_at'], ['Phiên bản', 'version']],
        IdentityConfigurationProposal: [['Đề xuất', 'proposal_id'], ['Trạng thái', 'state'], ['Phiên bản', 'version']],
        Approval: [['Phê duyệt', 'approval_id'], ['Trạng thái', 'state'], ['Quyết định', 'decision'], ['Hết hạn', 'expires_at'], ['Phiên bản', 'version']],
        ProviderProfile: [['Nhà cung cấp', 'provider'], ['Profile', 'profile_id'], ['Trạng thái', 'state'], ['Phiên bản', 'version']],
        Notification: [['Thông báo', 'notification_id'], ['Tiêu đề', 'title'], ['Trạng thái', 'state'], ['Phiên bản', 'version']],
        SupportGrant: [['Yêu cầu', 'request_id'], ['Lý do', 'reason'], ['Quyền', 'actions'], ['Trạng thái', 'state'], ['Hết hạn', 'expires_at'], ['Phiên bản', 'version']],
        CatalogListing: [['Mục catalog', 'listing_id'], ['Plugin', 'plugin_id'], ['Trạng thái', 'state'], ['Phiên bản', 'version']],
        Export: [['Lần xuất', 'export_id'], ['Định dạng', 'format'], ['Trạng thái', 'state'], ['Phiên bản', 'version']],
        ReleaseQualification: [['Bản phát hành', 'release_id'], ['Trạng thái', 'state'], ['Bằng chứng', 'evidence_ref'], ['Phiên bản', 'version']],
        ConformanceRun: [['Lần kiểm tra', 'run_id'], ['Profile', 'profile'], ['Trạng thái', 'state'], ['Phiên bản', 'version']],
        PolicyProposal: [['Đề xuất', 'proposal_id'], ['Trạng thái', 'state'], ['Lý do', 'reason'], ['Phiên bản', 'version']],
        PolicyDecision: [['Quyết định', 'decision'], ['Action', 'action'], ['Trạng thái', 'state'], ['Phiên bản', 'version']],
        Schedule: [['Lịch', 'schedule_id'], ['Workflow', 'workflow_id'], ['Trạng thái', 'state'], ['Phiên bản', 'version']],
        DeletionRequest: [['Yêu cầu', 'request_id'], ['Trạng thái', 'state'], ['Lý do', 'reason'], ['Phiên bản', 'version']],
      };
      const data = value as Record<string, unknown>;
      return (fields[type] ?? []).filter(([, key]) => data[key] !== undefined).map(([label, key]) => [label, viewValue(data[key])]);
    }
  }
}

function viewValue(value: unknown): string {
  if (typeof value === 'string' || typeof value === 'number') return String(value);
  if (typeof value === 'boolean') return value ? 'Có' : 'Không';
  if (Array.isArray(value)) return value.filter(item => typeof item === 'string' || typeof item === 'number').join(', ') || '—';
  if (value && typeof value === 'object' && 'digest' in value && typeof value.digest === 'string') return value.digest;
  return '—';
}

function formatTime(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? value : new Intl.DateTimeFormat('vi-VN', { dateStyle: 'medium', timeStyle: 'short' }).format(date);
}

function Unavailable({ title, error }: { title: string; error: unknown }) {
  const failure = error instanceof ApiFailure ? error : null;
  return <div className="unavailable" role="status"><ShieldAlert size={22} aria-hidden="true" /><div><strong>{title}</strong>
    <p>{failure?.body?.message ?? 'BFF báo capability này chưa sẵn sàng. Không có dữ liệu mẫu được thay thế.'}</p>
    {failure?.body?.correlation_id ? <small>Correlation {failure.body.correlation_id}</small> : null}</div></div>;
}

type Attempt = { fingerprint: string; key: string };

function CommandForm({ spec, tenantId, csrf, onChanged }: { spec: CommandFormSpec; tenantId: string; csrf: string; onChanged: () => void }) {
  const [values, setValues] = useState<Record<string, string | boolean>>(() => Object.fromEntries(spec.fields.map(field => [field.name, field.name.endsWith('_tenant') ? tenantId : field.kind === 'checkbox' ? false : ''])));
  const [confirmed, setConfirmed] = useState(false);
  const attempt = useRef<Attempt | null>(null);
  const mutation = useMutation({
    mutationFn: ({ payload, key }: { payload: unknown; key: string }) => submitCommand(`/v1/tenants/${encodeURIComponent(tenantId)}/commands/${spec.operation}`, payload, { csrf, idempotencyKey: key }),
    onSuccess: receipt => { attempt.current = null; onChanged(); if (receipt.state === 'SUCCEEDED') setConfirmed(false); },
  });

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (spec.destructive && !confirmed) return;
    const payload = spec.build(values);
    const fingerprint = JSON.stringify(payload);
    if (!attempt.current || attempt.current.fingerprint !== fingerprint) attempt.current = { fingerprint, key: idempotencyKey() };
    mutation.mutate({ payload, key: attempt.current.key });
  };
  const error = mutation.error;
  const conflict = error instanceof ApiFailure && (error.status === 409 || error.body?.recovery_action === 'REVIEW_CONFLICT');
  const unknown = error instanceof ApiFailure && (error.body?.effect_state === 'UNKNOWN' || error.body?.recovery_action === 'RECONCILE');
  const canRetrySame = error instanceof ApiFailure && error.body?.retryable === true && error.body.effect_state === 'NOT_SENT' && !conflict;

  return <Card className="command-card">
    <div className="command-heading"><div><h3>{spec.title}</h3><code>{spec.operation}</code></div>{spec.destructive ? <Badge tone="warning">XÁC NHẬN</Badge> : null}</div>
    <p>{spec.description}</p>
    <form onSubmit={submit} className="command-form">
      {spec.fields.map(field => <FormField key={field.name} field={field} value={values[field.name] ?? ''} onChange={value => {
        setValues(current => ({ ...current, [field.name]: value })); mutation.reset();
      }} />)}
      {spec.destructive ? <label className="confirm-field"><input type="checkbox" checked={confirmed} onChange={event => setConfirmed(event.target.checked)} />
        <span>Tôi đã kiểm tra đúng target và hiểu thao tác này thay đổi trạng thái.</span></label> : null}
      <div className="form-actions"><Button variant="primary" type="submit" disabled={mutation.isPending || !csrf || (spec.destructive && !confirmed)}>
        {mutation.isPending ? 'Đang gửi…' : 'Gửi thao tác'}</Button>
        {!csrf ? <span className="form-hint">Session hiện tại không có CSRF token cho unsafe request.</span> : null}</div>
    </form>
    <CommandFeedback receipt={mutation.data} error={error} pending={mutation.isPending} />
    {conflict ? <div className="recovery-row"><Button type="button" onClick={() => { mutation.reset(); onChanged(); }}><RefreshCw size={16} aria-hidden="true" />Tải trạng thái mới để review</Button><span>Command không được tự replay.</span></div> : null}
    {canRetrySame && !unknown ? <div className="recovery-row"><Button type="button" onClick={() => {
      const payload = spec.build(values); const fingerprint = JSON.stringify(payload);
      if (attempt.current?.fingerprint === fingerprint) mutation.mutate({ payload, key: attempt.current.key });
    }}>Thử lại cùng request</Button><span>Giữ nguyên idempotency key vì payload không đổi.</span></div> : null}
  </Card>;
}

function FormField({ field, value, onChange }: { field: FieldSpec; value: string | boolean; onChange: (value: string | boolean) => void }) {
  const id = `field-${field.name}`;
  if (field.kind === 'checkbox') return <label className="checkbox-field" htmlFor={id}><input id={id} type="checkbox" checked={Boolean(value)} onChange={event => onChange(event.target.checked)} /><span>{field.label}</span></label>;
  const common = { id, name: field.name, required: field.required, value: String(value), onChange: (event: { target: { value: string } }) => onChange(event.target.value), 'aria-describedby': field.help ? `${id}-help` : undefined };
  return <label className="form-field" htmlFor={id}><span>{field.label}{field.required ? <em aria-hidden="true"> *</em> : null}</span>
    {field.kind === 'select' ? <select {...common}>{field.options?.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}</select>
      : field.kind === 'textarea' ? <textarea {...common} rows={3} placeholder={field.placeholder} />
      : <input {...common} type={field.kind === 'email' ? 'email' : field.kind === 'number' ? 'number' : field.kind === 'datetime-local' ? 'datetime-local' : 'text'} min={field.min} placeholder={field.placeholder} />}
    {field.help ? <small id={`${id}-help`}>{field.help}</small> : null}
  </label>;
}

function CommandFeedback({ receipt, error, pending }: { receipt?: CommandReceipt; error: unknown; pending: boolean }) {
  if (pending) return <div className="command-feedback pending" role="status" aria-live="polite"><CircleDot size={18} aria-hidden="true" /><div><strong>Đang gửi command</strong><p>Chưa có business outcome.</p></div></div>;
  if (receipt) return <div className={`command-feedback ${receipt.state === 'SUCCEEDED' ? 'success' : 'pending'}`} role="status" aria-live="polite">
    {receipt.state === 'SUCCEEDED' ? <CheckCircle2 size={18} aria-hidden="true" /> : <CircleDot size={18} aria-hidden="true" />}
    <div><strong>{receipt.state === 'SUCCEEDED' ? 'Command đã hoàn tất đồng bộ' : 'Command đã được nhận'}</strong>
      <p>{receipt.state === 'ACCEPTED' ? 'Receipt ACCEPTED chưa phải business success. Theo dõi resource/run/effect được trả về.' : `Command ${receipt.command_id}`}</p>
      <ReceiptLinks receipt={receipt} /></div></div>;
  if (!error) return null;
  if (unsupported(error)) return <Unavailable title="Backend chưa hỗ trợ command này" error={error} />;
  const api = error instanceof ApiFailure ? error : null;
  const conflict = api && (api.status === 409 || api.body?.recovery_action === 'REVIEW_CONFLICT');
  const unknown = api && (api.body?.effect_state === 'UNKNOWN' || api.body?.recovery_action === 'RECONCILE');
  return <div className={`command-feedback ${unknown ? 'unknown' : 'error'}`} role="alert" aria-live="assertive"><AlertTriangle size={18} aria-hidden="true" /><div>
    <strong>{unknown ? 'Kết quả chưa xác định' : conflict ? 'Có conflict cần review' : 'Command không hoàn tất'}</strong>
    <p>{api?.body?.message ?? (error instanceof Error ? error.message : 'Không nhận được kết quả hợp lệ.')}</p>
    {api?.body?.details?.length ? <ul>{api.body.details.map((detail, index) => <li key={`${detail.field}-${index}`}>{detail.field}: {detail.reason}</li>)}</ul> : null}
    {api?.body?.correlation_id ? <small>Correlation {api.body.correlation_id}</small> : null}</div></div>;
}

function ReceiptLinks({ receipt }: { receipt: CommandReceipt }) {
  const refs = [receipt.resource ? `${receipt.resource.resource_type} ${receipt.resource.resource_id}` : null, receipt.run_id ? `Run ${receipt.run_id}` : null, receipt.effect_id ? `Effect ${receipt.effect_id}` : null].filter(Boolean);
  return refs.length ? <div className="receipt-refs">{refs.map(ref => <Badge key={ref}>{ref}</Badge>)}</div> : null;
}

let fallbackKey = 0;
function idempotencyKey() {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') return crypto.randomUUID();
  fallbackKey += 1;
  return `ui:${Date.now()}:${fallbackKey}`;
}
