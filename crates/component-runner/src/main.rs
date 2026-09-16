#[tokio::main]
async fn main() -> Result<(), masonwing_host_api::StartupError> {
    masonwing_host_api::run_service("masonwing-component-runner").await
}
