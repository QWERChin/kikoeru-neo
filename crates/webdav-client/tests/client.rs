//! The client against the fake WebDAV server in `common`.

mod common;

use common::{Fault, start};
use futures_util::StreamExt;
use http::{HeaderMap, HeaderValue, header::RANGE};
use webdav_client::{Client, Config, ErrorKind};

async fn library() -> common::Server {
    let server = start().await;
    let fake = &server.fake;
    fake.dir("/音声");
    fake.dir("/音声/RJ01");
    fake.file("/音声/RJ01/01 a&b #1.mp3", b"0123456789");
    fake.file("/音声/notes.txt", b"abc");
    server
}

#[tokio::test]
async fn a_listing_holds_the_direct_children_with_their_properties() {
    let server = library().await;
    let mut entries = server.client().list("/音声").await.unwrap();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["RJ01", "notes.txt"]);
    assert!(entries[0].is_dir);
    assert_eq!(entries[0].size, None);
    assert!(entries[0].etag.is_some());
    assert_eq!(entries[1].size, Some(3));
    assert!(entries[1].modified.is_some());

    let inner = server.client().list("/音声/RJ01").await.unwrap();
    assert_eq!(inner[0].name, "01 a&b #1.mp3");
    assert_eq!(server.fake.requests("PROPFIND"), 2);
}

#[tokio::test]
async fn stat_describes_files_and_collections_and_reports_missing_paths() {
    let server = library().await;
    let client = server.client();
    let file = client.stat("/音声/RJ01/01 a&b #1.mp3").await.unwrap();
    assert_eq!((file.is_dir, file.size), (false, Some(10)));
    assert!(client.stat("/音声/RJ01").await.unwrap().is_dir);
    let missing = client.stat("/音声/none.mp3").await.unwrap_err();
    assert_eq!(missing.kind(), ErrorKind::NotFound);
}

#[tokio::test]
async fn ranges_are_forwarded_and_the_answer_returned_as_is() {
    let server = library().await;
    let mut headers = HeaderMap::new();
    headers.insert(RANGE, HeaderValue::from_static("bytes=2-5"));
    let transfer = server
        .client()
        .open("/音声/RJ01/01 a&b #1.mp3", &headers)
        .await
        .unwrap();
    assert_eq!(transfer.status, 206);
    let body: Vec<u8> = transfer
        .body
        .map(|chunk| chunk.unwrap().to_vec())
        .concat()
        .await;
    assert_eq!(body, b"2345");
    let missing = server
        .client()
        .open("/音声/none.mp3", &HeaderMap::new())
        .await
        .err()
        .unwrap();
    assert_eq!(missing.kind(), ErrorKind::NotFound);
}

#[tokio::test]
async fn wrong_credentials_and_server_errors_are_unavailable_not_missing() {
    let server = library().await;
    let mut config = server.config();
    config.password = "wrong".into();
    let err = Client::new(config)
        .unwrap()
        .list("/音声")
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unavailable);

    server.fake.fault(
        "/音声",
        Fault::Status {
            status: 500,
            times: 10,
            retry_after: None,
        },
    );
    let err = server.client().list("/音声").await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unavailable);
    // The refused login, then three attempts at the failing listing.
    assert_eq!(server.fake.requests("PROPFIND"), 1 + 3);
}

#[tokio::test]
async fn throttling_is_waited_out_within_the_budget() {
    let server = library().await;
    server.fake.fault(
        "/音声",
        Fault::Status {
            status: 503,
            times: 1,
            retry_after: Some(1),
        },
    );
    let entries = server.client().list("/音声").await.unwrap();
    assert_eq!(entries.len(), 2);

    server.fake.fault(
        "/音声",
        Fault::Status {
            status: 429,
            times: 1,
            retry_after: Some(3600),
        },
    );
    let err = server.client().list("/音声").await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unavailable);
}

#[tokio::test]
async fn redirects_stay_on_the_origin() {
    let server = library().await;
    server.fake.dir("/moved");
    server.fake.fault(
        "/old",
        Fault::Redirect {
            location: format!("{}/moved/", server.url()),
        },
    );
    assert!(server.client().list("/old").await.unwrap().is_empty());

    server.fake.fault(
        "/away",
        Fault::Redirect {
            location: "http://203.0.113.1/dav/".into(),
        },
    );
    let err = server.client().list("/away").await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Upstream);
}

#[tokio::test]
async fn listings_that_point_outside_or_are_not_multistatus_fail() {
    let server = library().await;
    let outside = r#"<D:multistatus xmlns:D="DAV:">
        <D:response><D:href>/dav/%E9%9F%B3%E5%A3%B0/</D:href>
          <D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop>
          <D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>
        <D:response><D:href>/dav/other/secret.txt</D:href>
          <D:propstat><D:prop><D:resourcetype/></D:prop>
          <D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>
      </D:multistatus>"#;
    server.fake.fault("/音声", Fault::Body(outside.into()));
    let err = server.client().list("/音声").await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Upstream);

    // The collection itself refused in its own response: an outage.
    let refused = r#"<D:multistatus xmlns:D="DAV:">
        <D:response><D:href>/dav/%E9%9F%B3%E5%A3%B0/</D:href>
          <D:status>HTTP/1.1 403 Forbidden</D:status></D:response>
      </D:multistatus>"#;
    server.fake.fault("/音声", Fault::Body(refused.into()));
    let err = server.client().list("/音声").await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unavailable);

    server
        .fake
        .fault("/音声", Fault::Body("<html>please log in</html>".into()));
    let err = server.client().list("/音声").await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Upstream);
}

#[tokio::test]
async fn successful_properties_do_not_mask_failed_propstats() {
    let server = library().await;
    for (status, kind) in [
        (403, ErrorKind::Unavailable),
        (500, ErrorKind::Unavailable),
        (409, ErrorKind::Upstream),
    ] {
        for success_first in [false, true] {
            let failed = format!(
                "<D:propstat><D:prop><D:resourcetype/></D:prop>\
                 <D:status>HTTP/1.1 {status} Error</D:status></D:propstat>"
            );
            let success = "<D:propstat><D:prop><D:getetag>etag</D:getetag></D:prop>\
                           <D:status>HTTP/1.1 200 OK</D:status></D:propstat>";
            let props = if success_first {
                format!("{success}{failed}")
            } else {
                format!("{failed}{success}")
            };
            for href in ["/dav/media/", "/dav/media/RJ123456/"] {
                server.fake.fault(
                    "/media",
                    Fault::Body(format!(
                        "<D:multistatus xmlns:D=\"DAV:\">\
                         <D:response><D:href>{href}</D:href>{props}</D:response>\
                         </D:multistatus>"
                    )),
                );
                let err = server.client().list("/media").await.unwrap_err();
                assert_eq!(err.kind(), kind, "{status}, {success_first}, {href}");
                assert!(matches!(err, webdav_client::Error::Http { status: s, .. } if s == status));
                if href == "/dav/media/" {
                    let err = server.client().stat("/media").await.unwrap_err();
                    assert_eq!(err.kind(), kind, "stat: {status}, {success_first}");
                }
            }
        }
    }
}

#[test]
fn urls_with_credentials_or_queries_are_refused() {
    for url in [
        "ftp://nas.lan/dav",
        "https://u:p@nas.lan/dav",
        "https://nas.lan/dav?x=1",
    ] {
        assert!(
            Client::new(Config::new(url.parse().unwrap(), "u", "p")).is_err(),
            "{url}"
        );
    }
}
