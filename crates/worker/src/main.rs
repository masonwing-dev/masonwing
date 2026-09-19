//! Masonwing durable worker composition root.
//!
//! Two responsibilities run side by side:
//! 1. A health/status HTTP surface (`masonwing-host-api` scaffold router) so
//!    the compose healthcheck can observe liveness.
//! 2. The outbox→durable-engine relay that claims committed `run.start`
//!    events, re-verifies authority, and starts Temporal workflows under
//!    their stable identity.
//!
//! The relay adapter is UNQUALIFIED and this binary refuses to run in a live
//! environment.

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // The scaffold activation guards (external mutations disabled, live budget
    // zero) and the live-environment refusal are enforced by the shared
    // runtime config before anything is dispatched or bound.
    let runtime = masonwing_host_api::RuntimeConfig::from_env("masonwing-worker")?;
    let config = masonwing_worker::RelayConfig::from_env()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let environment = runtime.environment;

    // Spawn the relay in the background; the health surface stays on the
    // foreground so `axum::serve` can drive graceful shutdown.
    let relay_config = config.clone();
    let relay_environment = environment;
    let relay =
        tokio::spawn(
            async move { masonwing_worker::run_relay(relay_config, relay_environment).await },
        );

    // Reuse the scaffold router for health checks. Any HTTP command endpoint
    // on this service stays protected as the scaffold requires.
    let app = masonwing_host_api::router("masonwing-worker", environment);
    let listener = tokio::net::TcpListener::bind(runtime.bind_addr).await?;
    let server = axum::serve(listener, app);
    tokio::select! {
        result = server => { result?; }
        result = relay => {
            match result {
                Ok(Err(e)) => return Err(e.into()),
                Err(join) => return Err(join.into()),
                Ok(Ok(())) => {}
            }
        }
    }
    Ok(())
}
