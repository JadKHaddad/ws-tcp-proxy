use std::{
    net::SocketAddr,
    pin::pin,
    sync::{Arc, atomic::AtomicU32},
};

use axum::{
    Router,
    extract::{Query, State, WebSocketUpgrade, ws::Message},
    response::IntoResponse,
    routing::get,
};
use bytes::Bytes;
use clap::Parser;
use hickory_resolver::TokioResolver;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::{TcpListener, TcpStream},
};
use tracing_subscriber::EnvFilter;
use ws_tcp_proxy::{Args, MaybeTlsStream, WebSocketExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("ws_tcp_proxy=info"));

    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    let args = Args::parse();

    let addr = args.socket_addr;

    let state = Arc::new(AppState::new()?);
    let app = Router::new().route("/ws", get(ws)).with_state(state);

    tracing::info!(%addr, "listening");

    let listener = TcpListener::bind(addr).await?;

    axum::serve(listener, app).await?;

    Ok(())
}

async fn ws(
    params: Query<QueryParams>,
    upgrade: WebSocketUpgrade,
    state: State<Arc<AppState>>,
) -> impl IntoResponse {
    let id = state.next_connection_id();

    tracing::info!(id, params=?params.0, "New WebSocket connection");

    let socket = state
        .connect(id, params.domain.as_str(), params.port, params.ssl)
        .await
        .inspect_err(|err| {
            tracing::error!(%err, "Failed to connect to upstream TCP server");
        });

    upgrade.on_upgrade(move |mut ws| async move {
        match socket {
            Ok(socket) => {
                let bytes = serde_json::to_vec(&UpstreamResult::ok()).unwrap_or_default();

                if let Err(err) = ws.send(Message::Binary(Bytes::from(bytes))).await {
                    tracing::error!(%err, "Failed to send upstream result");
                    return;
                }

                proxy(id, ws.into_async_read_write(), socket).await;
            }
            Err(err) => {
                let bytes =
                    serde_json::to_vec(&UpstreamResult::err(err.to_string())).unwrap_or_default();

                if let Err(err) = ws.send(Message::Binary(Bytes::from(bytes))).await {
                    tracing::error!(%err, "Failed to send upstream result");
                }
            }
        }
    })
}

#[tracing::instrument(skip(a, b))]
async fn proxy(id: u32, a: impl AsyncRead + AsyncWrite, b: impl AsyncRead + AsyncWrite) {
    tracing::info!("Proxying");

    let mut a = pin!(a);
    let mut b = pin!(b);

    if let Err(err) = tokio::io::copy_bidirectional(&mut a, &mut b).await {
        tracing::error!(%err, "IO error during proxying");
    }

    tracing::info!("Connection closed");
}

struct AppState {
    connection_id: AtomicU32,
    resolver: TokioResolver,
}

impl AppState {
    fn new() -> Result<Self, anyhow::Error> {
        Ok(Self {
            connection_id: AtomicU32::new(0),
            resolver: TokioResolver::builder_tokio()?.build(),
        })
    }

    fn next_connection_id(&self) -> u32 {
        self.connection_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    #[tracing::instrument(skip(self))]
    async fn connect(
        &self,
        id: u32,
        domain: &str,
        port: u16,
        ssl: bool,
    ) -> Result<MaybeTlsStream<TcpStream>, anyhow::Error> {
        tracing::info!("Connecting to upstream TCP server");

        tracing::debug!("Resolving domain");

        let ip_addr = self
            .resolver
            .lookup_ip(domain)
            .await?
            .into_iter()
            .next()
            .ok_or(anyhow::Error::msg("No addresses found for the given host"))?;

        let addr = SocketAddr::new(ip_addr, port);

        tracing::debug!(%addr, "Connecting to address");

        let stream = TcpStream::connect(addr).await?;

        let stream = match ssl {
            true => MaybeTlsStream::rustls(stream, domain).await?,
            false => MaybeTlsStream::plain(stream),
        };

        Ok(stream)
    }
}

#[derive(Debug, serde::Deserialize)]
struct QueryParams {
    domain: String,
    port: u16,
    ssl: bool,
}

#[derive(Debug, serde::Serialize)]
struct UpstreamResult {
    error: Option<String>,
}

impl UpstreamResult {
    fn ok() -> Self {
        Self { error: None }
    }

    fn err(err: String) -> Self {
        Self { error: Some(err) }
    }
}
