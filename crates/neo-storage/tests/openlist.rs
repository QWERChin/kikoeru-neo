//! The openlist backend against a fake OpenList. The fake came from
//! openlist-client's tests when that crate moved to its own repository.

#[path = "fake_openlist/mod.rs"]
mod fake;

use fake::{Fault, dir, file, start};
use futures_util::StreamExt;
use neo_storage::{Access, EntryKind, OpenListStorage, RawPath, Storage, StorageError};

fn raw(path: &str) -> RawPath {
    RawPath::new(path.as_bytes().to_vec()).unwrap()
}

async fn library() -> (fake::Server, OpenListStorage) {
    let server = start().await;
    server.fake.put("/media", vec![dir("voice")]);
    server
        .fake
        .put("/media/voice", vec![dir("RJ01"), file("notes.txt", 3)]);
    server
        .fake
        .put("/media/voice/RJ01", vec![dir("mp3"), file("cover.jpg", 4)]);
    server
        .fake
        .put("/media/voice/RJ01/mp3", vec![file("01.mp3", 10)]);
    let storage = OpenListStorage::new(server.client(), "/media/voice/").unwrap();
    (server, storage)
}

#[tokio::test]
async fn list_and_stat_map_paths_below_the_base() {
    let (_server, storage) = library().await;
    let top = storage.list(&RawPath::root()).await.unwrap();
    assert_eq!(top.len(), 2);
    assert_eq!(top[0].name, b"RJ01");
    assert_eq!(top[0].kind, EntryKind::Dir);
    assert_eq!(top[1].size, Some(3));
    assert!(top[1].mtime_ms.is_some());

    let entry = storage.stat(&raw("RJ01/mp3/01.mp3")).await.unwrap();
    assert_eq!(
        (entry.name.as_slice(), entry.size),
        (&b"01.mp3"[..], Some(10))
    );
    assert_eq!(
        storage.stat(&raw("RJ01/none.mp3")).await,
        Err(StorageError::NotFound)
    );
}

#[tokio::test]
async fn walk_returns_paths_relative_to_the_walked_directory() {
    let (_server, storage) = library().await;
    let found = storage.walk(&raw("RJ01")).await.unwrap();
    let paths: Vec<String> = found.iter().map(|w| w.path.to_string_lossy()).collect();
    assert_eq!(paths, ["cover.jpg", "mp3", "mp3/01.mp3"]);
}

#[tokio::test]
async fn a_missing_base_path_is_unavailable_never_empty_or_missing() {
    let (server, _) = library().await;
    let storage = OpenListStorage::new(server.client(), "/media/unmounted").unwrap();
    assert!(matches!(
        storage.list(&RawPath::root()).await,
        Err(StorageError::Unavailable(_))
    ));
    assert!(matches!(
        storage.walk(&RawPath::root()).await,
        Err(StorageError::Unavailable(_))
    ));
}

#[tokio::test]
async fn an_empty_root_is_unavailable_for_list_and_walk() {
    for base in ["/media/voice", "/"] {
        let server = start().await;
        server.fake.put(base, vec![]);
        let storage = OpenListStorage::new(server.client(), base).unwrap();
        let expected = StorageError::Unavailable(format!(
            "{base} is empty; is the OpenList base path correct?"
        ));
        assert_eq!(storage.list(&RawPath::root()).await, Err(expected.clone()));
        assert_eq!(storage.walk(&RawPath::root()).await, Err(expected));
    }
}

#[tokio::test]
async fn an_empty_subdirectory_is_normal_for_list_and_walk() {
    let server = start().await;
    server.fake.put("/media", vec![dir("empty")]);
    server.fake.put("/media/empty", vec![]);
    let storage = OpenListStorage::new(server.client(), "/media").unwrap();
    assert!(storage.list(&raw("empty")).await.unwrap().is_empty());
    assert!(storage.walk(&raw("empty")).await.unwrap().is_empty());
    let entries = storage.list(&RawPath::root()).await.unwrap();
    assert_eq!(entries.len(), 1);
    let tree = storage.walk(&RawPath::root()).await.unwrap();
    assert_eq!(tree.len(), 1);
    assert_eq!(tree[0].path, raw("empty"));
}

#[tokio::test]
async fn a_missing_work_directory_is_not_found() {
    let (_server, storage) = library().await;
    assert_eq!(
        storage.walk(&raw("RJ99")).await,
        Err(StorageError::NotFound)
    );
}

#[tokio::test]
async fn a_subdirectory_that_vanishes_mid_walk_makes_the_walk_unavailable() {
    let (server, storage) = library().await;
    for _ in 0..3 {
        server.fake.fault(
            "list",
            "/media/voice/RJ01/mp3",
            Fault::Code(500, "failed get objs: object not found"),
        );
    }
    let result = storage.walk(&raw("RJ01")).await;
    assert!(
        matches!(result, Err(StorageError::Unavailable(_))),
        "{result:?}"
    );
}

#[tokio::test]
async fn backslashes_never_reach_openlist() {
    let (server, storage) = library().await;
    let result = storage.stat(&raw("RJ01\\..\\..\\secret")).await;
    assert!(
        matches!(result, Err(StorageError::Upstream(_))),
        "{result:?}"
    );
    assert_eq!(server.fake.hits("list", "/media/voice/RJ01\\..\\.."), 0);
}

#[tokio::test]
async fn access_opens_the_file_through_the_client_with_range() {
    let (_server, storage) = library().await;
    let Access::Remote(remote) = storage.access(&raw("RJ01/mp3/01.mp3")).await.unwrap() else {
        panic!("an OpenList file is remote");
    };
    assert_eq!(remote.size, Some(10));
    let mut headers = http::HeaderMap::new();
    headers.insert(http::header::RANGE, "bytes=3-".parse().unwrap());
    let response = remote.opener.open(&headers).await.unwrap();
    assert_eq!(response.status, 206);
    let body: Vec<u8> = response
        .body
        .map(|chunk| chunk.unwrap().to_vec())
        .concat()
        .await;
    assert_eq!(body, b"tents of /media/voice/RJ01/mp3/01.mp3");
}

#[tokio::test]
async fn a_directory_cannot_be_opened() {
    let (_server, storage) = library().await;
    assert!(matches!(
        storage.access(&raw("RJ01/mp3")).await,
        Err(StorageError::NotFound)
    ));
}

#[tokio::test]
async fn a_link_refused_even_after_refreshing_is_an_upstream_error() {
    let (server, storage) = library().await;
    let Access::Remote(remote) = storage.access(&raw("RJ01/mp3/01.mp3")).await.unwrap() else {
        panic!("an OpenList file is remote");
    };
    *server.fake.reject_files.lock().unwrap() = 2;
    let result = remote.opener.open(&http::HeaderMap::new()).await;
    assert!(
        matches!(result, Err(StorageError::Upstream(_))),
        "{:?}",
        result.err()
    );
}

#[tokio::test]
async fn files_below_a_vanished_base_are_unavailable_not_missing() {
    let (server, _) = library().await;
    let storage = OpenListStorage::new(server.client(), "/media/unmounted").unwrap();
    for path in ["RJ01", "RJ01/mp3/01.mp3"] {
        let result = storage.stat(&raw(path)).await;
        assert!(
            matches!(result, Err(StorageError::Unavailable(_))),
            "{path}: {result:?}"
        );
    }
    assert!(matches!(
        storage.walk(&raw("RJ01")).await,
        Err(StorageError::Unavailable(_))
    ));
}

#[tokio::test]
async fn a_link_missing_under_a_vanished_base_is_unavailable() {
    let (server, storage) = library().await;
    let Access::Remote(remote) = storage.access(&raw("RJ01/mp3/01.mp3")).await.unwrap() else {
        panic!("an OpenList file is remote");
    };
    // The base vanishes after the listing was cached.
    server.fake.put("/media/voice/RJ01/mp3", vec![]);
    server.fake.remove("/media/voice");
    let result = remote.opener.open(&http::HeaderMap::new()).await;
    assert!(
        matches!(result, Err(StorageError::Unavailable(_))),
        "{:?}",
        result.err()
    );
}

#[tokio::test]
async fn a_work_directory_openlist_cannot_find_is_asked_once_and_missing() {
    // openlist-client 0.2 takes "object not found" as an answer and no
    // longer retries it. The base is still there, so the work is missing,
    // which a later scan undoes with the same IDs if it comes back.
    let (server, storage) = library().await;
    server.fake.fault(
        "list",
        "/media/voice/RJ01",
        Fault::Code(500, "failed get objs: object not found"),
    );
    assert_eq!(
        storage.walk(&raw("RJ01")).await,
        Err(StorageError::NotFound)
    );
    assert_eq!(server.fake.hits("list", "/media/voice/RJ01"), 1);
}

#[tokio::test]
async fn throttling_past_the_budget_is_unavailable_not_missing() {
    let server = start().await;
    server.fake.put("/media/voice", vec![dir("RJ01")]);
    server
        .fake
        .put("/media/voice/RJ01", vec![file("01.mp3", 10)]);
    let mut config = server.config();
    config.cooldown_budget = std::time::Duration::from_millis(50);
    let storage = OpenListStorage::new(
        openlist_client::Client::new(config).unwrap(),
        "/media/voice",
    )
    .unwrap();
    for _ in 0..10 {
        server
            .fake
            .fault("list", "/media/voice/RJ01", Fault::Status(429, Some("1")));
    }
    let result = storage.walk(&raw("RJ01")).await;
    assert!(
        matches!(result, Err(StorageError::Unavailable(_))),
        "{result:?}"
    );
}

#[tokio::test]
async fn a_link_to_another_service_on_this_machine_is_not_fetched() {
    // A hostile OpenList could point a link at a local admin port, and the
    // proxy would hand its answer to the player.
    let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = std::sync::Arc::clone(&hits);
    let service = axum::Router::new().fallback(move || {
        counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        async { "secret" }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, service).await.unwrap() });

    let (server, storage) = library().await;
    *server.fake.link_base.lock().unwrap() = Some(format!("http://localhost:{port}"));
    let Access::Remote(remote) = storage.access(&raw("RJ01/mp3/01.mp3")).await.unwrap() else {
        panic!("an OpenList file is remote");
    };
    assert!(remote.opener.open(&http::HeaderMap::new()).await.is_err());
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0);
}
