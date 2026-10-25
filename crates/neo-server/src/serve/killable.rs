//! A listener whose connections can all be closed at once.
//!
//! axum runs every connection on its own detached task, so a graceful
//! shutdown that gives up on a stuck connection cannot reach it. Each
//! connection here fails its next read or write once the shared token is
//! cancelled, which makes hyper drop it and close the socket.

use std::{
    future::Future,
    io,
    net::SocketAddr,
    pin::Pin,
    task::{Context, Poll},
};

use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
};
use tokio_util::sync::{CancellationToken, WaitForCancellationFutureOwned};

pub(super) struct KillableListener {
    inner: TcpListener,
    kill: CancellationToken,
}

impl KillableListener {
    pub(super) fn new(inner: TcpListener, kill: CancellationToken) -> Self {
        Self { inner, kill }
    }
}

impl axum::serve::Listener for KillableListener {
    type Io = KillableStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        let (stream, addr) = axum::serve::Listener::accept(&mut self.inner).await;
        let stream = KillableStream {
            inner: stream,
            killed: Box::pin(self.kill.clone().cancelled_owned()),
        };
        (stream, addr)
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.inner.local_addr()
    }
}

pub(super) struct KillableStream {
    inner: TcpStream,
    killed: Pin<Box<WaitForCancellationFutureOwned>>,
}

impl KillableStream {
    /// Polling the token registers the connection's waker, so a connection
    /// blocked on a full socket buffer wakes up when the token fires.
    fn check(&mut self, cx: &mut Context<'_>) -> io::Result<()> {
        match self.killed.as_mut().poll(cx) {
            Poll::Ready(()) => Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "the server is shutting down",
            )),
            Poll::Pending => Ok(()),
        }
    }
}

impl AsyncRead for KillableStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.check(cx)?;
        Pin::new(&mut this.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for KillableStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        this.check(cx)?;
        Pin::new(&mut this.inner).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        this.check(cx)?;
        Pin::new(&mut this.inner).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.check(cx)?;
        Pin::new(&mut this.inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}
