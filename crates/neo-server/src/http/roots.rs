//! Adding, editing and removing roots through the API
//! (book/src/contract.md, "媒体库管理接口").

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use axum::{Json, extract::State, http::StatusCode};
use neo_core::dto::{
    ErrorCode, RootCreateRequest, RootInputSettings, RootKind, RootSettings, RootSource,
    RootStatus, RootUpdateRequest,
};
use neo_db as db;
use neo_db::roots::StoredRoot;
use neo_storage::{EntryKind, RawPath, Storage, StorageError};

use super::{
    AppState, HttpError, RootEntry,
    admin::root_status,
    extract::{AdminUser, ApiJson, ApiPath},
};
use crate::{
    roots::{self as library, StoredSettings},
    scan,
};

const MAX_DISPLAY_NAME_CHARS: usize = 100;

pub async fn create(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    ApiJson(request): ApiJson<RootCreateRequest>,
) -> Result<(StatusCode, Json<RootStatus>), HttpError> {
    let pending = state.track_root_change()?;
    if !library::is_valid_stored_id(&request.id) {
        return Err(HttpError::invalid_request(
            "id 只能用 1 到 32 个小写字母、数字、- 和 _，并以字母或数字开头",
        ));
    }
    let display_name = check_display_name(&request.display_name)?;
    let (mut settings, password) = check_settings(request.settings).await?;
    if let Some(remote) = remote_name(&settings)
        && password.is_none()
    {
        return Err(HttpError::invalid_request(format!(
            "{remote} 媒体库需要密码"
        )));
    }
    let data_dir = state.data_dir()?.root().to_owned();
    let id = request.id;
    // Checked before the slow connectivity check and again under the lock.
    check_free(&state, &id)?;
    let storage = try_settings(
        &state,
        &id,
        &settings,
        password.as_deref(),
        request.skip_check,
    )
    .await?;
    let changes = state.lock_root_changes().await;
    let entry = run_to_end(async move {
        let _pending = pending;
        let _changes = changes;
        check_free(&state, &id)?;
        let _secrets = lock_secrets(&data_dir).await?;
        if let Some(password) = &password {
            let version = library::write_password(&data_dir, &id, password)
                .map_err(|e| HttpError::internal(format!("{e:#}")))?;
            settings.set_password_version(Some(version));
        }
        let stored = settings.to_stored(&id, &display_name);
        let inserted = db::roots::insert_stored(state.db(), &stored, db::now_ms()).await;
        if !matches!(inserted, Ok(true)) {
            // COMMIT errors can have an uncertain outcome. Leave the file
            // for a later sweep to check against committed references.
            return match inserted {
                // A stored root whose settings could not be read at
                // startup is not in the registry but still has its row.
                Ok(_) => Err(root_exists(&id)),
                Err(e) => Err(e.into()),
            };
        }
        #[cfg(test)]
        library::crash_tests::crash_at("database_committed");
        let entry = publish(&state, &data_dir, &stored, storage)?;
        cleanup_passwords(&state, &data_dir).await;
        root_status(&state, &entry).await
    })
    .await?;
    Ok((StatusCode::CREATED, Json(entry)))
}

pub async fn update(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    ApiPath(id): ApiPath<String>,
    ApiJson(request): ApiJson<RootUpdateRequest>,
) -> Result<Json<RootStatus>, HttpError> {
    let pending = state.track_root_change()?;
    let display_name = check_display_name(&request.display_name)?;
    let (mut settings, password) = check_settings(request.settings).await?;
    let data_dir = state.data_dir()?.root().to_owned();
    // The saved password is read under the lock together with the settings
    // it belongs to. Read later, it could already be the password of a new
    // server that another edit is saving, and the check below would send
    // it to the old one.
    let saved = {
        let _changes = state.lock_root_changes().await;
        let (current, _) =
            check_editable(&state, &id, &settings, password.is_some(), &data_dir).await?;
        if password.is_none() && remote_name(&settings).is_some() {
            Some(read_password(&current, &data_dir, &id)?)
        } else {
            None
        }
    };
    let checked = try_settings(
        &state,
        &id,
        &settings,
        password
            .as_deref()
            .or(saved.as_deref().map(|p| p.trim_end_matches(['\r', '\n']))),
        request.skip_check,
    )
    .await?;
    let changes = state.lock_root_changes().await;
    let status = run_to_end(async move {
        let _pending = pending;
        let _changes = changes;
        let (current_settings, location_changed) =
            check_editable(&state, &id, &settings, password.is_some(), &data_dir).await?;
        if let Some(saved) = &saved
            && read_password(&current_settings, &data_dir, &id)? != *saved
        {
            return Err(HttpError::invalid_request(
                "保存的密码刚被另一个修改换掉了，请重新提交",
            ));
        }
        // A scan of the old location would write its listing digests back
        // after they are dropped, and the next scan would trust them.
        if location_changed
            && db::tasks::active_id(state.db(), &scan::dedupe_key(&id))
                .await?
                .is_some()
        {
            return Err(HttpError::scan_running());
        }
        let _secrets = lock_secrets(&data_dir).await?;
        settings.set_password_version(current_settings.password_version().map(str::to_owned));
        if let Some(password) = &password {
            let version = library::write_password(&data_dir, &id, password)
                .map_err(|e| HttpError::internal(format!("{e:#}")))?;
            settings.set_password_version(Some(version));
        }
        let stored = settings.to_stored(&id, &display_name);
        let updated =
            db::roots::update_stored(state.db(), &stored, location_changed, db::now_ms()).await;
        if !matches!(updated, Ok(true)) {
            return Err(match updated {
                Ok(_) => HttpError::internal(format!("root {id} has no stored settings")),
                Err(e) => e.into(),
            });
        }
        #[cfg(test)]
        library::crash_tests::crash_at("database_committed");
        let entry = publish(&state, &data_dir, &stored, checked)?;
        cleanup_passwords(&state, &data_dir).await;
        root_status(&state, &entry).await
    })
    .await?;
    Ok(Json(status))
}

pub async fn remove(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    ApiPath(id): ApiPath<String>,
) -> Result<StatusCode, HttpError> {
    let pending = state.track_root_change()?;
    let data_dir = state.data_dir()?.root().to_owned();
    // Scan requests take the same lock, so none can be admitted between
    // the check below and the removal.
    let changes = state.lock_root_changes().await;
    run_to_end(async move {
        let _pending = pending;
        let _changes = changes;
        stored_root(&state, &id)?;
        if db::tasks::active_id(state.db(), &scan::dedupe_key(&id))
            .await?
            .is_some()
        {
            return Err(HttpError::scan_running());
        }
        let _secrets = lock_secrets(&data_dir).await?;
        let order: Vec<_> = scan::root_order(&state.root_kinds())
            .into_iter()
            .filter(|r| *r != id)
            .collect();
        let mut tx = state.db().begin().await?;
        let works = db::roots::remove_stored(&mut tx, &id, db::now_ms())
            .await?
            .ok_or_else(|| HttpError::internal(format!("root {id} has no stored settings")))?;
        // A copy in this root may have been the kept one of a file that
        // other roots also hold. Another copy takes over, as after a scan.
        for work in works {
            neo_scanner::dedupe_work(&mut tx, work, &order).await?;
        }
        tx.commit().await?;
        #[cfg(test)]
        library::crash_tests::crash_at("database_committed");
        state.remove_root(&id);
        cleanup_passwords(&state, &data_dir).await;
        Ok(())
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Runs the part of a root change that writes the database, the password
/// file and the registry on a task of its own. A client that disconnects
/// midway drops the request future, which would otherwise stop between
/// those writes and leave them out of step.
pub(super) async fn run_to_end<T: Send + 'static>(
    change: impl Future<Output = Result<T, HttpError>> + Send + 'static,
) -> Result<T, HttpError> {
    tokio::spawn(change).await.map_err(HttpError::internal)?
}

/// The error for an id that is already taken.
fn check_free(state: &AppState, id: &str) -> Result<(), HttpError> {
    if state.roots().iter().any(|r| r.id == id) {
        return Err(root_exists(id));
    }
    Ok(())
}

/// The committed settings and whether the edit changes either saved or live location.
async fn check_editable(
    state: &AppState,
    id: &str,
    settings: &StoredSettings,
    has_password: bool,
    data_dir: &Path,
) -> Result<(StoredSettings, bool), HttpError> {
    let registered = stored_root(state, id)?;
    let stored = db::roots::list_stored(state.db())
        .await?
        .into_iter()
        .find(|row| row.id == id)
        .ok_or_else(|| HttpError::internal(format!("root {id} has no stored settings")))?;
    let saved: StoredSettings =
        serde_json::from_str(&stored.settings).map_err(HttpError::internal)?;
    // A failed COMMIT acknowledgement can leave the registry behind the
    // database. Match the password to its committed address, not that cache.
    let (current, _) = library::stored_entry(data_dir, &stored, state.watch_by_default())
        .map_err(HttpError::internal)?;
    if let Some(url) = remote_url(settings)
        && !has_password
        && !keeps_password(
            &current,
            settings,
            url,
            &saved
                .password_path(data_dir, id)
                .map_err(HttpError::internal)?,
        )
    {
        let remote = remote_name(settings).unwrap_or_default();
        return Err(HttpError::invalid_request(format!(
            "{remote} 媒体库需要密码，换了服务器地址时要重新填写"
        )));
    }
    // A lost acknowledgement leaves the old client available to scans.
    // Its digests need invalidation even if this request matches the database.
    let location_changed = !same_location(&current.settings, settings)
        || !same_location(&registered.settings, settings);
    Ok((saved, location_changed))
}

fn read_password(
    settings: &StoredSettings,
    data_dir: &Path,
    id: &str,
) -> Result<String, HttpError> {
    let path = settings
        .password_path(data_dir, id)
        .map_err(HttpError::internal)?;
    std::fs::read_to_string(path).map_err(HttpError::internal)
}

async fn cleanup_passwords(state: &AppState, data_dir: &Path) {
    if let Err(e) =
        library::cleanup_passwords(data_dir, state.db(), &state.0.configured_passwords).await
    {
        tracing::warn!("cannot clean up unreferenced password files: {e:#}");
    }
}

/// Whether new settings point at the same files as the current ones.
fn same_location(current: &RootSettings, new: &StoredSettings) -> bool {
    match (current, new) {
        (RootSettings::Local { path: old, .. }, StoredSettings::Local { path, .. }) => {
            Path::new(old) == path
        }
        (
            RootSettings::OpenList {
                url: old_url,
                base_path: old_base,
                username: old_user,
                ..
            },
            StoredSettings::OpenList {
                url,
                base_path,
                username,
                ..
            },
        ) => {
            library::parse_http_url(old_url) == library::parse_http_url(url)
                && old_base == base_path
                && old_user == username
        }
        (
            RootSettings::WebDav {
                url: old_url,
                base_path: old_base,
                username: old_user,
                ..
            },
            StoredSettings::WebDav {
                url,
                base_path,
                username,
                ..
            },
        ) => {
            library::parse_http_url(old_url) == library::parse_http_url(url)
                && old_base == base_path
                && old_user == username
        }
        _ => false,
    }
}

/// The remote service a root's password is for, or `None` for a local
/// root, which has no password.
fn remote_name(settings: &StoredSettings) -> Option<&'static str> {
    match settings {
        StoredSettings::Local { .. } => None,
        StoredSettings::OpenList { .. } => Some("OpenList"),
        StoredSettings::WebDav { .. } => Some("WebDAV"),
    }
}

fn remote_url(settings: &StoredSettings) -> Option<&str> {
    match settings {
        StoredSettings::Local { .. } => None,
        StoredSettings::OpenList { url, .. } | StoredSettings::WebDav { url, .. } => Some(url),
    }
}

/// Root changes and `kikoeru-neo backup` both take this, so a backup never
/// pairs a root's stored settings with another version of its password.
async fn lock_secrets(data_dir: &Path) -> Result<std::fs::File, HttpError> {
    let data_dir = data_dir.to_owned();
    tokio::task::spawn_blocking(move || library::lock_secrets(&data_dir))
        .await
        .map_err(HttpError::internal)?
        .map_err(|e| HttpError::internal(format!("{e:#}")))
}

/// The registry entry of a root added through the API, or the error for a
/// root that is missing or comes from config.toml.
fn stored_root(state: &AppState, id: &str) -> Result<RootEntry, HttpError> {
    let root = state
        .roots()
        .into_iter()
        .find(|r| r.id == id)
        .ok_or_else(|| {
            HttpError::new(StatusCode::NOT_FOUND, ErrorCode::NotFound, "没有这个媒体库")
        })?;
    if root.source == RootSource::Config {
        return Err(HttpError::new(
            StatusCode::CONFLICT,
            ErrorCode::RootReadOnly,
            format!("媒体库 {id} 写在 config.toml 里，要改请修改配置文件后重启"),
        ));
    }
    Ok(root)
}

/// Whether an edit may go on using the saved password. Only for the same
/// kind of root on the same server: sending it to a new address would let
/// anyone with an admin token read a password the API never returns.
fn keeps_password(
    current: &RootEntry,
    new: &StoredSettings,
    url: &str,
    password_file: &Path,
) -> bool {
    let saved = match (&current.settings, new) {
        (RootSettings::OpenList { url, .. }, StoredSettings::OpenList { .. })
        | (RootSettings::WebDav { url, .. }, StoredSettings::WebDav { .. }) => url,
        _ => return false,
    };
    let same_server = match (library::parse_http_url(saved), library::parse_http_url(url)) {
        (Some(saved), Some(url)) => saved.origin() == url.origin(),
        _ => false,
    };
    same_server && password_file.is_file()
}

fn root_exists(id: &str) -> HttpError {
    HttpError::new(
        StatusCode::CONFLICT,
        ErrorCode::RootExists,
        format!("已经有 id 为 {id} 的媒体库"),
    )
}

fn check_display_name(name: &str) -> Result<String, HttpError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_DISPLAY_NAME_CHARS {
        return Err(HttpError::invalid_request(format!(
            "显示名不能为空，最多 {MAX_DISPLAY_NAME_CHARS} 个字符"
        )));
    }
    Ok(name.to_owned())
}

/// The settings to store and the password, if one was given. A local path
/// must be an existing directory now, because a typo would otherwise only
/// show up as an empty scan.
async fn check_settings(
    input: RootInputSettings,
) -> Result<(StoredSettings, Option<String>), HttpError> {
    match input {
        RootInputSettings::Local {
            path,
            watch,
            auto_scan_interval,
        } => {
            check_interval(RootKind::Local, auto_scan_interval)?;
            let path = PathBuf::from(path);
            if !path.is_absolute() {
                return Err(HttpError::invalid_request("本地媒体库的路径必须是绝对路径"));
            }
            let is_dir = tokio::fs::metadata(&path).await.is_ok_and(|m| m.is_dir());
            if !is_dir {
                return Err(HttpError::invalid_request(format!(
                    "{} 不存在或不是目录",
                    path.display()
                )));
            }
            let settings = StoredSettings::Local {
                path,
                watch,
                auto_scan_interval,
            };
            Ok((settings, None))
        }
        RootInputSettings::OpenList {
            url,
            base_path,
            username,
            password,
            list_concurrency,
            link_rate,
            auto_scan_interval,
            watch,
        } => {
            check_interval(RootKind::OpenList, auto_scan_interval)?;
            check_no_watch(watch)?;
            if library::parse_http_url(&url).is_none() {
                return Err(HttpError::invalid_request("url 必须是 http 或 https 地址"));
            }
            if list_concurrency == Some(0) {
                return Err(HttpError::invalid_request("list_concurrency 必须大于 0"));
            }
            if link_rate.is_some_and(|r| !(r.is_finite() && r > 0.0)) {
                return Err(HttpError::invalid_request("link_rate 必须大于 0"));
            }
            check_password(password.as_deref())?;
            let settings = StoredSettings::OpenList {
                url,
                base_path,
                username,
                password_version: None,
                list_concurrency,
                link_rate,
                auto_scan_interval,
            };
            Ok((settings, password))
        }
        RootInputSettings::WebDav {
            url,
            base_path,
            username,
            password,
            list_concurrency,
            auto_scan_interval,
            watch,
        } => {
            check_interval(RootKind::WebDav, auto_scan_interval)?;
            check_no_watch(watch)?;
            if library::parse_http_url(&url).is_none() {
                return Err(HttpError::invalid_request("url 必须是 http 或 https 地址"));
            }
            if list_concurrency == Some(0) {
                return Err(HttpError::invalid_request("list_concurrency 必须大于 0"));
            }
            check_password(password.as_deref())?;
            let settings = StoredSettings::WebDav {
                url,
                base_path,
                username,
                password_version: None,
                list_concurrency,
                auto_scan_interval,
            };
            Ok((settings, password))
        }
    }
}

fn check_no_watch(watch: Option<bool>) -> Result<(), HttpError> {
    if watch == Some(true) {
        return Err(HttpError::invalid_request(
            "只有本地媒体库能监听目录，OpenList 和 WebDAV 媒体库请改用 auto_scan_interval",
        ));
    }
    Ok(())
}

fn check_interval(kind: RootKind, secs: Option<u32>) -> Result<(), HttpError> {
    let Some(secs) = secs else { return Ok(()) };
    crate::autoscan::check_interval(kind, secs).map_err(|_| {
        HttpError::invalid_request(match kind {
            RootKind::Local => "本地媒体库的 auto_scan_interval 要在 300 到 2592000 秒之间",
            RootKind::OpenList => {
                "OpenList 媒体库的 auto_scan_interval 要在 1800 到 2592000 秒之间"
            }
            RootKind::WebDav => "WebDAV 媒体库的 auto_scan_interval 要在 1800 到 2592000 秒之间",
        })
    })
}

fn check_password(password: Option<&str>) -> Result<(), HttpError> {
    if password == Some("") {
        return Err(HttpError::invalid_request("密码不能为空"));
    }
    // A password file loses its trailing line break when read, so a
    // password that ends in one would stop working after a restart.
    if password.is_some_and(|p| p.contains(['\r', '\n'])) {
        return Err(HttpError::invalid_request("密码不能包含换行"));
    }
    Ok(())
}

/// Builds the storage for new settings and, for OpenList and WebDAV
/// unless the request skips it, reaches `base_path` once with the account:
/// an OpenList listing, or a `Depth: 0` `PROPFIND` that must find a
/// collection.
async fn try_settings(
    state: &AppState,
    id: &str,
    settings: &StoredSettings,
    password: Option<&str>,
    skip_check: Option<bool>,
) -> Result<Arc<dyn Storage>, HttpError> {
    let storage = build(state.data_dir()?.root(), id, settings, password)?;
    let Some(remote) = remote_name(settings) else {
        return Ok(storage);
    };
    if skip_check.unwrap_or(false) {
        return Ok(storage);
    }
    let reached = match settings {
        StoredSettings::WebDav { .. } => match storage.stat(&RawPath::root()).await {
            Ok(entry) if entry.kind == EntryKind::Dir => Ok(()),
            Ok(_) => Err(StorageError::Upstream("base_path 不是目录".into())),
            Err(e) => Err(e),
        },
        _ => storage.list(&RawPath::root()).await.map(|_| ()),
    };
    match reached {
        Ok(()) => Ok(storage),
        Err(StorageError::NotFound) => Err(HttpError::new(
            StatusCode::BAD_GATEWAY,
            ErrorCode::UpstreamError,
            format!("{remote} 上找不到 base_path"),
        )),
        Err(StorageError::Unavailable(cause)) => Err(HttpError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::StorageUnavailable,
            format!("连不上 {remote}：{cause}"),
        )),
        Err(StorageError::Upstream(cause)) => Err(HttpError::new(
            StatusCode::BAD_GATEWAY,
            ErrorCode::UpstreamError,
            format!("{remote} 返回错误：{cause}"),
        )),
    }
}

/// The storage for settings, with `password` or else the saved one.
fn build(
    data_dir: &Path,
    id: &str,
    settings: &StoredSettings,
    password: Option<&str>,
) -> Result<Arc<dyn Storage>, HttpError> {
    let backend = settings
        .backend(library::password_path(data_dir, id))
        .map_err(|e| HttpError::invalid_request(format!("{e:#}")))?;
    library::build_storage_with(id, &backend, password)
        .map_err(|e| HttpError::invalid_request(format!("{e:#}")))
}

/// Puts a saved root into the registry and returns its entry.
fn publish(
    state: &AppState,
    data_dir: &Path,
    stored: &StoredRoot,
    storage: Arc<dyn Storage>,
) -> Result<RootEntry, HttpError> {
    let (entry, _) = library::stored_entry(data_dir, stored, state.watch_by_default())
        .map_err(|e| HttpError::internal(format!("{e:#}")))?;
    state.put_root(entry.clone(), Some(storage));
    Ok(entry)
}
