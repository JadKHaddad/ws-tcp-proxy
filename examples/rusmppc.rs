//!
//!
//! ```not_rust
//! cargo run --example rusmppc
//! ```

use std::time::Duration;

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use rusmpp::pdus::BindTransceiver;
use rusmppc::ConnectionBuilder;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tokio_util::io::{CopyToBytes, SinkWriter, StreamReader};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("rusmppc=info")
        .init();

    let url = "ws://proxy.rusmpp.org:7777/ws?ssl=true&domain=rusmpps.rusmpp.org&port=2776";

    // or over tls
    // let url = "wss://proxy.rusmpp.org:7776/ws?ssl=true&domain=rusmpps.rusmpp.org&port=2776";

    tracing::info!(%url, "Connecting to ws-tcp-proxy server");

    let (ws, _) = connect_async(url).await?;

    let (sink, stream) = ws.split();

    let stream = stream.filter_map(|msg| async move {
        match msg {
            Err(err) => Some(Err(std::io::Error::other(err))),
            Ok(Message::Binary(bytes)) => Some(Ok(bytes)),
            _ => None,
        }
    });

    let sink = sink
        .with(|bytes: Bytes| async move {
            Ok::<Message, tokio_tungstenite::tungstenite::Error>(Message::Binary(bytes))
        })
        .sink_map_err(std::io::Error::other);

    let stream = StreamReader::new(stream);
    let sink = SinkWriter::new(CopyToBytes::new(sink));

    let io = tokio::io::join(stream, sink);

    let (client, mut events) = ConnectionBuilder::new()
        .enquire_link_interval(Duration::from_secs(5))
        .response_timeout(Duration::from_secs(2))
        .events()
        .insights()
        .connected(io);

    tokio::spawn(async move {
        while let Some(event) = events.next().await {
            tracing::info!(?event, "Event");
        }

        tracing::info!("Connection closed");
    });

    tracing::info!("Binding transceiver");

    let response = client.bind_transceiver(BindTransceiver::default()).await?;

    tracing::info!(?response, "Bind transceiver response");

    tokio::time::sleep(Duration::from_secs(10)).await;

    tracing::info!("Shutting down");

    client.unbind().await?;
    client.close().await?;
    client.closed().await;

    Ok(())
}
