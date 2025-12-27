use std::{
    pin::Pin,
    task::{Context, Poll},
};

use axum::extract::ws::{Message, WebSocket};
use bytes::{Buf, Bytes};
use futures::{Sink, SinkExt, Stream, StreamExt, TryStreamExt, future};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_util::io::{CopyToBytes, StreamReader};

pub trait WebSocketExt {
    fn into_async_read_write(self) -> impl AsyncRead + AsyncWrite;
}

impl WebSocketExt for WebSocket {
    fn into_async_read_write(self) -> impl AsyncRead + AsyncWrite {
        let stream = self
            .filter_map(|msg| async {
                match msg {
                    Err(err) => Some(Err(err)),
                    Ok(Message::Binary(bytes)) => Some(Ok(bytes)),
                    _ => None,
                }
            })
            .map_err(std::io::Error::other)
            .with(|bytes: Bytes| future::ready(Ok::<Message, axum::Error>(Message::Binary(bytes))))
            .sink_map_err(std::io::Error::other);

        StreamSinkBytesAsyncReadWrite::new(StreamReader::new(CopyToBytes::new(stream)))
    }
}

pin_project_lite::pin_project! {
    /// An adapter that implements both [`AsyncRead`] and [`AsyncWrite`] for a combination of a
    /// [`Stream`] and a [`Sink`] of [`Bytes`].
    struct StreamSinkBytesAsyncReadWrite<S, B> {
        #[pin]
        inner: StreamReader<S, B>,
    }
}

impl<S, B> StreamSinkBytesAsyncReadWrite<S, B> {
    const fn new(inner: StreamReader<S, B>) -> Self {
        Self { inner }
    }
}

impl<S, B, E> AsyncRead for StreamSinkBytesAsyncReadWrite<S, B>
where
    S: Stream<Item = Result<B, E>>,
    B: Buf,
    E: Into<std::io::Error>,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.project().inner.poll_read(cx, buf)
    }
}

impl<S, B, E> AsyncWrite for StreamSinkBytesAsyncReadWrite<S, B>
where
    for<'a> S: Sink<&'a [u8], Error = E>,
    E: Into<std::io::Error>,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        let mut this = self.project();

        futures::ready!(this.inner.as_mut().poll_ready(cx).map_err(Into::into))?;
        match this.inner.as_mut().start_send(buf) {
            Ok(()) => Poll::Ready(Ok(buf.len())),
            Err(e) => Poll::Ready(Err(e.into())),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
        self.project().inner.poll_flush(cx).map_err(Into::into)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        self.project().inner.poll_close(cx).map_err(Into::into)
    }
}
