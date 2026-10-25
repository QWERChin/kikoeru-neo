mod desktop;
mod import;
mod number;

pub use desktop::{DesktopMode, DesktopState, MediaControl};
pub use import::{
    ImportCounts, ImportDiscovered, ImportErrorCode, ImportFailure, ImportIssue, ImportOldRoot,
    ImportPath, ImportPreview, ImportResult, ImportSample, ImportSource, ImportUser,
};
pub use number::{Id, MAX_SAFE_INTEGER, SafeInteger};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct User {
    pub id: Id,
    pub name: String,
    pub is_admin: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CircleRef {
    pub id: Id,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Progress {
    pub track_id: Option<Id>,
    pub position_ms: SafeInteger,
    pub track_duration_ms: Option<SafeInteger>,
    pub finished: bool,
    pub replaying: bool,
    pub resumable: bool,
    pub group_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct WorkSummary {
    pub id: Id,
    pub code: String,
    pub title: String,
    pub circle: Option<CircleRef>,
    pub release_date: Option<String>,
    pub nsfw: bool,
    pub rate_average: Option<f64>,
    pub rate_count: Option<u32>,
    pub has_cover: bool,
    pub cover_version: SafeInteger,
    pub missing: bool,
    pub favorite: bool,
    pub progress: Option<Progress>,
    pub my_rating: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct VaRef {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct TagRef {
    pub id: Id,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Audio,
    Video,
    Subtitle,
    Image,
    Text,
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct TrackItem {
    pub id: Id,
    pub path: String,
    pub kind: TrackKind,
    pub size: Option<SafeInteger>,
    pub duration_ms: Option<SafeInteger>,
    pub has_subtitle: bool,
    /// Formats of the same-name subtitles, in the order the subtitle
    /// endpoint prefers them: vtt, srt, lrc, ass.
    pub subtitle_formats: Vec<String>,
    pub missing: bool,
    pub video: Option<VideoInfo>,
    /// `None` until an `analysis` task has measured the current file.
    pub loudness: Option<Loudness>,
}

/// A track's measured loudness (contract "响度和波形").
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
pub struct Loudness {
    pub integrated_lufs: Option<Decibels>,
    pub true_peak_dbtp: Option<Decibels>,
}

/// `GET` and `PUT /api/me/settings`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
pub struct UserSettings {
    pub normalize_loudness: bool,
    pub target_lufs: Decibels,
    /// `None` is the default blue. A `PUT` without it counts as `None`, so
    /// clients from before the field can still save their other settings.
    #[serde(default)]
    pub accent: Option<Accent>,
    /// How far the back and forward buttons skip, 1 to 99 seconds. A `PUT`
    /// without them gets the defaults, for the same reason as `accent`.
    #[serde(default = "UserSettings::default_seek_back")]
    pub seek_back_seconds: u32,
    #[serde(default = "UserSettings::default_seek_forward")]
    pub seek_forward_seconds: u32,
}

impl UserSettings {
    pub const DEFAULT_SEEK_BACK: u32 = 15;
    pub const DEFAULT_SEEK_FORWARD: u32 = 30;
    /// The button icon draws the number inside the arrow, with room for two digits.
    pub const SEEK_SECONDS: std::ops::RangeInclusive<u32> = 1..=99;

    fn default_seek_back() -> u32 {
        Self::DEFAULT_SEEK_BACK
    }

    fn default_seek_forward() -> u32 {
        Self::DEFAULT_SEEK_FORWARD
    }
}

/// `GET` and `PUT /api/admin/settings`, and the body of
/// `POST /api/admin/settings/proxy-check`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ServerSettings {
    /// The proxy for metadata and cover requests. `None` leaves them to the
    /// proxy environment variables. A `PUT` without it counts as `None`.
    #[serde(default)]
    pub metadata_proxy: Option<String>,
}

/// DLsite answered `POST /api/admin/settings/proxy-check`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ProxyCheckResult {
    pub url: String,
    pub status: u16,
    pub elapsed_ms: u32,
}

/// An accent colour, named after the iOS system colour the frontend draws
/// it as. The set is fixed so both themes stay legible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum Accent {
    Blue,
    Purple,
    Pink,
    Red,
    Orange,
    Green,
    Teal,
    Indigo,
}

impl Accent {
    pub const ALL: [Self; 8] = [
        Self::Blue,
        Self::Purple,
        Self::Pink,
        Self::Red,
        Self::Orange,
        Self::Green,
        Self::Teal,
        Self::Indigo,
    ];

    /// The name in JSON and in `user_settings.accent`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Blue => "blue",
            Self::Purple => "purple",
            Self::Pink => "pink",
            Self::Red => "red",
            Self::Orange => "orange",
            Self::Green => "green",
            Self::Teal => "teal",
            Self::Indigo => "indigo",
        }
    }

    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.as_str() == name)
    }
}

/// A level in LUFS or dB. Whole values are written without a fraction
/// (`-16`, not `-16.0`), like [`Fraction`].
#[derive(Debug, Clone, Copy, PartialEq, TS)]
#[ts(type = "number")]
pub struct Decibels(pub f64);

impl<'de> Deserialize<'de> for Decibels {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        f64::deserialize(deserializer).map(Self)
    }
}

impl Serialize for Decibels {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Levels are small, so a whole value always fits an i64 exactly.
        if self.0.fract() == 0.0 && self.0.abs() < 1e6 {
            serializer.serialize_i64(self.0 as i64)
        } else {
            serializer.serialize_f64(self.0)
        }
    }
}

/// The picture of a probed video track.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct VideoInfo {
    pub codec: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// A font file for JASSUB, from `GET /api/tracks/{id}/fonts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct TrackFont {
    pub id: Id,
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct WorkDetail {
    #[serde(flatten)]
    #[ts(flatten)]
    pub summary: WorkSummary,
    pub vas: Vec<VaRef>,
    pub tags: Vec<TagRef>,
    pub total_duration_ms: Option<SafeInteger>,
    pub tracks: Vec<TrackItem>,
    pub my_review: Option<Review>,
    /// `None` for users who are not admins, who never see root details.
    pub locations: Option<Vec<WorkLocation>>,
}

/// Where a work's directory sits, so an admin can find it in the root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct WorkLocation {
    pub root_id: String,
    pub root_name: String,
    pub path: String,
    pub missing: bool,
}

/// The user's own rating and short review of a work. `rating` and `text`
/// are never both `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Review {
    pub rating: Option<u8>,
    pub text: Option<String>,
    pub updated_at: Option<SafeInteger>,
}

/// The body of `PUT /api/me/reviews/{work_id}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ReviewInput {
    pub rating: Option<u8>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum RootKind {
    Local,
    OpenList,
    WebDav,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum RootHealth {
    Ok,
    Unavailable,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CircleCount {
    pub id: Id,
    pub name: String,
    pub work_count: SafeInteger,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct VaCount {
    pub id: String,
    pub name: String,
    pub work_count: SafeInteger,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct TagCount {
    pub id: Id,
    pub name: String,
    pub work_count: SafeInteger,
}

/// `PUT /api/works/{id}/metadata`: the whole editable metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct MetadataRequest {
    pub title: String,
    pub release_date: Option<String>,
    pub nsfw: bool,
    pub circle_name: Option<String>,
    pub va_names: Vec<String>,
    pub tag_names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct MissingWork {
    pub id: Id,
    pub code: String,
    pub title: String,
    pub missing_at: SafeInteger,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct MissingLocation {
    pub id: Id,
    pub work_id: Id,
    pub code: String,
    pub root_id: String,
    pub path: String,
    pub missing_at: SafeInteger,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct MissingReport {
    pub works: Vec<MissingWork>,
    pub locations: Vec<MissingLocation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CleanupRequest {
    pub work_ids: Vec<Id>,
    pub location_ids: Vec<Id>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CleanupResponse {
    pub works_deleted: SafeInteger,
    pub locations_deleted: SafeInteger,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct RefreshResponse {
    pub task_id: Id,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct TagMergeRequest {
    pub from: Id,
    pub into: Id,
}

/// `POST /api/admin/scan`. A missing or null `root` scans every root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ScanRequest {
    #[serde(default)]
    pub root: Option<String>,
    /// Walks every OpenList work, including those whose top-level listing
    /// has not changed since the last scan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub full: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ScanResponse {
    pub task_ids: Vec<Id>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct RootStatus {
    pub id: String,
    pub display_name: String,
    pub kind: RootKind,
    pub status: RootHealth,
    pub last_scan_at: Option<SafeInteger>,
    pub work_count: SafeInteger,
    pub source: RootSource,
    pub settings: RootSettings,
    /// When the next automatic scan is due, Unix milliseconds. Null when
    /// automatic scans are off.
    pub next_scan_at: Option<SafeInteger>,
    /// Whether the directory watcher is running for this root.
    pub watching: bool,
}

/// Where a root is defined. Config roots are read-only through the API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum RootSource {
    Config,
    Db,
}

/// A root's connection settings as the API shows them. The OpenList
/// password is never included, only whether one is set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum RootSettings {
    Local {
        path: String,
        /// The effective value, after the default for the running mode.
        watch: bool,
        /// Seconds between automatic scans, or null when they are off.
        auto_scan_interval: Option<u32>,
    },
    OpenList {
        url: String,
        base_path: String,
        username: String,
        password_set: bool,
        list_concurrency: Option<u32>,
        link_rate: Option<f64>,
        auto_scan_interval: Option<u32>,
    },
    /// A read-only WebDAV share (book/src/storage.md, "webdav").
    WebDav {
        url: String,
        base_path: String,
        username: String,
        password_set: bool,
        list_concurrency: Option<u32>,
        auto_scan_interval: Option<u32>,
    },
}

/// `POST /api/admin/roots`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct RootCreateRequest {
    pub id: String,
    pub display_name: String,
    pub settings: RootInputSettings,
    /// Save even when OpenList cannot be reached. Absent means false.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub skip_check: Option<bool>,
}

/// `PUT /api/admin/roots/{id}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct RootUpdateRequest {
    pub display_name: String,
    pub settings: RootInputSettings,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub skip_check: Option<bool>,
}

/// Settings as a client sends them. `password` is required when a root is
/// created and keeps the stored one when it is null on an update.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum RootInputSettings {
    Local {
        path: String,
        /// Null takes the default for the running mode.
        #[serde(default)]
        #[ts(optional = nullable)]
        watch: Option<bool>,
        #[serde(default)]
        #[ts(optional = nullable)]
        auto_scan_interval: Option<u32>,
    },
    OpenList {
        url: String,
        base_path: String,
        username: String,
        #[serde(default)]
        #[ts(optional = nullable)]
        password: Option<String>,
        #[serde(default)]
        #[ts(optional = nullable)]
        list_concurrency: Option<u32>,
        #[serde(default)]
        #[ts(optional = nullable)]
        link_rate: Option<f64>,
        #[serde(default)]
        #[ts(optional = nullable)]
        auto_scan_interval: Option<u32>,
        /// Read only so that `true` can be refused with 400: a remote root
        /// cannot be watched, and the contract forbids dropping it silently.
        #[serde(default, skip_serializing)]
        #[ts(skip)]
        watch: Option<bool>,
    },
    WebDav {
        url: String,
        base_path: String,
        username: String,
        #[serde(default)]
        #[ts(optional = nullable)]
        password: Option<String>,
        #[serde(default)]
        #[ts(optional = nullable)]
        list_concurrency: Option<u32>,
        #[serde(default)]
        #[ts(optional = nullable)]
        auto_scan_interval: Option<u32>,
        /// Read only so that `true` can be refused with 400: a remote root
        /// cannot be watched, and the contract forbids dropping it silently.
        #[serde(default, skip_serializing)]
        #[ts(skip)]
        watch: Option<bool>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub page: u32,
    pub page_size: u32,
    pub total: SafeInteger,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidRequest,
    Unauthorized,
    InvalidCredentials,
    SetupRequired,
    Forbidden,
    NotFound,
    MediaNotReady,
    Missing,
    TooLarge,
    QueueFull,
    TooManyAttempts,
    InternalError,
    UpstreamError,
    NotReady,
    StorageUnavailable,
    InsufficientStorage,
    ScanRunning,
    RootExists,
    RootReadOnly,
    NotApplicable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ErrorResponse {
    pub error: ApiError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Ok,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct HealthResponse {
    pub status: HealthStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ReadyStatus {
    Ready,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ReadyResponse {
    pub status: ReadyStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct VersionResponse {
    pub version: String,
    pub git_commit: Option<String>,
    pub build_time: Option<SafeInteger>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct LoginRequest {
    pub name: String,
    pub password: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct LoginResponse {
    pub token: String,
    pub expires_at: SafeInteger,
    pub user: User,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct MediaTokenResponse {
    pub token: String,
    pub expires_at: SafeInteger,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct PasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ProgressReportOrder {
    pub session_id: String,
    pub sequence: SafeInteger,
}

/// `PUT /api/me/progress/{work_id}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ProgressRequest {
    pub track_id: Id,
    pub position_ms: SafeInteger,
    pub group_path: String,
    #[serde(default)]
    pub report_order: Option<ProgressReportOrder>,
}

/// `POST /api/me/progress/{work_id}/finish`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct FinishRequest {
    pub track_id: Id,
    pub group_path: String,
    /// The play page's "mark as listened" button, which skips the
    /// last-track check.
    #[serde(default)]
    pub manual: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    Scan,
    Probe,
    Transcode,
    Metadata,
    Analysis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
    Interrupted,
}

/// Where a browser-playable version of a track is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct PlayableMedia {
    pub url: String,
    pub mime_type: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct TaskSnapshot {
    pub id: Id,
    pub kind: TaskKind,
    pub state: TaskState,
    pub progress: Option<Fraction>,
    pub message: Option<String>,
    pub result: Option<PlayableMedia>,
    /// A queued scan's place in the scan lane, 1 being the next to start.
    pub queue_position: Option<u32>,
    pub created_at: SafeInteger,
    pub updated_at: SafeInteger,
}

/// A number from 0 to 1. Whole values are written without a fraction (`1`,
/// not `1.0`), the way the contract samples and JavaScript print them.
#[derive(Debug, Clone, Copy, PartialEq, TS)]
#[ts(type = "number")]
pub struct Fraction(pub f64);

impl<'de> Deserialize<'de> for Fraction {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        f64::deserialize(deserializer).map(Self)
    }
}

impl Serialize for Fraction {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.0.fract() == 0.0 && self.0.abs() <= 1.0 {
            serializer.serialize_i64(self.0 as i64)
        } else {
            serializer.serialize_f64(self.0)
        }
    }
}

/// `POST /api/tracks/{id}/prepare`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct PrepareRequest {
    /// MIME types for which `canPlayType` answered `probably` or `maybe`.
    pub can_play: Vec<String>,
    /// The original failed to play, so a transcode is required.
    #[serde(default)]
    pub force_fallback: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PrepareResponse {
    /// 200: play `url` now. The URL is relative and takes `?token=`.
    Ready { url: String, mime_type: String },
    /// 202: poll `GET /api/tasks/{task_id}` until it is done.
    Processing { task_id: Id },
}
