#[tokio::main]
async fn main() -> anyhow::Result<()> {
    memehub::run().await
}
