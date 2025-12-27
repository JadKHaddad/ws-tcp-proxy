use std::net::SocketAddr;

use clap::Parser;

#[derive(Debug, Parser)]
pub struct Args {
    /// Socket address to listen on
    #[clap(long, default_value_t = SocketAddr::from(([0, 0, 0, 0], 7777)))]
    pub socket_addr: SocketAddr,
}
