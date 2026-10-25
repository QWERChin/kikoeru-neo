//! Metadata requests through the proxy from the settings page, and changing
//! it while the sources are in use. The sources point at `.invalid` hosts,
//! which never resolve, so a request only gets an answer through the fake
//! proxy. Nothing here touches the network.

use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Json, Router,
    extract::{Request, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use neo_scanner::metadata::{
    AsmrOneConfig, AsmrOneProvider, DlsiteConfig, DlsiteProvider, HvdbConfig, HvdbProvider,
    MetadataError, MetadataProvider, ProviderChain, ProxyChoice, ProxyUrl,
};
use serde_json::json;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use url::Url;

/// The authority of each request the fake received, as `host:port` or
/// `host`, which tells a proxied request from a direct one.
type Seen = Arc<Mutex<Vec<String>>>;

/// A server that answers like DLsite, asmr.one and hvdb at once, by path.
/// An HTTP proxy request carries the full URL, which axum routes by its
/// path like any other, so the fake is also the proxy. `refuse` lists hosts
/// it answers 502 for, as a proxy that cannot reach them would.
async fn fake(refuse: &'static [&'static str]) -> (SocketAddr, Seen) {
    let seen = Seen::default();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new()
        .fallback(answer)
        .with_state((seen.clone(), refuse));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, seen)
}

async fn answer(
    State((seen, refuse)): State<(Seen, &'static [&'static str])>,
    request: Request,
) -> Response {
    let host = request
        .uri()
        .host()
        .or_else(|| {
            request
                .headers()
                .get(header::HOST)
                .and_then(|h| h.to_str().ok())
                .map(|h| h.split(':').next().unwrap_or(h))
        })
        .unwrap_or_default()
        .to_owned();
    seen.lock().unwrap().push(host.clone());
    if refuse.contains(&host.as_str()) {
        return StatusCode::BAD_GATEWAY.into_response();
    }
    let path = request.uri().path().to_owned();
    match path.as_str() {
        "/" => (StatusCode::FOUND, [(header::LOCATION, "/home")]).into_response(),
        "/maniax/api/=/product.json" => Json(json!([{
            "product_name": "Proxied title",
            "maker_id": "RG100",
            "maker_name": "Circle",
            "age_category": 1,
            "image_main": {"url": "http://dlsite.invalid/img/main.jpg"},
        }]))
        .into_response(),
        "/maniax/product/info/ajax" => Json(json!({})).into_response(),
        "/img/main.jpg" => (&b"jpeg"[..]).into_response(),
        "/api/workInfo/1" => StatusCode::NOT_FOUND.into_response(),
        "/api/workInfo/1000001" => Json(json!({
            "title": format!("From {host}"),
            "source_id": "RJ01000001",
        }))
        .into_response(),
        "/Dashboard/Details/1000001" => "<html><title>HVDB - RJ01000001</title>\
             <a href=\"/Dashboard/CVWorks/1\">声優</a></html>"
            .into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

/// A SOCKS5 server that tunnels every connection to `target`, whatever
/// address the client asked for, and records the host names it was asked
/// for. Only what reqwest sends is handled: no authentication, CONNECT.
async fn socks5(target: SocketAddr) -> (SocketAddr, Seen) {
    let asked = Seen::default();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let log = asked.clone();
    tokio::spawn(async move {
        loop {
            let (client, _) = listener.accept().await.unwrap();
            let log = log.clone();
            tokio::spawn(async move {
                let _ = tunnel(client, target, log).await;
            });
        }
    });
    (addr, asked)
}

async fn tunnel(mut client: TcpStream, target: SocketAddr, log: Seen) -> std::io::Result<()> {
    let mut greeting = [0; 2];
    client.read_exact(&mut greeting).await?;
    let mut methods = vec![0; usize::from(greeting[1])];
    client.read_exact(&mut methods).await?;
    client.write_all(&[5, 0]).await?;
    let mut request = [0; 4];
    client.read_exact(&mut request).await?;
    let host = match request[3] {
        1 => {
            let mut ip = [0; 4];
            client.read_exact(&mut ip).await?;
            std::net::Ipv4Addr::from(ip).to_string()
        }
        3 => {
            let mut len = [0; 1];
            client.read_exact(&mut len).await?;
            let mut name = vec![0; usize::from(len[0])];
            client.read_exact(&mut name).await?;
            String::from_utf8_lossy(&name).into_owned()
        }
        _ => return Ok(()),
    };
    let mut port = [0; 2];
    client.read_exact(&mut port).await?;
    log.lock().unwrap().push(host);
    let mut upstream = TcpStream::connect(target).await?;
    client.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0]).await?;
    tokio::io::copy_bidirectional(&mut client, &mut upstream).await?;
    Ok(())
}

fn through(scheme: &str, addr: SocketAddr) -> ProxyChoice {
    let url = ProxyUrl::parse(&format!("{scheme}://{addr}"))
        .unwrap()
        .unwrap();
    ProxyChoice::Fixed(url)
}

fn dlsite(proxy: ProxyChoice) -> DlsiteProvider {
    DlsiteProvider::new(DlsiteConfig {
        base: "http://dlsite.invalid/".parse().unwrap(),
        spacing: Duration::ZERO,
        retries: 0,
        timeout: Duration::from_secs(5),
        proxy,
        ..DlsiteConfig::default()
    })
    .unwrap()
}

#[tokio::test]
async fn fetches_metadata_and_covers_through_the_proxy() {
    let (proxy, seen) = fake(&[]).await;
    let provider = dlsite(through("http", proxy));

    let meta = provider.fetch("RJ01000001").await.unwrap();
    assert_eq!(meta.title, "Proxied title");
    let cover = provider
        .cover(meta.cover_url.as_ref().unwrap())
        .await
        .unwrap();
    assert_eq!(cover, b"jpeg");

    // product.json, the product info and the cover, each for DLsite's host.
    assert_eq!(*seen.lock().unwrap(), ["dlsite.invalid"; 3]);
}

#[tokio::test]
async fn a_new_proxy_applies_to_the_next_request() {
    let (proxy, seen) = fake(&[]).await;
    // Without a proxy the host does not resolve, whatever the environment
    // of the test run holds.
    let provider = dlsite(ProxyChoice::Environment);
    let chain = ProviderChain::new(Arc::new(provider), None);
    assert!(matches!(
        chain.fetch("RJ01000001").await,
        Err(MetadataError::Unavailable(_))
    ));

    chain.set_proxy(&through("http", proxy)).await.unwrap();
    assert_eq!(
        chain.fetch("RJ01000001").await.unwrap().title,
        "Proxied title"
    );

    chain.set_proxy(&ProxyChoice::Environment).await.unwrap();
    assert!(chain.fetch("RJ01000001").await.is_err());
    assert_eq!(
        seen.lock().unwrap().len(),
        2,
        "only the proxied fetch arrived"
    );
}

#[tokio::test]
async fn socks5h_hands_the_host_name_to_the_proxy() {
    let (server, seen) = fake(&[]).await;
    let (proxy, asked) = socks5(server).await;
    let provider = dlsite(through("socks5h", proxy));

    assert_eq!(
        provider.fetch("RJ01000001").await.unwrap().title,
        "Proxied title"
    );

    assert!(asked.lock().unwrap().iter().all(|h| h == "dlsite.invalid"));
    assert!(!seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn hvdb_follows_the_chain_to_a_new_proxy() {
    let (proxy, _) = fake(&[]).await;
    let primary = DlsiteProvider::new(DlsiteConfig {
        base: "http://dlsite.invalid/".parse().unwrap(),
        spacing: Duration::ZERO,
        retries: 0,
        ..DlsiteConfig::default()
    })
    .unwrap();
    let hvdb = HvdbProvider::new(HvdbConfig {
        base: "http://hvdb.invalid/".parse().unwrap(),
        spacing: Duration::ZERO,
        retries: 0,
        ..HvdbConfig::default()
    })
    .unwrap();
    let chain = ProviderChain::new(Arc::new(primary), None).with_voice_actors(Arc::new(hvdb));

    chain.set_proxy(&through("http", proxy)).await.unwrap();

    // The fake's DLsite answer has no voice actors, so hvdb is asked too.
    assert_eq!(chain.fetch("RJ01000001").await.unwrap().vas, ["声優"]);
}

#[tokio::test]
async fn a_new_proxy_probes_the_asmr_one_mirrors_again() {
    // The first proxy reaches both mirrors. The second cannot reach the
    // first mirror, which the provider had already chosen.
    let (first, _) = fake(&[]).await;
    let (second, _) = fake(&["one.invalid"]).await;
    let provider = AsmrOneProvider::new(AsmrOneConfig {
        mirrors: vec![
            "http://one.invalid/".parse().unwrap(),
            "http://two.invalid/".parse().unwrap(),
        ],
        spacing: Duration::ZERO,
        retries: 0,
        proxy: through("http", first),
        ..AsmrOneConfig::default()
    })
    .unwrap();
    assert_eq!(
        provider.fetch("RJ01000001").await.unwrap().title,
        "From one.invalid"
    );

    provider.set_proxy(&through("http", second)).await.unwrap();

    assert_eq!(
        provider.fetch("RJ01000001").await.unwrap().title,
        "From two.invalid"
    );
}

#[tokio::test]
async fn check_reports_the_answer_without_following_it() {
    let (proxy, seen) = fake(&[]).await;
    // The provider's own proxy is not the one checked.
    let provider = dlsite(ProxyChoice::Environment);

    let reached = provider.check(&through("http", proxy)).await.unwrap();

    assert_eq!(reached.url, Url::parse("http://dlsite.invalid/").unwrap());
    assert_eq!(reached.status, 302);
    assert_eq!(*seen.lock().unwrap(), ["dlsite.invalid"]);
    // The check left the provider's own client alone.
    assert!(provider.fetch("RJ01000001").await.is_err());
}

#[tokio::test]
async fn check_says_why_nothing_answered() {
    // A port nothing listens on, as a proxy that is not running.
    let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = closed.local_addr().unwrap();
    drop(closed);
    let provider = dlsite(ProxyChoice::Environment);

    let Err(MetadataError::Unavailable(reason)) = provider.check(&through("http", addr)).await
    else {
        panic!("the check got an answer");
    };

    // reqwest's own message only says the request failed. The cause
    // follows it.
    assert!(reason.to_lowercase().contains("refused"), "{reason}");
}
