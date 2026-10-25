//! The webdav backend against the fake WebDAV server from webdav-client.

#[path = "../../webdav-client/tests/common/mod.rs"]
mod fake;

use futures_util::StreamExt;
use neo_storage::{Access, EntryKind, RawPath, Storage, StorageError, WebDavStorage};

fn raw(path: &str) -> RawPath {
    RawPath::new(path.as_bytes().to_vec()).unwrap()
}

async fn library() -> (fake::Server, WebDavStorage) {
    let server = fake::start().await;
    let fake = &server.fake;
    fake.dir("/media");
    fake.dir("/media/voice");
    fake.dir("/media/voice/RJ01");
    fake.dir("/media/voice/RJ01/mp3");
    fake.file("/media/voice/RJ01/mp3/01.mp3", b"0123456789");
    fake.file("/media/voice/RJ01/cover.jpg", b"jpeg");
    fake.file("/media/voice/notes.txt", b"abc");
    let storage = WebDavStorage::new(server.client(), "/media/voice/").unwrap();
    (server, storage)
}

#[tokio::test]
async fn list_stat_and_walk_map_paths_below_the_base() {
    let (_server, storage) = library().await;
    let mut top = storage.list(&RawPath::root()).await.unwrap();
    top.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(top[0].name, b"RJ01");
    assert_eq!(top[0].kind, EntryKind::Dir);
    assert!(top[0].etag.is_some(), "directory ETags join the digest");
    assert_eq!(top[1].size, Some(3));

    let entry = storage.stat(&raw("RJ01/mp3/01.mp3")).await.unwrap();
    assert_eq!(
        (entry.name.as_slice(), entry.size),
        (&b"01.mp3"[..], Some(10))
    );
    let mut paths: Vec<String> = storage
        .walk(&raw("RJ01"))
        .await
        .unwrap()
        .iter()
        .map(|w| w.path.to_string_lossy())
        .collect();
    paths.sort();
    assert_eq!(paths, ["cover.jpg", "mp3", "mp3/01.mp3"]);
}

#[tokio::test]
async fn control_characters_in_directory_names_do_not_escape_the_base() {
    let server = fake::start_at("").await;
    server.fake.dir("/media");
    server.fake.dir("/RJ999999");
    server.fake.file("/RJ999999/01.mp3", b"outside the library");
    for name in [".\t.", ".\r.", ".\n."] {
        server.fake.dir(&format!("/media/{name}"));
        server.fake.dir(&format!("/media/{name}/RJ123456"));
    }
    let storage = WebDavStorage::new(server.client(), "/media").unwrap();
    let entries = storage.list(&RawPath::root()).await.unwrap();
    assert_eq!(entries.len(), 3);
    for entry in entries {
        let path = RawPath::root().join(&entry.name).unwrap();
        let children = storage.list(&path).await.unwrap();
        let names: Vec<_> = children.iter().map(|child| child.name.as_slice()).collect();
        assert_eq!(names, [b"RJ123456".as_slice()], "{path:?}");
    }
    let log = server.fake.log.lock().unwrap();
    assert!(
        log.iter()
            .all(|request| request.starts_with("PROPFIND /media")),
        "{log:?}"
    );
}

#[tokio::test]
async fn a_missing_file_is_missing_only_while_the_base_is_there() {
    let (server, storage) = library().await;
    assert_eq!(
        storage.stat(&raw("RJ01/none.mp3")).await,
        Err(StorageError::NotFound)
    );
    // The share is unmounted: every path vanishes, which is an outage.
    server.fake.remove("/media/voice");
    assert!(matches!(
        storage.stat(&raw("RJ01/cover.jpg")).await,
        Err(StorageError::Unavailable(_))
    ));
    assert!(matches!(
        storage.list(&RawPath::root()).await,
        Err(StorageError::Unavailable(_))
    ));
}

#[tokio::test]
async fn files_are_read_through_the_server_with_ranges() {
    let (_server, storage) = library().await;
    let Access::Remote(remote) = storage.access(&raw("RJ01/mp3/01.mp3")).await.unwrap() else {
        panic!("a remote file");
    };
    assert_eq!(remote.size, Some(10));
    let mut headers = http::HeaderMap::new();
    headers.insert(http::header::RANGE, "bytes=7-".parse().unwrap());
    let response = remote.opener.open(&headers).await.unwrap();
    assert_eq!(response.status, 206);
    let body: Vec<u8> = response
        .body
        .map(|chunk| chunk.unwrap().to_vec())
        .concat()
        .await;
    assert_eq!(body, b"789");
}

#[tokio::test]
async fn control_characters_in_file_names_are_not_stripped_or_double_encoded() {
    let server = fake::start_at("").await;
    server.fake.dir("/media");
    server.fake.file("/media/ab.mp3", b"wrong file");
    let files = [
        ("a\tb.mp3", b"tab".as_slice()),
        ("a\rb.mp3", b"carriage return".as_slice()),
        ("a\nb.mp3", b"line feed".as_slice()),
        ("a%09b.mp3", b"literal percent escape".as_slice()),
    ];
    for (name, body) in files {
        server.fake.file(&format!("/media/{name}"), body);
    }
    let storage = WebDavStorage::new(server.client(), "/media").unwrap();
    for (name, expected) in files {
        let Access::Remote(remote) = storage.access(&raw(name)).await.unwrap() else {
            panic!("a remote file");
        };
        let response = remote.opener.open(&http::HeaderMap::new()).await.unwrap();
        let body: Vec<u8> = response
            .body
            .map(|chunk| chunk.unwrap().to_vec())
            .concat()
            .await;
        assert_eq!(body, expected, "{name:?}");
        assert_eq!(remote.size, Some(expected.len() as u64), "{name:?}");
    }
}

#[tokio::test]
async fn backslashes_never_reach_the_server() {
    let (server, storage) = library().await;
    let before = server.fake.requests("PROPFIND");
    assert!(matches!(
        storage.stat(&raw("RJ01\\..\\..\\secret")).await,
        Err(StorageError::Upstream(_))
    ));
    assert_eq!(server.fake.requests("PROPFIND"), before);
}
