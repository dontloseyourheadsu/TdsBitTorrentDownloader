use tracker::server::TrackerServer;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let server = TrackerServer::new(6969);
    server.start().await?;
    Ok(())
}
