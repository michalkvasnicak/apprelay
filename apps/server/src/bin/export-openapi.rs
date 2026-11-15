fn main() -> Result<(), Box<dyn std::error::Error>> {
    serde_json::to_writer_pretty(std::io::stdout(), &apprelay_server::http::document())?;
    Ok(())
}
