#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    tokio::signal::ctrl_c().await
}
