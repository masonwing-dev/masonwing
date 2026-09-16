/* Generated from immutable spec by pnpm contracts:generate. Do not edit. */

/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "OpaqueId".
 */
export type OpaqueId = string;
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Digest".
 */
export type Digest = string;
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Timestamp".
 */
export type Timestamp = string;
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Version".
 */
export type Version = string;

export interface UrnMasonwingContracts100 {
  OpaqueId?: OpaqueId;
  Digest?: Digest;
  Timestamp?: Timestamp;
  Version?: Version;
  ArtifactRef?: ArtifactRef;
  ResourceRef?: ResourceRef;
  PrincipalRef?: PrincipalRef;
  DecisionContext?: DecisionContext;
  RunContext?: RunContext;
  Error?: Error;
  CommandReceipt?: CommandReceipt;
  EventEnvelope?: EventEnvelope;
  ApprovalBinding?: ApprovalBinding;
  EffectIntent?: EffectIntent;
  EffectReceipt?: EffectReceipt;
  CostReservation?: CostReservation;
  Connection?: Connection;
  PluginDependency?: PluginDependency;
  UIContribution?: UIContribution;
  PluginManifest?: PluginManifest;
  ProviderProfile?: ProviderProfile;
  ProviderNativeEnvelope?: ProviderNativeEnvelope;
  CapabilityReport?: CapabilityReport;
  SessionView?: SessionView;
  SourceRights?: SourceRights;
  ReadCollection?: ReadCollection;
  ResourceProjection?: ResourceProjection;
  Run?: Run;
  Membership?: Membership;
  Delegation?: Delegation;
  HandlerContract?: HandlerContract;
  WorkflowNode?: WorkflowNode;
  WorkflowDefinition?: WorkflowDefinition;
  ToolDescriptor?: ToolDescriptor;
  ToolInvocation?: ToolInvocation;
  ToolResult?: ToolResult;
  EvidenceRunRecord?: EvidenceRunRecord;
  UploadSession?: UploadSession;
  ConnectorAuthorization?: ConnectorAuthorization;
  Command_product_compose?: CommandProductCompose;
  Command_plugin_install?: CommandPluginInstall;
  Command_plugin_enable?: CommandPluginEnable;
  Command_plugin_disable?: CommandPluginDisable;
  Command_plugin_upgrade?: CommandPluginUpgrade;
  Command_plugin_revoke?: CommandPluginRevoke;
  Command_plugin_uninstall?: CommandPluginUninstall;
  Command_plugin_invoke?: CommandPluginInvoke;
  Command_identity_configure?: CommandIdentityConfigure;
  Command_membership_invite?: CommandMembershipInvite;
  Command_membership_accept?: CommandMembershipAccept;
  Command_membership_change?: CommandMembershipChange;
  Command_membership_revoke?: CommandMembershipRevoke;
  Command_policy_evaluate?: CommandPolicyEvaluate;
  Command_policy_propose?: CommandPolicyPropose;
  Command_grant_create?: CommandGrantCreate;
  Command_grant_revoke?: CommandGrantRevoke;
  Command_artifact_begin?: CommandArtifactBegin;
  Command_artifact_finalize?: CommandArtifactFinalize;
  Command_deletion_request?: CommandDeletionRequest;
  Command_run_start?: CommandRunStart;
  Command_run_cancel?: CommandRunCancel;
  Command_approval_decide?: CommandApprovalDecide;
  Command_effect_propose?: CommandEffectPropose;
  Command_effect_dispatch?: CommandEffectDispatch;
  Command_effect_reconcile?: CommandEffectReconcile;
  Command_effect_compensate?: CommandEffectCompensate;
  Command_budget_configure?: CommandBudgetConfigure;
  Command_provider_configure?: CommandProviderConfigure;
  Command_provider_compact?: CommandProviderCompact;
  Command_connection_authorize?: CommandConnectionAuthorize;
  Command_connection_revoke?: CommandConnectionRevoke;
  Command_schedule_create?: CommandScheduleCreate;
  Command_deadletter_replay?: CommandDeadletterReplay;
  Command_ui_register?: CommandUiRegister;
  Command_export_create?: CommandExportCreate;
  Command_notification_read?: CommandNotificationRead;
  Command_support_request?: CommandSupportRequest;
  Command_kill_switch_set?: CommandKillSwitchSet;
  Command_release_qualify?: CommandReleaseQualify;
  Command_catalog_submit?: CommandCatalogSubmit;
  Command_catalog_review?: CommandCatalogReview;
  Command_conformance_run?: CommandConformanceRun;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "ArtifactRef".
 */
export interface ArtifactRef {
  artifact_id: string;
  tenant_id: string;
  digest: string;
  schema_version: string;
  classification: "PUBLIC" | "INTERNAL" | "CONFIDENTIAL" | "RESTRICTED";
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "ResourceRef".
 */
export interface ResourceRef {
  resource_type: string;
  resource_id: string;
  version: number;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "PrincipalRef".
 */
export interface PrincipalRef {
  type: "USER" | "SERVICE" | "AUTOMATION" | "SUPPORT";
  id: string;
  issuer: string;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "DecisionContext".
 */
export interface DecisionContext {
  tenant_id: string;
  principal: PrincipalRef;
  action: string;
  resource: ResourceRef;
  policy_version: string;
  evaluated_at: string;
  decision: "ALLOW" | "DENY";
  /**
   * @maxItems 100
   */
  diagnostic_codes: string[];
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "RunContext".
 */
export interface RunContext {
  tenant_id: string;
  run_id: string;
  trace_id: string;
  principal: PrincipalRef;
  delegation_id: string;
  plugin_digest: string;
  workflow_version: string;
  fence: number;
  deadline: string;
  /**
   * @maxItems 100
   */
  grant_actions: string[];
  /**
   * @maxItems 500
   */
  evidence_refs: ArtifactRef[];
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Error".
 */
export interface Error {
  code: string;
  message: string;
  correlation_id: string;
  effect_state: "NOT_SENT" | "CONFIRMED" | "UNKNOWN" | "NOT_APPLICABLE";
  retryable: boolean;
  recovery_action:
    | "NONE"
    | "RETRY_READ"
    | "REAUTHENTICATE"
    | "REVIEW_CONFLICT"
    | "RECONCILE"
    | "REQUEST_PERMISSION"
    | "CONTACT_OPERATOR"
    | "REFRESH_SNAPSHOT";
  /**
   * @maxItems 100
   */
  details: {
    field: string;
    reason: string;
  }[];
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "CommandReceipt".
 */
export interface CommandReceipt {
  command_id: string;
  state: "ACCEPTED" | "SUCCEEDED";
  resource: ResourceRef | null;
  run_id: string | null;
  effect_id: string | null;
  correlation_id: string;
  accepted_at: string;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "EventEnvelope".
 */
export interface EventEnvelope {
  event_id: string;
  tenant_id: string;
  type: string;
  schema_version: string;
  aggregate: ResourceRef;
  sequence: number;
  occurred_at: string;
  trace_id: string;
  payload_ref: ArtifactRef;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "ApprovalBinding".
 */
export interface ApprovalBinding {
  proposal_id: string;
  tenant_id: string;
  action: string;
  target: ResourceRef;
  content_digest: string;
  scope_digest: string;
  policy_version: string;
  expires_at: string;
  approved_by: PrincipalRef;
  max_cost_microunits: number;
  currency: string;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "EffectIntent".
 */
export interface EffectIntent {
  effect_id: string;
  tenant_id: string;
  run_id: string;
  action: string;
  connection_id: string;
  target: ResourceRef;
  content_ref: ArtifactRef;
  fingerprint: string;
  idempotency_key: string;
  approval_id: string | null;
  reservation_id: string;
  state:
    | "PREPARED"
    | "WAITING_APPROVAL"
    | "AUTHORIZED"
    | "EXECUTING"
    | "SUCCEEDED"
    | "FAILED_CONFIRMED"
    | "OUTCOME_UNKNOWN"
    | "RECONCILING"
    | "MANUAL_REVIEW"
    | "CANCELLED";
  version: number;
  created_at: string;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "EffectReceipt".
 */
export interface EffectReceipt {
  receipt_id: string;
  effect_id: string;
  outcome: "SUCCEEDED" | "FAILED_CONFIRMED" | "PROVEN_ABSENT";
  remote_id: string | null;
  remote_version: string | null;
  remote_url: string | null;
  evidence_ref: ArtifactRef;
  observed_at: string;
  provider_contract_version: string;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "CostReservation".
 */
export interface CostReservation {
  reservation_id: string;
  tenant_id: string;
  run_id: string;
  currency: string;
  upper_bound_microunits: number;
  settled_microunits: number | null;
  state: "RESERVED" | "SETTLED" | "UNKNOWN" | "RELEASED";
  price_profile: string;
  version: number;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Connection".
 */
export interface Connection {
  connection_id: string;
  tenant_id: string;
  provider: string;
  external_account: string;
  /**
   * @maxItems 100
   */
  scopes: string[];
  secret_ref: string;
  state: "PENDING" | "ACTIVE" | "NEEDS_REAUTH" | "PAUSED" | "REVOKED";
  contract_version: string;
  /**
   * @maxItems 100
   */
  capabilities: string[];
  verified_at: string | null;
  version: number;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "PluginDependency".
 */
export interface PluginDependency {
  plugin_id: string;
  contract_range: string;
  optional: boolean;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "UIContribution".
 */
export interface UIContribution {
  id: string;
  slot: "NAVIGATION" | "ROUTE" | "DASHBOARD" | "ENTITY_TAB" | "EDITOR_PANEL" | "SETTINGS" | "COMMAND";
  path: string;
  ui_contract_version: string;
  trust: "TRUSTED_BUNDLE" | "DECLARATIVE" | "ISOLATED_IFRAME";
  required_action: string;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "PluginManifest".
 */
export interface PluginManifest {
  id: string;
  version: string;
  contract_version: string;
  artifact_digest: string;
  publisher_id: string;
  execution_class: "TRUSTED_NATIVE" | "WASM_COMPONENT" | "REMOTE_WORKER";
  /**
   * @maxItems 100
   */
  requested_capabilities: string[];
  /**
   * @maxItems 100
   */
  dependencies: PluginDependency[];
  /**
   * @maxItems 100
   */
  ui: UIContribution[];
  /**
   * @maxItems 100
   */
  migrations: {
    id: string;
    digest: string;
    compatibility: "ADDITIVE" | "BACKFILL" | "DESTRUCTIVE";
    reversible: boolean;
  }[];
  license_expression: string;
  sbom_digest: string;
  signature_ref: string;
  /**
   * @maxItems 500
   */
  handlers: HandlerContract[];
  /**
   * @maxItems 100
   */
  workflows: ArtifactRef[];
  /**
   * @maxItems 100
   */
  data_contracts: ArtifactRef[];
  /**
   * @maxItems 500
   */
  tools: ToolDescriptor[];
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "HandlerContract".
 */
export interface HandlerContract {
  id: string;
  input_schema_ref: ArtifactRef;
  output_schema_ref: ArtifactRef;
  effects: "READ_ONLY" | "PROPOSES_EFFECT" | "DETERMINISTIC";
  /**
   * @maxItems 100
   */
  required_capabilities: string[];
  max_execution_seconds: number;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "ToolDescriptor".
 */
export interface ToolDescriptor {
  id: string;
  version: string;
  input_schema_ref: ArtifactRef;
  output_schema_ref: ArtifactRef;
  action: string;
  effect_class: "READ_ONLY" | "PROPOSES_EFFECT";
  connection_provider: string | null;
  trust: "FIRST_PARTY" | "UNTRUSTED_METADATA";
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "ProviderProfile".
 */
export interface ProviderProfile {
  id: string;
  provider: string;
  model: string;
  adapter_version: string;
  /**
   * @maxItems 100
   */
  capabilities: string[];
  continuation_mode: "MANUAL_HISTORY" | "PREVIOUS_RESPONSE_ID" | "NONE";
  compaction_mode: "NONE" | "SERVER" | "STANDALONE";
  max_turns: number;
  max_output_tokens: number;
  wall_clock_seconds: number;
  /**
   * @maxItems 4
   */
  allowed_data_classes:
    | []
    | ["PUBLIC" | "INTERNAL" | "CONFIDENTIAL" | "RESTRICTED"]
    | ["PUBLIC" | "INTERNAL" | "CONFIDENTIAL" | "RESTRICTED", "PUBLIC" | "INTERNAL" | "CONFIDENTIAL" | "RESTRICTED"]
    | [
        "PUBLIC" | "INTERNAL" | "CONFIDENTIAL" | "RESTRICTED",
        "PUBLIC" | "INTERNAL" | "CONFIDENTIAL" | "RESTRICTED",
        "PUBLIC" | "INTERNAL" | "CONFIDENTIAL" | "RESTRICTED"
      ]
    | [
        "PUBLIC" | "INTERNAL" | "CONFIDENTIAL" | "RESTRICTED",
        "PUBLIC" | "INTERNAL" | "CONFIDENTIAL" | "RESTRICTED",
        "PUBLIC" | "INTERNAL" | "CONFIDENTIAL" | "RESTRICTED",
        "PUBLIC" | "INTERNAL" | "CONFIDENTIAL" | "RESTRICTED"
      ];
  region_policy: string;
  retention_policy: string;
  price_profile: string;
  live_status: "DISABLED" | "QUALIFICATION" | "APPROVED";
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "ProviderNativeEnvelope".
 */
export interface ProviderNativeEnvelope {
  provider: string;
  adapter_version: string;
  model: string;
  conversation_id: string;
  /**
   * @maxItems 10000
   */
  output: {
    [k: string]: unknown;
  }[];
  continuation_mode: "MANUAL_HISTORY" | "PREVIOUS_RESPONSE_ID";
  previous_response_id: string | null;
  usage_state: "KNOWN" | "UNKNOWN";
  encrypted_store_ref: string;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "CapabilityReport".
 */
export interface CapabilityReport {
  subject_id: string;
  artifact_digest: string;
  contract_version: string;
  environment: "MOCK" | "LOCAL" | "STAGING_LIVE" | "PRODUCTION_LIVE";
  /**
   * @minItems 1
   */
  results: [
    {
      capability: string;
      status: "PASS" | "FAIL" | "NOT_RUN";
      evidence_digest: string | null;
    },
    ...{
      capability: string;
      status: "PASS" | "FAIL" | "NOT_RUN";
      evidence_digest: string | null;
    }[]
  ];
  generated_at: string;
  review_state: "PENDING" | "INDEPENDENTLY_REVIEWED";
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "SessionView".
 */
export interface SessionView {
  authenticated: boolean;
  principal: PrincipalRef | null;
  /**
   * @maxItems 1000
   */
  tenant_ids: string[];
  active_tenant_id: string | null;
  expires_at: string | null;
  step_up_expires_at: string | null;
  csrf_token: string | null;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "SourceRights".
 */
export interface SourceRights {
  rights_id: string;
  access_mode: "FIRST_PARTY" | "AUTHORIZED_API" | "LICENSED" | "PERMITTED_CRAWL" | "MANUAL_IMPORT";
  allow_acquire: boolean;
  allow_ai_analysis: boolean;
  allow_transform: boolean;
  allow_public_redistribution: boolean;
  /**
   * @maxItems 100
   */
  allowed_provider_profiles: string[];
  expires_at: string | null;
  evidence_ref: ArtifactRef;
  version: number;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "ReadCollection".
 */
export interface ReadCollection {
  /**
   * @maxItems 200
   */
  items: ResourceRef[];
  next_cursor: string | null;
  snapshot_at: string;
  data_state: "FRESH" | "STALE" | "PARTIAL" | "NO_DATA";
  total_visible: number | null;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "ResourceProjection".
 */
export interface ResourceProjection {
  resource: ResourceRef;
  artifact_ref: ArtifactRef;
  updated_at: string;
  data_state: "FRESH" | "STALE" | "PARTIAL" | "NO_DATA";
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Run".
 */
export interface Run {
  run_id: string;
  tenant_id: string;
  workflow_id: string;
  workflow_version: string;
  state:
    | "QUEUED"
    | "RUNNING"
    | "WAITING_APPROVAL"
    | "WAITING_RETRY"
    | "BLOCKED"
    | "CANCEL_REQUESTED"
    | "SUCCEEDED"
    | "FAILED"
    | "CANCELLED";
  plugin_digest: string;
  grant_id: string;
  version: number;
  created_at: string;
  updated_at: string;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Membership".
 */
export interface Membership {
  membership_id: string;
  tenant_id: string;
  principal_id: string;
  /**
   * @maxItems 32
   */
  roles: string[];
  state: "INVITED" | "ACTIVE" | "REVOKED";
  version: number;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Delegation".
 */
export interface Delegation {
  grant_id: string;
  tenant_id: string;
  principal: PrincipalRef;
  /**
   * @maxItems 100
   */
  actions: string[];
  /**
   * @maxItems 500
   */
  resources: ResourceRef[];
  expires_at: string;
  parent_grant_id: string | null;
  state: "ACTIVE" | "REVOKED" | "EXPIRED";
  version: number;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "WorkflowNode".
 */
export interface WorkflowNode {
  id: string;
  kind: "DETERMINISTIC" | "ACTIVITY" | "MODEL" | "APPROVAL" | "EFFECT" | "TIMER" | "BOUNDED_LOOP";
  handler_id: string | null;
  input_schema_ref: ArtifactRef;
  output_schema_ref: ArtifactRef;
  max_attempts: number;
  timeout_seconds: number;
  max_iterations: number;
  on_failure: "BLOCK" | "FAIL" | "RETRY_BOUNDED";
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "WorkflowDefinition".
 */
export interface WorkflowDefinition {
  id: string;
  version: string;
  input_schema_ref: ArtifactRef;
  output_schema_ref: ArtifactRef;
  entry_node: string;
  /**
   * @minItems 1
   * @maxItems 500
   */
  nodes: [WorkflowNode, ...WorkflowNode[]];
  /**
   * @maxItems 1000
   */
  edges: {
    from: string;
    to: string;
    condition:
      | "SUCCESS"
      | "FAILURE"
      | "APPROVED"
      | "REJECTED"
      | "TRUE"
      | "FALSE"
      | "TIMER_FIRED"
      | "LOOP_CONTINUE"
      | "LOOP_DONE";
  }[];
  max_total_steps: number;
  max_model_turns: number;
  /**
   * @maxItems 100
   */
  required_actions: string[];
  digest: string;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "ToolInvocation".
 */
export interface ToolInvocation {
  invocation_id: string;
  run_context: RunContext;
  tool_id: string;
  tool_version: string;
  arguments_ref: ArtifactRef;
  connection_id: string | null;
  idempotency_key: string;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "ToolResult".
 */
export interface ToolResult {
  invocation_id: string;
  state: "SUCCEEDED" | "FAILED" | "BLOCKED" | "INCOMPLETE";
  output_ref: ArtifactRef | null;
  error: Error | null;
  proposed_effect_id: string | null;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "EvidenceRunRecord".
 */
export interface EvidenceRunRecord {
  run_id: string;
  test_suite_version: string;
  candidate_digest: string;
  fixture_digest: string;
  config_digest: string;
  environment: "MOCK" | "LOCAL" | "STAGING_LIVE" | "PRODUCTION_LIVE";
  collected_count: number;
  passed_count: number;
  failed_count: number;
  skipped_count: number;
  started_at: string;
  ended_at: string;
  exit_code: number;
  /**
   * @maxItems 1000
   */
  artifact_digests: string[];
  status: "PASS" | "FAIL" | "NOT_RUN" | "CANCELLED" | "ERROR";
  independent_review: "PENDING" | "APPROVED" | "REJECTED";
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "UploadSession".
 */
export interface UploadSession {
  upload_id: string;
  tenant_id: string;
  artifact_id: string;
  upload_path: string;
  expected_size_bytes: number;
  expected_digest: string;
  content_type: string;
  expires_at: string;
  state: "OPEN" | "UPLOADED" | "FINALIZED" | "EXPIRED" | "FAILED";
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "ConnectorAuthorization".
 */
export interface ConnectorAuthorization {
  authorization_id: string;
  tenant_id: string;
  connection_id: string;
  authorization_url: string;
  return_to: string;
  expires_at: string;
  state: "PENDING" | "COMPLETED" | "EXPIRED" | "FAILED";
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_product_compose".
 */
export interface CommandProductCompose {
  product_id: string;
  plugin_lock_ref: ArtifactRef;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_plugin_install".
 */
export interface CommandPluginInstall {
  manifest: PluginManifest;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_plugin_enable".
 */
export interface CommandPluginEnable {
  plugin_id: string;
  artifact_digest: string;
  /**
   * @maxItems 100
   */
  grants: string[];
  expected_version: number;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_plugin_disable".
 */
export interface CommandPluginDisable {
  plugin_id: string;
  expected_version: number;
  reason: string;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_plugin_upgrade".
 */
export interface CommandPluginUpgrade {
  plugin_id: string;
  new_manifest: PluginManifest;
  expected_version: number;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_plugin_revoke".
 */
export interface CommandPluginRevoke {
  plugin_id: string;
  artifact_digest: string;
  reason: string;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_plugin_uninstall".
 */
export interface CommandPluginUninstall {
  plugin_id: string;
  expected_version: number;
  data_policy: "RETAIN" | "EXPORT_THEN_RETAIN";
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_plugin_invoke".
 */
export interface CommandPluginInvoke {
  plugin_id: string;
  handler: string;
  input_ref: ArtifactRef;
  grant_id: string;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_identity_configure".
 */
export interface CommandIdentityConfigure {
  issuer: string;
  client_id: string;
  client_secret_ref: string;
  /**
   * @maxItems 10
   */
  allowed_redirect_uris:
    | []
    | [string]
    | [string, string]
    | [string, string, string]
    | [string, string, string, string]
    | [string, string, string, string, string]
    | [string, string, string, string, string, string]
    | [string, string, string, string, string, string, string]
    | [string, string, string, string, string, string, string, string]
    | [string, string, string, string, string, string, string, string, string]
    | [string, string, string, string, string, string, string, string, string, string];
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_membership_invite".
 */
export interface CommandMembershipInvite {
  email: string;
  /**
   * @maxItems 10
   */
  roles:
    | []
    | [string]
    | [string, string]
    | [string, string, string]
    | [string, string, string, string]
    | [string, string, string, string, string]
    | [string, string, string, string, string, string]
    | [string, string, string, string, string, string, string]
    | [string, string, string, string, string, string, string, string]
    | [string, string, string, string, string, string, string, string, string]
    | [string, string, string, string, string, string, string, string, string, string];
  expires_at: string;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_membership_accept".
 */
export interface CommandMembershipAccept {
  invite_token: string;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_membership_change".
 */
export interface CommandMembershipChange {
  membership_id: string;
  /**
   * @maxItems 10
   */
  roles:
    | []
    | [string]
    | [string, string]
    | [string, string, string]
    | [string, string, string, string]
    | [string, string, string, string, string]
    | [string, string, string, string, string, string]
    | [string, string, string, string, string, string, string]
    | [string, string, string, string, string, string, string, string]
    | [string, string, string, string, string, string, string, string, string]
    | [string, string, string, string, string, string, string, string, string, string];
  expected_version: number;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_membership_revoke".
 */
export interface CommandMembershipRevoke {
  membership_id: string;
  expected_version: number;
  reason: string;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_policy_evaluate".
 */
export interface CommandPolicyEvaluate {
  action: string;
  resource: ResourceRef;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_policy_propose".
 */
export interface CommandPolicyPropose {
  policy_ref: ArtifactRef;
  expected_version: number;
  review_ref: ArtifactRef;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_grant_create".
 */
export interface CommandGrantCreate {
  delegate: PrincipalRef;
  /**
   * @maxItems 100
   */
  actions: string[];
  /**
   * @maxItems 500
   */
  resources: ResourceRef[];
  expires_at: string;
  parent_grant_id: string | null;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_grant_revoke".
 */
export interface CommandGrantRevoke {
  grant_id: string;
  expected_version: number;
  reason: string;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_artifact_begin".
 */
export interface CommandArtifactBegin {
  classification: "PUBLIC" | "INTERNAL" | "CONFIDENTIAL" | "RESTRICTED";
  content_type: string;
  size_bytes: number;
  expected_digest: string;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_artifact_finalize".
 */
export interface CommandArtifactFinalize {
  artifact_id: string;
  observed_digest: string;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_deletion_request".
 */
export interface CommandDeletionRequest {
  resource: ResourceRef;
  reason: string;
  confirmation: string;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_run_start".
 */
export interface CommandRunStart {
  workflow_id: string;
  workflow_version: string;
  input_ref: ArtifactRef;
  grant_id: string;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_run_cancel".
 */
export interface CommandRunCancel {
  run_id: string;
  expected_version: number;
  reason: string;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_approval_decide".
 */
export interface CommandApprovalDecide {
  proposal_id: string;
  decision: "APPROVE" | "REJECT" | "CANCEL";
  expected_version: number;
  content_digest: string;
  reason: string;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_effect_propose".
 */
export interface CommandEffectPropose {
  action: string;
  connection_id: string;
  target: ResourceRef;
  content_ref: ArtifactRef;
  grant_id: string;
  price_profile: string;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_effect_dispatch".
 */
export interface CommandEffectDispatch {
  effect_id: string;
  expected_version: number;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_effect_reconcile".
 */
export interface CommandEffectReconcile {
  effect_id: string;
  expected_version: number;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_effect_compensate".
 */
export interface CommandEffectCompensate {
  effect_id: string;
  compensation_ref: ArtifactRef;
  approval_id: string;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_budget_configure".
 */
export interface CommandBudgetConfigure {
  currency: string;
  limit_microunits: number;
  period: "RUN" | "DAY" | "MONTH";
  expected_version: number;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_provider_configure".
 */
export interface CommandProviderConfigure {
  profile: ProviderProfile;
  credential_ref: string;
  qualification_ref: ArtifactRef | null;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_provider_compact".
 */
export interface CommandProviderCompact {
  conversation_id: string;
  profile_id: string;
  expected_version: number;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_connection_authorize".
 */
export interface CommandConnectionAuthorize {
  provider: string;
  /**
   * @maxItems 100
   */
  requested_scopes: string[];
  return_to: string;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_connection_revoke".
 */
export interface CommandConnectionRevoke {
  connection_id: string;
  expected_version: number;
  reason: string;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_schedule_create".
 */
export interface CommandScheduleCreate {
  workflow_id: string;
  input_ref: ArtifactRef;
  grant_id: string;
  cron: string;
  timezone: string;
  overlap: "SKIP" | "COALESCE_LATEST";
  dst_gap: "SKIP";
  dst_fold: "RUN_ONCE";
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_deadletter_replay".
 */
export interface CommandDeadletterReplay {
  event_id: string;
  reason: string;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_ui_register".
 */
export interface CommandUiRegister {
  plugin_id: string;
  /**
   * @maxItems 100
   */
  contributions: UIContribution[];
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_export_create".
 */
export interface CommandExportCreate {
  resource_type: string;
  format: "JSON" | "CSV";
  filter_ref: ArtifactRef;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_notification_read".
 */
export interface CommandNotificationRead {
  notification_id: string;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_support_request".
 */
export interface CommandSupportRequest {
  /**
   * @maxItems 100
   */
  actions: string[];
  expires_at: string;
  reason: string;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_kill_switch_set".
 */
export interface CommandKillSwitchSet {
  scope: "TENANT" | "PLUGIN" | "CONNECTION";
  target_id: string;
  active: boolean;
  reason: string;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_release_qualify".
 */
export interface CommandReleaseQualify {
  artifact_digest: string;
  evidence_ref: ArtifactRef;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_catalog_submit".
 */
export interface CommandCatalogSubmit {
  manifest: PluginManifest;
  support_contact: string;
  description: string;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_catalog_review".
 */
export interface CommandCatalogReview {
  listing_id: string;
  decision: "APPROVE" | "REJECT";
  evidence_ref: ArtifactRef;
  expected_version: number;
}
/**
 * This interface was referenced by `UrnMasonwingContracts100`'s JSON-Schema
 * via the `definition` "Command_conformance_run".
 */
export interface CommandConformanceRun {
  artifact_digest: string;
  suite_version: string;
  profile: "MOCK" | "LOCAL" | "STAGING_LIVE";
}
