import type { ResourceType } from '@masonwing/api-client';

export type FieldKind = 'text' | 'email' | 'number' | 'select' | 'textarea' | 'tags' | 'datetime-local' | 'checkbox';
export type FieldSpec = Readonly<{
  name: string;
  label: string;
  kind?: FieldKind;
  required?: boolean;
  placeholder?: string;
  help?: string;
  options?: readonly { value: string; label: string }[];
  min?: number;
}>;

export type CommandFormSpec = Readonly<{
  operation: string;
  title: string;
  description: string;
  fields: readonly FieldSpec[];
  build: (values: Record<string, string | boolean>) => unknown;
  destructive?: boolean;
}>;

export type FeatureConfig = Readonly<{
  description: string;
  resources: readonly ResourceType[];
  readNote?: string;
}>;

const text = (name: string, label: string, required = true, placeholder?: string): FieldSpec => ({ name, label, required, placeholder });
const num = (name: string, label: string, min = 0): FieldSpec => ({ name, label, kind: 'number', required: true, min });
const tags = (name: string, label: string, required = false, help = 'Phân tách bằng dấu phẩy.'): FieldSpec => ({ name, label, kind: 'tags', required, help });
const select = (name: string, label: string, values: readonly string[]): FieldSpec => ({
  name, label, kind: 'select', required: true, options: values.map(value => ({ value, label: value.replaceAll('_', ' ') })),
});
const dateTime = (name: string, label: string): FieldSpec => ({ name, label, kind: 'datetime-local', required: true });
const bool = (name: string, label: string): FieldSpec => ({ name, label, kind: 'checkbox' });

const splitTags = (value: string | boolean) => String(value).split(',').map(item => item.trim()).filter(Boolean);
const iso = (value: string | boolean) => new Date(String(value)).toISOString();
const integer = (value: string | boolean) => Number.parseInt(String(value), 10);
const resource = (v: Record<string, string | boolean>, prefix: string) => ({
  resource_type: String(v[`${prefix}_type`]), resource_id: String(v[`${prefix}_id`]), version: integer(v[`${prefix}_version`]),
});
const artifact = (v: Record<string, string | boolean>, prefix: string) => ({
  artifact_id: String(v[`${prefix}_id`]), tenant_id: String(v[`${prefix}_tenant`]), digest: String(v[`${prefix}_digest`]),
  schema_version: String(v[`${prefix}_schema`]), classification: String(v[`${prefix}_classification`]),
});
const artifactFields = (prefix: string, label: string): FieldSpec[] => [
  text(`${prefix}_id`, `${label} · ID`), text(`${prefix}_tenant`, `${label} · Tenant`), text(`${prefix}_digest`, `${label} · Digest`),
  text(`${prefix}_schema`, `${label} · Schema version`), select(`${prefix}_classification`, `${label} · Phân loại`, ['PUBLIC', 'INTERNAL', 'CONFIDENTIAL', 'RESTRICTED']),
];
const resourceFields = (prefix: string, label: string): FieldSpec[] => [
  text(`${prefix}_type`, `${label} · Loại`), text(`${prefix}_id`, `${label} · ID`), num(`${prefix}_version`, `${label} · Version`),
];

export const featureConfig: Record<string, FeatureConfig> = {
  'F-001': { description: 'Ghép một product từ các phiên bản plugin đã được kiểm chứng.', resources: ['ProductComposition', 'PluginManifest'] },
  'F-002': { description: 'Khám phá plugin manifests và danh tính artifact đã cài.', resources: ['PluginManifest'] },
  'F-003': { description: 'Quản lý trạng thái, phiên bản và quyền của plugin đã cài.', resources: ['PluginInstallation'] },
  'F-004': { description: 'Thực thi plugin và xem kết quả đã được xác minh.', resources: ['PluginInvocation'] },
  'F-005': { description: 'Xem đề xuất cấu hình đăng nhập của workspace.', resources: ['IdentityConfigurationProposal'] },
  'F-006': { description: 'Quản lý thành viên, lời mời và phiên làm việc của workspace.', resources: ['Membership', 'MembershipInvitation'] },
  'F-007': { description: 'Xem kết quả đánh giá và đề xuất thay đổi chính sách truy cập.', resources: ['PolicyDecision', 'PolicyProposal'] },
  'F-008': { description: 'Xem delegation hiện hành và thu hồi grant theo version.', resources: ['Delegation'] },
  'F-009': { description: 'Trạng thái dữ liệu được đọc qua projection của từng aggregate.', resources: [], readNote: 'Baseline không công bố một Data aggregate chung.' },
  'F-010': { description: 'Theo dõi file upload và các yêu cầu xóa dữ liệu.', resources: ['UploadSession', 'DeletionRequest'] },
  'F-011': { description: 'Xem run thực và gửi start/cancel với idempotency, version guard.', resources: ['Run'] },
  'F-012': { description: 'Xem và quyết định trên đúng phiên bản đề xuất cần phê duyệt.', resources: ['Approval'] },
  'F-013': { description: 'Theo dõi thao tác bên ngoài và các quy tắc tạm dừng thực thi.', resources: ['EffectIntent', 'KillSwitch'] },
  'F-014': { description: 'Xem giới hạn ngân sách và các khoản đã được giữ chỗ.', resources: ['BudgetSettings', 'CostReservation'] },
  'F-015': { description: 'Xem cấu hình và trạng thái xác minh nhà cung cấp.', resources: ['ProviderProfile'] },
  'F-016': { description: 'Theo dõi connection và authorization state trước khi revoke hoặc reauthorize.', resources: ['Connection', 'ConnectorAuthorization'] },
  'F-017': { description: 'Quản lý lịch thực thi và trạng thái xử lý sự kiện.', resources: ['Schedule'] },
  'F-018': { description: 'Đăng ký UI contribution có namespace, slot và required action rõ ràng.', resources: ['PluginManifest'] },
  'F-019': { description: 'Theo dõi các lần xuất dữ liệu và thông báo của workspace.', resources: ['Export', 'Notification'] },
  'F-020': { description: 'Xem yêu cầu hỗ trợ cùng phạm vi quyền và thời hạn cụ thể.', resources: ['SupportGrant'] },
  'F-021': { description: 'Kiểm tra bằng chứng và kết quả xác minh bản phát hành.', resources: ['ReleaseQualification'] },
  'F-022': { description: 'Xem các mục catalog và plugin trong workspace.', resources: ['CatalogListing', 'PluginManifest'] },
  'F-023': { description: 'Xem kết quả kiểm tra tương thích của từng plugin.', resources: ['ConformanceRun'] },
};

export const commandForms: Record<string, CommandFormSpec> = Object.fromEntries(([
  {
    operation: 'product.compose', title: 'Lắp ghép product', description: 'Dùng plugin lock artifact đã tồn tại; receipt ACCEPTED chỉ có nghĩa request đã được nhận.',
    fields: [text('product_id', 'Product ID'), ...artifactFields('lock', 'Plugin lock')],
    build: v => ({ product_id: v.product_id, plugin_lock_ref: artifact(v, 'lock') }),
  },
  {
    operation: 'plugin.enable', title: 'Enable plugin', description: 'Version guard ngăn ghi đè trạng thái plugin đã thay đổi.',
    fields: [text('plugin_id', 'Plugin ID'), text('artifact_digest', 'Artifact digest'), tags('grants', 'Grant IDs'), num('expected_version', 'Expected version')],
    build: v => ({ plugin_id: v.plugin_id, artifact_digest: v.artifact_digest, grants: splitTags(v.grants), expected_version: integer(v.expected_version) }),
  },
  {
    operation: 'plugin.disable', title: 'Disable plugin', description: 'Yêu cầu lý do và version hiện tại.', destructive: true,
    fields: [text('plugin_id', 'Plugin ID'), num('expected_version', 'Expected version'), { ...text('reason', 'Lý do'), kind: 'textarea' }],
    build: v => ({ plugin_id: v.plugin_id, expected_version: integer(v.expected_version), reason: v.reason }),
  },
  {
    operation: 'plugin.revoke', title: 'Revoke plugin artifact', description: 'Thu hồi đúng artifact digest đã định danh.', destructive: true,
    fields: [text('plugin_id', 'Plugin ID'), text('artifact_digest', 'Artifact digest'), { ...text('reason', 'Lý do'), kind: 'textarea' }],
    build: v => ({ plugin_id: v.plugin_id, artifact_digest: v.artifact_digest, reason: v.reason }),
  },
  {
    operation: 'plugin.uninstall', title: 'Uninstall plugin', description: 'Không tự động xóa dữ liệu; chọn data policy trong contract.', destructive: true,
    fields: [text('plugin_id', 'Plugin ID'), num('expected_version', 'Expected version'), select('data_policy', 'Data policy', ['RETAIN', 'EXPORT_THEN_RETAIN'])],
    build: v => ({ plugin_id: v.plugin_id, expected_version: integer(v.expected_version), data_policy: v.data_policy }),
  },
  {
    operation: 'identity.configure', title: 'Cấu hình OIDC', description: 'Client secret là secret reference; browser không nhận raw secret.',
    fields: [text('issuer', 'Issuer URL'), text('client_id', 'Client ID'), text('client_secret_ref', 'Client secret reference'), tags('allowed_redirect_uris', 'Allowed redirect URIs', true)],
    build: v => ({ issuer: v.issuer, client_id: v.client_id, client_secret_ref: v.client_secret_ref, allowed_redirect_uris: splitTags(v.allowed_redirect_uris) }),
  },
  {
    operation: 'membership.invite', title: 'Mời thành viên', description: 'Invitation chỉ tạo membership theo roles được backend cho phép.',
    fields: [{ ...text('email', 'Email'), kind: 'email' }, tags('roles', 'Roles', true), dateTime('expires_at', 'Hết hạn')],
    build: v => ({ email: v.email, roles: splitTags(v.roles), expires_at: iso(v.expires_at) }),
  },
  {
    operation: 'membership.accept', title: 'Chấp nhận lời mời', description: 'Invite token được gửi thẳng tới same-origin BFF.',
    fields: [text('invite_token', 'Invite token')], build: v => ({ invite_token: v.invite_token }),
  },
  {
    operation: 'membership.change', title: 'Đổi roles', description: 'Cập nhật có version guard.',
    fields: [text('membership_id', 'Membership ID'), tags('roles', 'Roles', true), num('expected_version', 'Expected version')],
    build: v => ({ membership_id: v.membership_id, roles: splitTags(v.roles), expected_version: integer(v.expected_version) }),
  },
  {
    operation: 'membership.revoke', title: 'Thu hồi membership', description: 'Destructive action yêu cầu target, version và lý do.', destructive: true,
    fields: [text('membership_id', 'Membership ID'), num('expected_version', 'Expected version'), { ...text('reason', 'Lý do'), kind: 'textarea' }],
    build: v => ({ membership_id: v.membership_id, expected_version: integer(v.expected_version), reason: v.reason }),
  },
  {
    operation: 'policy.evaluate', title: 'Kiểm tra policy', description: 'Backend vẫn là authorization authority.',
    fields: [text('action', 'Action'), ...resourceFields('target', 'Resource')],
    build: v => ({ action: v.action, resource: resource(v, 'target') }),
  },
  {
    operation: 'grant.revoke', title: 'Thu hồi delegation', description: 'Thu hồi grant hiện hành bằng optimistic version.', destructive: true,
    fields: [text('grant_id', 'Grant ID'), num('expected_version', 'Expected version'), { ...text('reason', 'Lý do'), kind: 'textarea' }],
    build: v => ({ grant_id: v.grant_id, expected_version: integer(v.expected_version), reason: v.reason }),
  },
  {
    operation: 'artifact.begin', title: 'Bắt đầu upload artifact', description: 'Khai báo metadata trước khi upload bytes.',
    fields: [select('classification', 'Phân loại', ['PUBLIC', 'INTERNAL', 'CONFIDENTIAL', 'RESTRICTED']), text('content_type', 'Content type'), num('size_bytes', 'Kích thước bytes'), text('expected_digest', 'Expected digest')],
    build: v => ({ classification: v.classification, content_type: v.content_type, size_bytes: integer(v.size_bytes), expected_digest: v.expected_digest }),
  },
  {
    operation: 'artifact.finalize', title: 'Finalize artifact', description: 'Finalize chỉ sau khi bytes đã được upload đầy đủ.',
    fields: [text('artifact_id', 'Artifact ID'), text('observed_digest', 'Observed digest')],
    build: v => ({ artifact_id: v.artifact_id, observed_digest: v.observed_digest }),
  },
  {
    operation: 'run.start', title: 'Bắt đầu run', description: 'Run nhận input artifact và delegation grant rõ ràng.',
    fields: [text('workflow_id', 'Workflow ID'), text('workflow_version', 'Workflow version'), ...artifactFields('input', 'Input artifact'), text('grant_id', 'Grant ID')],
    build: v => ({ workflow_id: v.workflow_id, workflow_version: v.workflow_version, input_ref: artifact(v, 'input'), grant_id: v.grant_id }),
  },
  {
    operation: 'run.cancel', title: 'Hủy run', description: 'Cancel dùng version guard và không giả định run đã dừng ngay.', destructive: true,
    fields: [text('run_id', 'Run ID'), num('expected_version', 'Expected version'), { ...text('reason', 'Lý do'), kind: 'textarea' }],
    build: v => ({ run_id: v.run_id, expected_version: integer(v.expected_version), reason: v.reason }),
  },
  {
    operation: 'approval.decide', title: 'Quyết định proposal', description: 'Digest và version phải khớp proposal đang review; conflict phải review lại.',
    fields: [text('proposal_id', 'Proposal ID'), select('decision', 'Quyết định', ['APPROVE', 'REJECT', 'CANCEL']), num('expected_version', 'Expected version'), text('content_digest', 'Content digest'), { ...text('reason', 'Lý do'), kind: 'textarea' }],
    build: v => ({ proposal_id: v.proposal_id, decision: v.decision, expected_version: integer(v.expected_version), content_digest: v.content_digest, reason: v.reason }),
  },
  {
    operation: 'effect.dispatch', title: 'Dispatch effect', description: 'Gửi effect đã được authorize; unknown outcome sẽ khóa blind retry.',
    fields: [text('effect_id', 'Effect ID'), num('expected_version', 'Expected version')], build: v => ({ effect_id: v.effect_id, expected_version: integer(v.expected_version) }),
  },
  {
    operation: 'effect.reconcile', title: 'Đối soát effect', description: 'Dùng cho effect có trạng thái cần đối soát.',
    fields: [text('effect_id', 'Effect ID'), num('expected_version', 'Expected version')], build: v => ({ effect_id: v.effect_id, expected_version: integer(v.expected_version) }),
  },
  {
    operation: 'kill-switch.set', title: 'Đặt kill switch', description: 'Scope và target phải được xác nhận rõ.', destructive: true,
    fields: [select('scope', 'Scope', ['TENANT', 'PLUGIN', 'CONNECTION']), text('target_id', 'Target ID'), bool('active', 'Bật kill switch'), { ...text('reason', 'Lý do'), kind: 'textarea' }],
    build: v => ({ scope: v.scope, target_id: v.target_id, active: Boolean(v.active), reason: v.reason }),
  },
  {
    operation: 'budget.configure', title: 'Cấu hình budget', description: 'Live budget vẫn chịu policy của backend; form không suy diễn activation.',
    fields: [text('currency', 'Currency', true, 'USD'), num('limit_microunits', 'Limit (microunits)'), select('period', 'Chu kỳ', ['RUN', 'DAY', 'MONTH']), num('expected_version', 'Expected version')],
    build: v => ({ currency: v.currency, limit_microunits: integer(v.limit_microunits), period: v.period, expected_version: integer(v.expected_version) }),
  },
  {
    operation: 'provider.compact', title: 'Compact conversation', description: 'Compaction chỉ dùng profile đã cấu hình.',
    fields: [text('conversation_id', 'Conversation ID'), text('profile_id', 'Profile ID'), num('expected_version', 'Expected version')],
    build: v => ({ conversation_id: v.conversation_id, profile_id: v.profile_id, expected_version: integer(v.expected_version) }),
  },
  {
    operation: 'connection.authorize', title: 'Kết nối tài khoản', description: 'Backend tạo authorization URL; credentials không đi qua form này.',
    fields: [text('provider', 'Provider'), tags('requested_scopes', 'Requested scopes', true), text('return_to', 'Return path', true, '/workspace')],
    build: v => ({ provider: v.provider, requested_scopes: splitTags(v.requested_scopes), return_to: v.return_to }),
  },
  {
    operation: 'connection.revoke', title: 'Thu hồi connection', description: 'Revoke theo connection version hiện tại.', destructive: true,
    fields: [text('connection_id', 'Connection ID'), num('expected_version', 'Expected version'), { ...text('reason', 'Lý do'), kind: 'textarea' }],
    build: v => ({ connection_id: v.connection_id, expected_version: integer(v.expected_version), reason: v.reason }),
  },
  {
    operation: 'deadletter.replay', title: 'Replay dead letter', description: 'Replay đúng event ID, có lý do audit.',
    fields: [text('event_id', 'Event ID'), { ...text('reason', 'Lý do'), kind: 'textarea' }], build: v => ({ event_id: v.event_id, reason: v.reason }),
  },
  {
    operation: 'ui.register', title: 'Đăng ký UI contribution', description: 'Một contribution được gửi bằng typed fields; route collision vẫn bị host từ chối.',
    fields: [text('plugin_id', 'Plugin ID'), text('contribution_id', 'Contribution ID'), select('slot', 'Slot', ['NAVIGATION', 'ROUTE', 'DASHBOARD', 'ENTITY_TAB', 'EDITOR_PANEL', 'SETTINGS', 'COMMAND']), text('path', 'Path'), text('ui_contract_version', 'UI contract version'), select('trust', 'Trust', ['TRUSTED_BUNDLE', 'DECLARATIVE', 'ISOLATED_IFRAME']), text('required_action', 'Required action')],
    build: v => ({ plugin_id: v.plugin_id, contributions: [{ id: v.contribution_id, slot: v.slot, path: v.path, ui_contract_version: v.ui_contract_version, trust: v.trust, required_action: v.required_action }] }),
  },
  {
    operation: 'notification.read', title: 'Đánh dấu đã đọc', description: 'Chỉ đánh dấu notification cụ thể.', fields: [text('notification_id', 'Notification ID')], build: v => ({ notification_id: v.notification_id }),
  },
  {
    operation: 'support.request', title: 'Yêu cầu support access', description: 'Phạm vi action và thời hạn phải hữu hạn.',
    fields: [tags('actions', 'Actions', true), dateTime('expires_at', 'Hết hạn'), { ...text('reason', 'Lý do'), kind: 'textarea' }],
    build: v => ({ actions: splitTags(v.actions), expires_at: iso(v.expires_at), reason: v.reason }),
  },
  {
    operation: 'conformance.run', title: 'Chạy conformance suite', description: 'Profile live chỉ là qualification target; không tự kích hoạt provider.',
    fields: [text('artifact_digest', 'Artifact digest'), text('suite_version', 'Suite version'), select('profile', 'Profile', ['MOCK', 'LOCAL', 'STAGING_LIVE'])],
    build: v => ({ artifact_digest: v.artifact_digest, suite_version: v.suite_version, profile: v.profile }),
  },
] satisfies CommandFormSpec[]).map(spec => [spec.operation, spec]));

export const resourceLabels: Record<ResourceType, string> = {
  Run: 'Runs', Membership: 'Memberships', Delegation: 'Delegations', EffectIntent: 'Effects', Connection: 'Connections',
  CostReservation: 'Cost reservations', PluginManifest: 'Plugins', UploadSession: 'Uploads', ConnectorAuthorization: 'Authorizations',
  ProductComposition: 'Products', PluginInstallation: 'Plugin đã cài', PluginInvocation: 'Kết quả thực thi',
  Approval: 'Phê duyệt', BudgetSettings: 'Ngân sách', KillSwitch: 'Tạm dừng thực thi', ProviderProfile: 'Nhà cung cấp',
  Notification: 'Thông báo', SupportGrant: 'Quyền hỗ trợ', CatalogListing: 'Catalog', Export: 'Xuất dữ liệu',
  ReleaseQualification: 'Xác minh bản phát hành', ConformanceRun: 'Kiểm tra tương thích', PolicyProposal: 'Đề xuất chính sách',
  Schedule: 'Lịch thực thi', DeletionRequest: 'Yêu cầu xóa', PolicyDecision: 'Đánh giá chính sách',
  MembershipInvitation: 'Lời mời thành viên', IdentityConfigurationProposal: 'Cấu hình đăng nhập',
};

export const advancedOperationNotes: Record<string, string> = {
  'plugin.install': 'Manifest đầy đủ gồm handlers, workflows, tools, dependencies và provenance; thực hiện từ catalog/SDK đã validate manifest.',
  'plugin.upgrade': 'Upgrade cần full new_manifest và migration metadata; dùng catalog/SDK để tránh nhập thiếu contract fields.',
  'plugin.invoke': 'Invocation cần typed input artifact và grant; dùng entity/tool surface phát sinh input đúng schema.',
  'policy.propose': 'Policy proposal cần policy/review artifact references và expected version.',
  'grant.create': 'Grant creation cần principal, action set, resource set và expiry đã được review cùng nhau.',
  'deletion.request': 'Deletion request cần typed ResourceRef và confirmation; khởi tạo từ resource detail để tránh sai target.',
  'effect.propose': 'Effect proposal cần target ResourceRef, content ArtifactRef, grant và price profile; khởi tạo từ workflow/effect context.',
  'effect.compensate': 'Compensation cần compensation artifact và approval binding; khởi tạo từ effect detail.',
  'provider.configure': 'Provider profile có capability, data class, retention, price và qualification fields; dùng provider setup surface chuyên biệt.',
  'schedule.create': 'Schedule cần workflow input ArtifactRef, grant và DST/overlap policy; dùng workflow scheduler surface.',
  'export.create': 'Export filter là ArtifactRef; khởi tạo export từ resource/search context.',
  'release.qualify': 'Qualification cần evidence ArtifactRef; khởi tạo từ evidence/release context.',
  'catalog.submit': 'Catalog submission cần full signed PluginManifest; submit từ plugin package/catalog workflow.',
  'catalog.review': 'Catalog review cần evidence ArtifactRef và listing version; mở từ listing review detail.',
};
