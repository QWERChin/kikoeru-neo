use neo_core::dto::*;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

fn sample<T: Serialize + DeserializeOwned>(name: &str, dto: T) {
    sample_in("m1", name, dto);
}

fn sample_in<T: Serialize + DeserializeOwned>(milestone: &str, name: &str, dto: T) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../contract")
        .join(milestone)
        .join(name);
    let expected: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(serde_json::to_value(dto).unwrap(), expected);
    let decoded: T = serde_json::from_value(expected.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), expected);
}

fn integer(value: i64) -> SafeInteger {
    value.try_into().unwrap()
}

fn id(value: i64) -> Id {
    value.try_into().unwrap()
}

fn user() -> User {
    User {
        id: id(1),
        name: "admin".into(),
        is_admin: true,
    }
}

#[test]
fn login_request() {
    sample(
        "login-request.json",
        LoginRequest {
            name: "admin".into(),
            password: "example-password".into(),
        },
    );
}

#[test]
fn login_response() {
    sample(
        "login-response.json",
        LoginResponse {
            token: "eyJhbGciOiJIUzI1NiJ9.example.signature".into(),
            expires_at: integer(1794227000000),
            user: user(),
        },
    );
}

#[test]
fn me() {
    sample("me.json", user());
}

#[test]
fn media_token() {
    sample(
        "media-token.json",
        MediaTokenResponse {
            token: "eyJhbGciOiJIUzI1NiJ9.media.signature".into(),
            expires_at: integer(1791656600000),
        },
    );
}

#[test]
fn password_request() {
    sample(
        "password-request.json",
        PasswordRequest {
            current_password: "example-password".into(),
            new_password: "a-longer-new-password".into(),
        },
    );
}

#[test]
fn version() {
    sample(
        "version.json",
        VersionResponse {
            version: "0.1.0".into(),
            git_commit: Some("25ce8c8".into()),
            build_time: Some(integer(1791635000000)),
        },
    );
}

#[test]
fn error_unauthorized() {
    sample(
        "error-unauthorized.json",
        ErrorResponse {
            error: ApiError {
                code: ErrorCode::Unauthorized,
                message: "登录已失效，请重新登录".into(),
            },
        },
    );
}

#[test]
fn error_setup_required() {
    sample(
        "error-setup-required.json",
        ErrorResponse {
            error: ApiError {
                code: ErrorCode::SetupRequired,
                message:
                    "还没有管理员账号，请在服务器上运行 kikoeru-neo user create --admin <名字>"
                        .into(),
            },
        },
    );
}

#[test]
fn works_page() {
    sample(
        "works-page.json",
        Page {
            items: vec![
                WorkSummary {
                    id: id(12),
                    code: "RJ01234567".into(),
                    title: "示例作品一".into(),
                    circle: Some(CircleRef {
                        id: id(3),
                        name: "示例社团".into(),
                    }),
                    release_date: Some("2024-05-01".into()),
                    nsfw: false,
                    rate_average: Some(4.62),
                    rate_count: Some(318),
                    has_cover: true,
                    cover_version: integer(1),
                    missing: false,
                    favorite: true,
                    progress: Some(Progress {
                        track_id: Some(id(345)),
                        position_ms: integer(812000),
                        track_duration_ms: Some(integer(1995000)),
                        finished: true,
                        replaying: true,
                        resumable: true,
                        group_path: Some("mp3/SE有".into()),
                    }),
                    my_rating: Some(5),
                },
                WorkSummary {
                    id: id(13),
                    code: "RJ00123456".into(),
                    title: "示例作品二".into(),
                    circle: Some(CircleRef {
                        id: id(4),
                        name: "另一个社团".into(),
                    }),
                    release_date: None,
                    nsfw: true,
                    rate_average: None,
                    rate_count: None,
                    has_cover: false,
                    cover_version: integer(0),
                    missing: false,
                    favorite: false,
                    progress: Some(Progress {
                        track_id: None,
                        position_ms: integer(101000),
                        track_duration_ms: None,
                        finished: false,
                        replaying: false,
                        resumable: false,
                        group_path: None,
                    }),
                    my_rating: None,
                },
                WorkSummary {
                    id: id(14),
                    code: "BJ01234567".into(),
                    title: "示例作品三".into(),
                    circle: None,
                    release_date: Some("2023-11-20".into()),
                    nsfw: false,
                    rate_average: Some(3.9),
                    rate_count: Some(12),
                    has_cover: true,
                    cover_version: integer(2),
                    missing: false,
                    favorite: false,
                    progress: None,
                    my_rating: None,
                },
            ],
            page: 1,
            page_size: 40,
            total: integer(3),
        },
    );
}

#[test]
fn system_responses_and_empty_page() {
    assert_eq!(
        serde_json::to_value(HealthResponse {
            status: HealthStatus::Ok
        })
        .unwrap(),
        json!({"status": "ok"})
    );
    assert_eq!(
        serde_json::to_value(ReadyResponse {
            status: ReadyStatus::Ready
        })
        .unwrap(),
        json!({"status": "ready"})
    );
    assert_eq!(
        serde_json::to_value(VersionResponse {
            version: "0.1.0".into(),
            git_commit: None,
            build_time: None
        })
        .unwrap(),
        json!({"version": "0.1.0", "git_commit": null, "build_time": null})
    );
    assert_eq!(
        serde_json::to_value(Page::<WorkSummary> {
            items: vec![],
            page: 1,
            page_size: 40,
            total: integer(0)
        })
        .unwrap(),
        json!({"items": [], "page": 1, "page_size": 40, "total": 0})
    );
}

#[test]
fn integers_preserve_javascript_precision() {
    for value in [-MAX_SAFE_INTEGER, 0, MAX_SAFE_INTEGER] {
        let encoded = serde_json::to_value(integer(value)).unwrap();
        assert_eq!(encoded, json!(value));
        assert_eq!(
            serde_json::from_value::<SafeInteger>(encoded).unwrap(),
            integer(value)
        );
    }
    for value in [
        i64::MIN,
        -MAX_SAFE_INTEGER - 1,
        MAX_SAFE_INTEGER + 1,
        i64::MAX,
    ] {
        assert!(SafeInteger::try_from(value).is_err());
        assert!(serde_json::from_value::<SafeInteger>(json!(value)).is_err());
    }
    for value in [1, MAX_SAFE_INTEGER] {
        assert_eq!(serde_json::to_value(id(value)).unwrap(), json!(value));
        assert_eq!(
            serde_json::from_value::<Id>(json!(value)).unwrap(),
            id(value)
        );
    }
    for value in [-1, 0, MAX_SAFE_INTEGER + 1] {
        assert!(Id::try_from(value).is_err());
        assert!(serde_json::from_value::<Id>(json!(value)).is_err());
    }
    for value in [json!(1.5), json!("1"), Value::Null] {
        assert!(serde_json::from_value::<Id>(value.clone()).is_err());
        assert!(serde_json::from_value::<SafeInteger>(value).is_err());
    }
}

#[test]
fn progress_request() {
    sample_in(
        "m2",
        "progress-request.json",
        ProgressRequest {
            track_id: id(102),
            position_ms: integer(812_000),
            group_path: "mp3/SE有".into(),
            report_order: Some(ProgressReportOrder {
                session_id: "a3f0d725817c4b98b6e12c049d35af70".into(),
                sequence: integer(2),
            }),
        },
    );
}

#[test]
fn progress_request_null_and_legacy() {
    let request = ProgressRequest {
        track_id: id(102),
        position_ms: integer(812_000),
        group_path: "mp3/SE有".into(),
        report_order: None,
    };
    sample_in("m2", "progress-request-null.json", request.clone());
    let legacy = serde_json::from_value::<ProgressRequest>(json!({
        "track_id": 102, "position_ms": 812000, "group_path": "mp3/SE有"
    }))
    .unwrap();
    assert_eq!(legacy, request);
}

#[test]
fn finish_request() {
    sample_in(
        "m2",
        "finish-request.json",
        FinishRequest {
            track_id: id(102),
            group_path: "mp3/SE有".into(),
            manual: false,
        },
    );
    let without_manual: FinishRequest =
        serde_json::from_value(json!({"track_id": 1, "group_path": ""})).unwrap();
    assert!(!without_manual.manual);
}

#[test]
fn work_detail() {
    let track = |n: i64, path: &str, kind, size: Option<i64>, duration: Option<i64>| TrackItem {
        id: id(n),
        path: path.into(),
        kind,
        size: size.map(integer),
        duration_ms: duration.map(integer),
        has_subtitle: false,
        subtitle_formats: Vec::new(),
        missing: false,
        video: None,
        loudness: None,
    };
    let mut tracks = vec![
        track(
            101,
            "mp3/SE有/Tr01.mp3",
            TrackKind::Audio,
            Some(4821733),
            Some(301000),
        ),
        track(
            102,
            "mp3/SE有/Tr02.mp3",
            TrackKind::Audio,
            Some(31912880),
            Some(1995000),
        ),
        track(
            103,
            "mp3/SE有/Tr01.lrc",
            TrackKind::Subtitle,
            Some(2311),
            None,
        ),
        track(104, "wav/Tr01.wav", TrackKind::Audio, Some(53120044), None),
        track(105, "wav/Tr02.wav", TrackKind::Audio, None, None),
        track(106, "cover.jpg", TrackKind::Image, Some(182733), None),
    ];
    tracks[0].has_subtitle = true;
    tracks[0].subtitle_formats = vec!["lrc".into()];
    tracks[4].missing = true;
    tracks[0].loudness = Some(Loudness {
        integrated_lufs: Some(Decibels(-18.4)),
        true_peak_dbtp: Some(Decibels(-1.2)),
    });
    sample_in(
        "m2",
        "work-detail.json",
        WorkDetail {
            summary: WorkSummary {
                id: id(12),
                code: "RJ01234567".into(),
                title: "示例作品一".into(),
                circle: Some(CircleRef {
                    id: id(3),
                    name: "示例社团".into(),
                }),
                release_date: Some("2024-05-01".into()),
                nsfw: false,
                rate_average: Some(4.62),
                rate_count: Some(318),
                has_cover: true,
                cover_version: integer(1),
                missing: false,
                favorite: true,
                progress: Some(Progress {
                    track_id: Some(id(102)),
                    position_ms: integer(812000),
                    track_duration_ms: Some(integer(1995000)),
                    finished: true,
                    replaying: true,
                    resumable: true,
                    group_path: Some("mp3/SE有".into()),
                }),
                my_rating: Some(4),
            },
            vas: vec![
                VaRef {
                    id: "va-sample-1".into(),
                    name: "示例声优甲".into(),
                },
                VaRef {
                    id: "va-sample-2".into(),
                    name: "示例声优乙".into(),
                },
            ],
            tags: vec![
                TagRef {
                    id: id(7),
                    name: "ASMR".into(),
                },
                TagRef {
                    id: id(9),
                    name: "耳语".into(),
                },
            ],
            total_duration_ms: None,
            tracks,
            my_review: Some(review()),
            locations: Some(vec![
                WorkLocation {
                    root_id: "local".into(),
                    root_name: "本地作品".into(),
                    path: "示例社团/RJ01234567".into(),
                    missing: false,
                },
                WorkLocation {
                    root_id: "cloud".into(),
                    root_name: "云端作品".into(),
                    path: "RJ01234567".into(),
                    missing: true,
                },
            ]),
        },
    );
}

#[test]
fn roots() {
    sample_in(
        "m2",
        "roots.json",
        vec![
            RootStatus {
                id: "local".into(),
                display_name: "本地作品".into(),
                kind: RootKind::Local,
                status: RootHealth::Ok,
                last_scan_at: Some(integer(1791699000000)),
                work_count: integer(120),

                source: RootSource::Config,
                settings: RootSettings::Local {
                    path: "/srv/media/works".into(),
                    watch: false,
                    auto_scan_interval: None,
                },
                next_scan_at: None,
                watching: false,
            },
            RootStatus {
                id: "cloud".into(),
                display_name: "云端作品".into(),
                kind: RootKind::OpenList,
                status: RootHealth::Unknown,
                last_scan_at: None,
                work_count: integer(48),

                source: RootSource::Config,
                settings: openlist_settings(None),
                next_scan_at: None,
                watching: false,
            },
        ],
    );
}

#[test]
fn task_running() {
    sample_in(
        "m2",
        "task-running.json",
        TaskSnapshot {
            id: id(88),
            kind: TaskKind::Transcode,
            state: TaskState::Running,
            progress: Some(Fraction(0.42)),
            message: None,
            result: None,
            queue_position: None,
            created_at: integer(1_791_700_000_000),
            updated_at: integer(1_791_700_021_000),
        },
    );
}

#[test]
fn task_done() {
    sample_in(
        "m2",
        "task-done.json",
        TaskSnapshot {
            id: id(88),
            kind: TaskKind::Transcode,
            state: TaskState::Done,
            progress: Some(Fraction(1.0)),
            message: None,
            result: Some(PlayableMedia {
                url: "/api/tracks/104/playable".into(),
                mime_type: "audio/mp4".into(),
            }),
            queue_position: None,
            created_at: integer(1_791_700_000_000),
            updated_at: integer(1_791_700_064_000),
        },
    );
}

#[test]
fn scan_response() {
    sample_in(
        "m2",
        "scan-response.json",
        ScanResponse {
            task_ids: vec![id(91)],
        },
    );
}

#[test]
fn prepare_request() {
    sample_in(
        "m2",
        "prepare-request.json",
        PrepareRequest {
            can_play: [
                "audio/mpeg",
                "audio/wav",
                "audio/flac",
                "audio/mp4",
                "video/mp4",
            ]
            .map(String::from)
            .to_vec(),
            force_fallback: false,
        },
    );
}

#[test]
fn prepare_responses() {
    sample_in(
        "m2",
        "prepare-ready.json",
        PrepareResponse::Ready {
            url: "/api/tracks/102/stream".into(),
            mime_type: "audio/mpeg".into(),
        },
    );
    sample_in(
        "m2",
        "prepare-processing.json",
        PrepareResponse::Processing { task_id: id(88) },
    );
}

#[test]
fn error_media_not_ready() {
    sample_in(
        "m2",
        "error-media-not-ready.json",
        ErrorResponse {
            error: ApiError {
                code: ErrorCode::MediaNotReady,
                message: "这个文件还在转码，请稍后再试".into(),
            },
        },
    );
}

fn m3_work(n: i64, code: &str, title: &str, missing: bool, playing: bool) -> WorkSummary {
    WorkSummary {
        id: id(n),
        code: code.into(),
        title: title.into(),
        circle: Some(CircleRef {
            id: id(3),
            name: "示例社团".into(),
        }),
        release_date: Some("2024-05-01".into()),
        nsfw: false,
        rate_average: Some(4.62),
        rate_count: Some(318),
        has_cover: true,
        cover_version: integer(1),
        missing,
        favorite: true,
        progress: playing.then(|| Progress {
            track_id: Some(id(345)),
            position_ms: integer(812000),
            track_duration_ms: Some(integer(1995000)),
            finished: false,
            replaying: false,
            resumable: true,
            group_path: Some("mp3/SE有".into()),
        }),
        my_rating: None,
    }
}

#[test]
fn favorites_and_history_pages() {
    sample_in(
        "m3",
        "favorites-page.json",
        Page {
            items: vec![
                m3_work(12, "RJ01234567", "示例作品一", false, true),
                m3_work(15, "RJ01000015", "示例作品四", true, false),
            ],
            page: 1,
            page_size: 40,
            total: integer(2),
        },
    );
    sample_in(
        "m3",
        "history-page.json",
        Page {
            items: vec![m3_work(12, "RJ01234567", "示例作品一", false, true)],
            page: 1,
            page_size: 40,
            total: integer(1),
        },
    );
}

#[test]
fn filter_lists() {
    sample_in(
        "m3",
        "circles.json",
        vec![
            CircleCount {
                id: id(3),
                name: "示例社团".into(),
                work_count: integer(12),
            },
            CircleCount {
                id: id(4),
                name: "另一个社团".into(),
                work_count: integer(1),
            },
        ],
    );
    sample_in(
        "m3",
        "vas.json",
        vec![
            VaCount {
                id: "va-sample-1".into(),
                name: "示例声优甲".into(),
                work_count: integer(30),
            },
            VaCount {
                id: "va-sample-2".into(),
                name: "示例声优乙".into(),
                work_count: integer(2),
            },
        ],
    );
    sample_in(
        "m3",
        "tags.json",
        vec![
            TagCount {
                id: id(7),
                name: "ASMR".into(),
                work_count: integer(120),
            },
            TagCount {
                id: id(9),
                name: "耳语".into(),
                work_count: integer(8),
            },
        ],
    );
}

#[test]
fn tag_merge_request() {
    sample_in(
        "m4",
        "tag-merge-request.json",
        TagMergeRequest {
            from: id(9),
            into: id(7),
        },
    );
}

#[test]
fn tasks_page() {
    sample_in(
        "m4",
        "tasks-page.json",
        Page {
            items: vec![
                TaskSnapshot {
                    id: id(121),
                    kind: TaskKind::Scan,
                    state: TaskState::Running,
                    progress: Some(Fraction(0.35)),
                    message: Some("同步轨道 410/1200".into()),
                    result: None,
                    queue_position: None,
                    created_at: integer(1_791_800_000_000),
                    updated_at: integer(1_791_800_090_000),
                },
                TaskSnapshot {
                    id: id(120),
                    kind: TaskKind::Metadata,
                    state: TaskState::Failed,
                    progress: None,
                    message: Some("DLsite 暂时无法访问".into()),
                    result: None,
                    queue_position: None,
                    created_at: integer(1_791_799_000_000),
                    updated_at: integer(1_791_799_004_000),
                },
            ],
            page: 1,
            page_size: 40,
            total: integer(2),
        },
    );
}

#[test]
fn task_queued_scan() {
    sample_in(
        "m4",
        "task-queued-scan.json",
        TaskSnapshot {
            id: id(122),
            kind: TaskKind::Scan,
            state: TaskState::Queued,
            progress: None,
            message: None,
            result: None,
            queue_position: Some(2),
            created_at: integer(1_791_800_100_000),
            updated_at: integer(1_791_800_100_000),
        },
    );
}

#[test]
fn metadata_request() {
    sample_in(
        "m4",
        "metadata-request.json",
        MetadataRequest {
            title: "示例作品一（修正）".into(),
            release_date: Some("2024-05-01".into()),
            nsfw: false,
            circle_name: Some("示例社团".into()),
            va_names: vec!["示例声优甲".into(), "示例声优乙".into()],
            tag_names: vec!["ASMR".into(), "耳语".into()],
        },
    );
}

#[test]
fn missing_and_cleanup() {
    sample_in(
        "m4",
        "missing.json",
        MissingReport {
            works: vec![MissingWork {
                id: id(15),
                code: "RJ01000015".into(),
                title: "示例作品四".into(),
                missing_at: integer(1791700000000),
            }],
            locations: vec![MissingLocation {
                id: id(31),
                work_id: id(12),
                code: "RJ01234567".into(),
                root_id: "cloud".into(),
                path: "示例目录/RJ01234567".into(),
                missing_at: integer(1791700000000),
            }],
        },
    );
    sample_in(
        "m4",
        "cleanup-request.json",
        CleanupRequest {
            work_ids: vec![id(15)],
            location_ids: vec![id(31)],
        },
    );
    sample_in(
        "m4",
        "cleanup-response.json",
        CleanupResponse {
            works_deleted: integer(1),
            locations_deleted: integer(1),
        },
    );
}

#[test]
fn m6_video_track() {
    sample_in(
        "m6",
        "track-video.json",
        TrackItem {
            id: id(201),
            path: "映像/本編.mkv".into(),
            kind: TrackKind::Video,
            size: Some(integer(734_003_200)),
            duration_ms: Some(integer(1_520_000)),
            has_subtitle: true,
            subtitle_formats: vec!["vtt".into(), "ass".into()],
            missing: false,
            video: Some(VideoInfo {
                codec: "h264".into(),
                width: Some(1920),
                height: Some(1080),
            }),
            loudness: None,
        },
    );
}

#[test]
fn m6_hls() {
    let hls = || PlayableMedia {
        url: "/api/tracks/201/hls/index.m3u8".into(),
        mime_type: "application/vnd.apple.mpegurl".into(),
    };
    sample_in(
        "m6",
        "prepare-ready-hls.json",
        PrepareResponse::Ready {
            url: hls().url,
            mime_type: hls().mime_type,
        },
    );
    sample_in(
        "m6",
        "task-done-hls.json",
        TaskSnapshot {
            id: id(95),
            kind: TaskKind::Transcode,
            state: TaskState::Done,
            progress: Some(Fraction(1.0)),
            message: None,
            result: Some(hls()),
            queue_position: None,
            created_at: integer(1_791_700_000_000),
            updated_at: integer(1_791_700_300_000),
        },
    );
}

#[test]
fn m6_fonts() {
    sample_in(
        "m6",
        "fonts.json",
        vec![
            TrackFont {
                id: id(210),
                name: "示例字体.ttf".into(),
                url: "/api/tracks/210/stream".into(),
            },
            TrackFont {
                id: id(211),
                name: "SampleSans-Bold.otf".into(),
                url: "/api/tracks/211/stream".into(),
            },
        ],
    );
}

#[test]
fn refresh_response() {
    sample_in(
        "m4",
        "refresh-response.json",
        RefreshResponse { task_id: id(120) },
    );
}

fn openlist_settings(auto_scan_interval: Option<u32>) -> RootSettings {
    RootSettings::OpenList {
        url: "https://openlist.example.lan".into(),
        base_path: "/cloud/云端作品".into(),
        username: "kikoeru".into(),
        password_set: true,
        list_concurrency: None,
        link_rate: None,
        auto_scan_interval,
    }
}

#[test]
fn m7_roots() {
    sample_in(
        "m7",
        "roots.json",
        vec![
            RootStatus {
                id: "local".into(),
                display_name: "本地作品".into(),
                kind: RootKind::Local,
                status: RootHealth::Ok,
                last_scan_at: Some(integer(1791699000000)),
                work_count: integer(120),
                source: RootSource::Config,
                settings: RootSettings::Local {
                    path: "/srv/media/works".into(),
                    watch: true,
                    auto_scan_interval: Some(86400),
                },
                next_scan_at: Some(integer(1791785400000)),
                watching: true,
            },
            RootStatus {
                id: "cloud".into(),
                display_name: "云端作品".into(),
                kind: RootKind::OpenList,
                status: RootHealth::Unknown,
                last_scan_at: None,
                work_count: integer(48),
                source: RootSource::Db,
                settings: openlist_settings(Some(21600)),
                next_scan_at: Some(integer(1791720600000)),
                watching: false,
            },
        ],
    );
    sample_in(
        "m7",
        "root-create-request.json",
        RootCreateRequest {
            id: "cloud".into(),
            display_name: "云端作品".into(),
            settings: RootInputSettings::OpenList {
                url: "https://openlist.example.lan".into(),
                base_path: "/cloud/云端作品".into(),
                username: "kikoeru".into(),
                password: Some("sample password".into()),
                list_concurrency: None,
                link_rate: None,
                auto_scan_interval: Some(21600),
                watch: None,
            },
            skip_check: Some(false),
        },
    );
}

#[test]
fn webdav_roots() {
    sample_in(
        "webdav",
        "roots.json",
        vec![RootStatus {
            id: "nas".into(),
            display_name: "NAS".into(),
            kind: RootKind::WebDav,
            status: RootHealth::Ok,
            last_scan_at: Some(integer(1791699000000)),
            work_count: integer(212),
            source: RootSource::Db,
            settings: RootSettings::WebDav {
                url: "https://nas.example.lan/dav".into(),
                base_path: "/音声".into(),
                username: "kikoeru".into(),
                password_set: true,
                list_concurrency: None,
                auto_scan_interval: Some(21600),
            },
            next_scan_at: Some(integer(1791720600000)),
            watching: false,
        }],
    );
    sample_in(
        "webdav",
        "root-create-request.json",
        RootCreateRequest {
            id: "nas".into(),
            display_name: "NAS".into(),
            settings: RootInputSettings::WebDav {
                url: "https://nas.example.lan/dav".into(),
                base_path: "/音声".into(),
                username: "kikoeru".into(),
                password: Some("sample password".into()),
                list_concurrency: None,
                auto_scan_interval: Some(21600),
                watch: None,
            },
            skip_check: Some(false),
        },
    );
}

#[test]
fn loudness() {
    sample_in(
        "loudness",
        "loudness.json",
        Loudness {
            integrated_lufs: Some(Decibels(-18.4)),
            true_peak_dbtp: Some(Decibels(-1.2)),
        },
    );
    sample_in(
        "loudness",
        "loudness-processing.json",
        PrepareResponse::Processing { task_id: id(91) },
    );
    sample_in(
        "loudness",
        "user-settings.json",
        UserSettings {
            normalize_loudness: true,
            target_lufs: Decibels(-16.0),
            accent: Some(Accent::Teal),
            seek_back_seconds: 10,
            seek_forward_seconds: 45,
        },
    );
}

#[test]
fn server_settings() {
    sample_in(
        "settings",
        "server-settings.json",
        ServerSettings {
            metadata_proxy: Some("http://127.0.0.1:7890".into()),
        },
    );
    sample_in(
        "settings",
        "proxy-check.json",
        ProxyCheckResult {
            url: "https://www.dlsite.com/".into(),
            status: 200,
            elapsed_ms: 412,
        },
    );
}

fn review() -> Review {
    Review {
        rating: Some(4),
        text: Some("耳かき的部分很舒服，后半略长。".into()),
        updated_at: Some(integer(1_760_245_200_000)),
    }
}

#[test]
fn reviews() {
    sample_in("reviews", "review.json", review());
    sample_in(
        "reviews",
        "review-request.json",
        ReviewInput {
            rating: Some(4),
            text: Some("耳かき的部分很舒服，后半略长。".into()),
        },
    );
    let mut rated = m3_work(12, "RJ01234567", "示例作品一", false, true);
    rated.my_rating = Some(4);
    sample_in(
        "reviews",
        "reviews-page.json",
        Page {
            items: vec![rated, m3_work(15, "RJ01000015", "示例作品四", true, false)],
            page: 1,
            page_size: 40,
            total: integer(2),
        },
    );
}
