//! The local backend against a real temporary directory.

use std::{
    ffi::OsStr,
    fs,
    io::Read,
    os::unix::{ffi::OsStrExt, fs::PermissionsExt},
    path::Path,
};

use neo_storage::{Access, EntryKind, LocalStorage, RawPath, Source, Storage, StorageError};

fn p(s: &str) -> RawPath {
    RawPath::new(s).unwrap()
}

/// root/
///   RJ01000001/
///     mp3/01.mp3   (5 bytes)
///     cover.jpg    (3 bytes)
///     <GBK name>.mp3
///   link -> RJ01000001
///   outside -> /  (a link that would leave the root)
fn library(dir: &Path) -> std::path::PathBuf {
    let root = dir.join("root");
    let work = root.join("RJ01000001");
    fs::create_dir_all(work.join("mp3")).unwrap();
    fs::write(work.join("mp3/01.mp3"), b"audio").unwrap();
    fs::write(work.join("cover.jpg"), b"jpg").unwrap();
    fs::write(
        work.join(OsStr::from_bytes(b"\xb2\xe2\xca\xd4.mp3")),
        b"gbk",
    )
    .unwrap();
    std::os::unix::fs::symlink(&work, root.join("link")).unwrap();
    std::os::unix::fs::symlink("/", root.join("outside")).unwrap();
    root
}

#[tokio::test]
async fn lists_sorted_entries_without_following_links() {
    let dir = tempfile::tempdir().unwrap();
    let s = LocalStorage::new(library(dir.path()));

    let top = s.list(&RawPath::root()).await.unwrap();
    let kinds: Vec<_> = top.iter().map(|e| (e.name.as_slice(), e.kind)).collect();
    assert_eq!(
        kinds,
        [
            (&b"RJ01000001"[..], EntryKind::Dir),
            (&b"link"[..], EntryKind::Symlink),
            (&b"outside"[..], EntryKind::Symlink),
        ]
    );

    let work = s.list(&p("RJ01000001")).await.unwrap();
    let cover = work.iter().find(|e| e.name == b"cover.jpg").unwrap();
    assert_eq!(cover.size, Some(3));
    assert!(cover.mtime_ms.is_some_and(|t| t > 0));
    assert!(work.iter().any(|e| e.name == b"\xb2\xe2\xca\xd4.mp3"));
    assert_eq!(
        work.iter().find(|e| e.name == b"mp3").unwrap().size,
        None,
        "directories have no size"
    );

    // Paths through a symlink do not exist as far as the backend is concerned.
    assert_eq!(s.list(&p("link")).await, Err(StorageError::NotFound));
    assert_eq!(s.list(&p("outside/etc")).await, Err(StorageError::NotFound));
    assert_eq!(
        s.stat(&p("link/cover.jpg")).await,
        Err(StorageError::NotFound)
    );
    assert!(matches!(
        s.source(&p("outside/etc/hostname")).await,
        Err(StorageError::NotFound)
    ));
}

#[tokio::test]
async fn walks_the_whole_tree_and_opens_raw_names() {
    let dir = tempfile::tempdir().unwrap();
    let root = library(dir.path());
    let s = LocalStorage::new(&root);

    let paths: Vec<_> = s
        .walk(&p("RJ01000001"))
        .await
        .unwrap()
        .into_iter()
        .map(|w| (w.path.into_bytes(), w.entry.kind))
        .collect();
    assert_eq!(
        paths,
        [
            (b"cover.jpg".to_vec(), EntryKind::File),
            (b"mp3".to_vec(), EntryKind::Dir),
            (b"\xb2\xe2\xca\xd4.mp3".to_vec(), EntryKind::File),
            (b"mp3/01.mp3".to_vec(), EntryKind::File),
        ]
    );

    let gbk = RawPath::new(b"RJ01000001/\xb2\xe2\xca\xd4.mp3".to_vec()).unwrap();
    let Ok(Source::LocalFile(file)) = s.source(&gbk).await else {
        panic!("expected a local file");
    };
    let mut bytes = Vec::new();
    (&*file.file).read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"gbk");
    assert_eq!(file.size, 3);
    let Ok(Access::ServeLocal(file)) = s.access(&p("RJ01000001/mp3/01.mp3")).await else {
        panic!("expected a local file");
    };
    assert!(file.path.starts_with(&root));
    assert!(file.mtime_ms.is_some());

    assert!(matches!(
        s.source(&p("RJ01000001/mp3")).await,
        Err(StorageError::NotFound)
    ));
    assert!(matches!(
        s.access(&p("RJ01000001/none.mp3")).await,
        Err(StorageError::NotFound)
    ));
    assert_eq!(
        s.stat(&p("RJ01000001/mp3/01.mp3")).await.unwrap().size,
        Some(5)
    );
}

#[tokio::test]
async fn a_missing_root_is_unavailable_not_empty() {
    let dir = tempfile::tempdir().unwrap();
    let s = LocalStorage::new(dir.path().join("not-mounted"));
    assert!(matches!(
        s.list(&RawPath::root()).await,
        Err(StorageError::Unavailable(_))
    ));
    assert!(matches!(
        s.stat(&p("RJ01000001")).await,
        Err(StorageError::Unavailable(_))
    ));
    assert!(matches!(
        s.walk(&RawPath::root()).await,
        Err(StorageError::Unavailable(_))
    ));

    // An unmounted mount point is an empty directory, not an empty library.
    let empty = dir.path().join("mountpoint");
    fs::create_dir(&empty).unwrap();
    let s = LocalStorage::new(&empty);
    assert!(matches!(
        s.list(&RawPath::root()).await,
        Err(StorageError::Unavailable(_))
    ));
    assert!(matches!(
        s.stat(&p("RJ01000001/01.mp3")).await,
        Err(StorageError::Unavailable(_))
    ));
    assert!(matches!(
        s.source(&p("RJ01000001/01.mp3")).await,
        Err(StorageError::Unavailable(_))
    ));

    let file_root = dir.path().join("file");
    fs::write(&file_root, "").unwrap();
    let s = LocalStorage::new(&file_root);
    assert!(matches!(
        s.list(&RawPath::root()).await,
        Err(StorageError::Unavailable(_))
    ));
}

#[tokio::test]
async fn an_unreadable_directory_fails_the_walk() {
    let dir = tempfile::tempdir().unwrap();
    let root = library(dir.path());
    let locked = root.join("RJ01000001/mp3");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read_dir(&locked).is_ok() {
        // Running as root: permissions cannot lock the directory.
        return;
    }
    let s = LocalStorage::new(&root);
    let result = s.walk(&p("RJ01000001")).await;
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        matches!(result, Err(StorageError::Unavailable(_))),
        "{result:?}"
    );
}

#[tokio::test]
async fn special_files_are_not_opened() {
    let dir = tempfile::tempdir().unwrap();
    let root = library(dir.path());
    let fifo = root.join("RJ01000001/pipe");
    let status = std::process::Command::new("mkfifo").arg(&fifo).status();
    if !status.is_ok_and(|s| s.success()) {
        return;
    }
    let s = LocalStorage::new(&root);
    // A writer waiting on the FIFO must stay blocked: opening it for
    // reading, even non-blocking, would let the writer through.
    let writer_path = fifo.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let opened = fs::OpenOptions::new()
            .write(true)
            .open(&writer_path)
            .is_ok();
        let _ = tx.send(opened);
    });
    assert!(matches!(
        s.source(&p("RJ01000001/pipe")).await,
        Err(StorageError::NotFound)
    ));
    assert!(
        rx.recv_timeout(std::time::Duration::from_millis(300))
            .is_err(),
        "the FIFO was opened for reading"
    );
    // Release the writer thread.
    drop(fs::File::open(&fifo));
    let listed = s.list(&p("RJ01000001")).await.unwrap();
    assert_eq!(
        listed.iter().find(|e| e.name == b"pipe").unwrap().kind,
        EntryKind::Other
    );
}

#[tokio::test]
async fn shift_jis_names_with_a_backslash_byte_are_kept() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    fs::create_dir_all(root.join("RJ01000001")).unwrap();
    // "ソ.mp3" in Shift_JIS: its second byte is 0x5C, a backslash.
    let name = b"\x83\x5c.mp3";
    fs::write(
        root.join("RJ01000001").join(OsStr::from_bytes(name)),
        b"sjis",
    )
    .unwrap();
    let s = LocalStorage::new(&root);
    let tree = s.walk(&p("RJ01000001")).await.unwrap();
    assert_eq!(tree[0].path.as_bytes(), name);
    let mut path = b"RJ01000001/".to_vec();
    path.extend_from_slice(name);
    let Ok(Source::LocalFile(file)) = s.source(&RawPath::new(path).unwrap()).await else {
        panic!("expected a local file");
    };
    assert_eq!(file.size, 4);
}
