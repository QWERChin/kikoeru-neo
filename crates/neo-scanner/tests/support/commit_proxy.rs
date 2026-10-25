use std::{io, time::Duration};

use neo_db::Db;
use sqlx::postgres::{PgPoolOptions, PgSslMode};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpStream, UnixListener, UnixStream},
    task::JoinHandle,
};

#[derive(Clone, Copy)]
pub enum Disconnect {
    BeforeCommit,
    AfterCommit,
}

/// Only the writer uses the proxy; assertions and startup recovery use a
/// separate connection to the real, disposable PostgreSQL database.
pub struct CommitProxy {
    pub db: Db,
    task: JoinHandle<io::Result<()>>,
    _dir: tempfile::TempDir,
}

impl CommitProxy {
    pub async fn start(db: &Db, disconnect: Disconnect) -> Self {
        let mut db = db.clone();
        let Db::Postgres(pg) = &mut db else {
            panic!("the commit proxy needs PostgreSQL");
        };
        let options = pg.write.connect_options().as_ref().clone();
        let dir = tempfile::tempdir().unwrap();
        let listener =
            UnixListener::bind(dir.path().join(format!(".s.PGSQL.{}", options.get_port())))
                .unwrap();
        let proxy_options = options
            .clone()
            .socket(dir.path())
            .ssl_mode(PgSslMode::Disable);
        let task = tokio::spawn(async move {
            tokio::time::timeout(Duration::from_secs(15), async {
                let (client, _) = listener.accept().await?;
                if let Some(socket) = options.get_socket() {
                    let server = UnixStream::connect(
                        socket.join(format!(".s.PGSQL.{}", options.get_port())),
                    )
                    .await?;
                    relay(client, server, disconnect).await
                } else {
                    let server =
                        TcpStream::connect((options.get_host(), options.get_port())).await?;
                    relay(client, server, disconnect).await
                }
            })
            .await?
        });
        let proxy = Self {
            db,
            task,
            _dir: dir,
        };
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(5))
            .connect_with(proxy_options)
            .await
            .unwrap();
        let mut proxy = proxy;
        let Db::Postgres(pg) = &mut proxy.db else {
            unreachable!()
        };
        pg.write = pool;
        proxy
    }

    pub async fn assert_disconnected(&mut self) {
        (&mut self.task).await.unwrap().unwrap();
    }
}

impl Drop for CommitProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn body(reader: &mut (impl AsyncRead + Unpin)) -> io::Result<Vec<u8>> {
    let size = reader.read_u32().await?;
    let mut body = vec![0; (size - 4) as usize];
    reader.read_exact(&mut body).await?;
    Ok(body)
}

async fn send(writer: &mut (impl AsyncWrite + Unpin), body: &[u8]) -> io::Result<()> {
    writer.write_u32(body.len() as u32 + 4).await?;
    writer.write_all(body).await
}

async fn relay(
    mut client: UnixStream,
    mut server: impl AsyncRead + AsyncWrite + Unpin,
    disconnect: Disconnect,
) -> io::Result<()> {
    // The startup packet has no message tag. TLS is disabled on the proxy
    // connection so all later packets use the ordinary PostgreSQL framing.
    send(&mut server, &body(&mut client).await?).await?;
    let (mut client_read, mut client_write) = tokio::io::split(client);
    let (mut server_read, mut server_write) = tokio::io::split(&mut server);
    tokio::select! {
        result = async {
            loop {
                let tag = client_read.read_u8().await?;
                let message = body(&mut client_read).await?;
                if matches!(disconnect, Disconnect::BeforeCommit)
                    && tag == b'Q' && message == b"COMMIT\0"
                {
                    return Ok(());
                }
                server_write.write_u8(tag).await?;
                send(&mut server_write, &message).await?;
            }
        } => result,
        result = async {
            loop {
                let tag = server_read.read_u8().await?;
                let message = body(&mut server_read).await?;
                if matches!(disconnect, Disconnect::AfterCommit)
                    && tag == b'C' && message == b"COMMIT\0"
                {
                    // PostgreSQL has committed, but SQLx receives neither
                    // CommandComplete nor ReadyForQuery for this COMMIT.
                    return Ok(());
                }
                client_write.write_u8(tag).await?;
                send(&mut client_write, &message).await?;
            }
        } => result,
    }
}
