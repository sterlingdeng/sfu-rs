use sfu_rs::sfu::sfu;
use sfu_rs::signal::websocket::server;

use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();

    let sfu = sfu::Sfu::new();
    let server = server::Server::new("127.0.0.1:8080", sfu.clone());

    match server.listen_and_serve().await {
        Ok(_) => Ok(()),
        Err(e) => Err(e.into()),
    }
}
