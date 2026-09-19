use crate::store::{Mutation, PostgresStore, Tx};
use masonwing_kernel::runtime::{AuthorizedCommand, CommandFailure};

impl PostgresStore {
    pub(crate) async fn dispatch(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        match command.operation.as_str() {
            "product.compose" => self.compose_product(tx, command).await,
            "plugin.install" => self.install_plugin(tx, command).await,
            "plugin.enable" => self.enable_plugin(tx, command).await,
            "plugin.disable" => self.disable_plugin(tx, command).await,
            "plugin.upgrade" => self.upgrade_plugin(tx, command).await,
            "plugin.revoke" => self.revoke_plugin(tx, command).await,
            "plugin.uninstall" => self.uninstall_plugin(tx, command).await,
            "plugin.invoke" => self.invoke_plugin(tx, command).await,
            "run.start" => self.start_run(tx, command).await,
            "run.cancel" => self.cancel_run(tx, command).await,
            "approval.decide" => self.decide_approval(tx, command).await,
            "budget.configure" => self.configure_budget(tx, command).await,
            "effect.propose" => self.propose_effect(tx, command).await,
            "effect.dispatch" => self.dispatch_effect(tx, command).await,
            "effect.reconcile" => self.reconcile_effect(tx, command).await,
            "effect.compensate" => self.compensate_effect(tx, command).await,
            "provider.compact" => self.compact_provider(tx, command).await,
            "connection.authorize" => self.authorize_connection(tx, command).await,
            "connection.revoke" => self.revoke_connection(tx, command).await,
            "release.qualify" => self.qualify_release(tx, command).await,
            "catalog.submit" => self.submit_catalog(tx, command).await,
            "catalog.review" => self.review_catalog(tx, command).await,
            "conformance.run" => self.run_conformance(tx, command).await,
            "schedule.create" => self.create_schedule(tx, command).await,
            "deadletter.replay" => self.replay_deadletter(tx, command).await,
            "ui.register" => self.register_ui(tx, command).await,
            "export.create" => self.create_export(tx, command).await,
            "kill-switch.set" => self.set_kill_switch(tx, command).await,
            "notification.read" => self.mark_notification_read(tx, command).await,
            "support.request" => self.request_support(tx, command).await,
            "provider.configure" => self.configure_provider(tx, command).await,
            "grant.create" => self.create_grant(tx, command).await,
            "grant.revoke" => self.revoke_grant(tx, command).await,
            "artifact.begin" => self.begin_artifact(tx, command).await,
            "artifact.finalize" => self.finalize_artifact(tx, command).await,
            "deletion.request" => self.request_deletion(tx, command).await,
            "membership.invite" => {
                self.invite_membership(
                    tx,
                    command,
                    self.invite_key
                        .as_ref()
                        .ok_or_else(|| CommandFailure::unavailable("INVITE_KEY_UNAVAILABLE"))?,
                )
                .await
            }
            "membership.change" => self.change_membership(tx, command).await,
            "membership.revoke" => self.revoke_membership(tx, command).await,
            "policy.evaluate" => self.evaluate_policy(tx, command).await,
            "policy.propose" => self.propose_policy(tx, command).await,
            "identity.configure" => self.configure_identity(tx, command).await,
            _ => Err(CommandFailure::unavailable("COMMAND_HANDLER_UNAVAILABLE")),
        }
    }
}
