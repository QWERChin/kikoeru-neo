use axum::{Json, extract::State, http::StatusCode};
use neo_core::dto::{ErrorCode, Page, WorkDetail, WorkSummary};
use neo_db::{
    detail::{self, DetailError},
    works::{self, Filter, ListQuery, Order, Scope, Sort},
};
use serde::Deserialize;

use super::{
    AppState, HttpError,
    extract::{ApiPath, ApiQuery, CurrentUser},
};

pub const DEFAULT_PAGE_SIZE: u32 = 40;
pub const MAX_PAGE_SIZE: u32 = 100;

/// Parameters arrive as text so a bad value gets a message naming it.
/// Unknown parameters are ignored.
#[derive(Deserialize)]
pub struct Params {
    page: Option<String>,
    page_size: Option<String>,
    sort: Option<String>,
    order: Option<String>,
    seed: Option<String>,
    q: Option<String>,
    tag: Option<String>,
    circle: Option<String>,
    va: Option<String>,
    root: Option<String>,
    include_missing: Option<String>,
}

/// `page` and `page_size` of every work list.
#[derive(Deserialize)]
pub struct PageParams {
    page: Option<String>,
    page_size: Option<String>,
}

fn positive(name: &str, value: Option<&str>, default: u32) -> Result<u32, HttpError> {
    match value {
        None => Ok(default),
        Some(v) => {
            v.parse::<u32>().ok().filter(|n| *n > 0).ok_or_else(|| {
                HttpError::invalid_request(format!("{name} 必须是正整数，收到 {v:?}"))
            })
        }
    }
}

fn id_param(name: &str, value: Option<&str>) -> Result<Option<i64>, HttpError> {
    value
        .map(|v| {
            v.parse::<i64>().ok().filter(|n| *n > 0).ok_or_else(|| {
                HttpError::invalid_request(format!("{name} 必须是正整数，收到 {v:?}"))
            })
        })
        .transpose()
}

pub(crate) fn paging(page: Option<&str>, page_size: Option<&str>) -> Result<(u32, u32), HttpError> {
    let page = positive("page", page, 1)?;
    let page_size = positive("page_size", page_size, DEFAULT_PAGE_SIZE)?.min(MAX_PAGE_SIZE);
    Ok((page, page_size))
}

fn sort(params: &Params) -> Result<Sort, HttpError> {
    Ok(match params.sort.as_deref() {
        None | Some("added") => Sort::Added,
        Some("release") => Sort::Release,
        Some("rating") => Sort::Rating,
        Some("random") => {
            let seed = params
                .seed
                .as_deref()
                .ok_or_else(|| HttpError::invalid_request("sort=random 需要 seed 参数"))?;
            let seed = seed.parse::<u64>().map_err(|_| {
                HttpError::invalid_request(format!("seed 必须是非负整数，收到 {seed:?}"))
            })?;
            // Only the low 32 bits matter; any non-negative integer works.
            Sort::Random {
                seed: (seed % (1 << 32)) as u32,
            }
        }
        Some(v) => {
            return Err(HttpError::invalid_request(format!(
                "sort 只能是 added、release、rating 或 random，收到 {v:?}"
            )));
        }
    })
}

/// Longer text matches no title anyway, and SQLite refuses LIKE patterns
/// over 50,000 bytes with an error that would surface as a 500.
const MAX_QUERY_CHARS: usize = 200;

fn filter(params: &Params) -> Result<Filter, HttpError> {
    for (name, value) in [("q", &params.q), ("va", &params.va)] {
        if value
            .as_deref()
            .is_some_and(|v| v.chars().count() > MAX_QUERY_CHARS)
        {
            return Err(HttpError::invalid_request(format!(
                "{name} 最多 {MAX_QUERY_CHARS} 个字符"
            )));
        }
    }
    let text = |v: &Option<String>| {
        v.as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
    };
    Ok(Filter {
        q: text(&params.q),
        tag: id_param("tag", params.tag.as_deref())?,
        circle: id_param("circle", params.circle.as_deref())?,
        va: text(&params.va),
        root: text(&params.root),
        include_missing: match params.include_missing.as_deref() {
            None | Some("false") => false,
            Some("true") => true,
            Some(v) => {
                return Err(HttpError::invalid_request(format!(
                    "include_missing 只能是 true 或 false，收到 {v:?}"
                )));
            }
        },
    })
}

pub async fn list(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    ApiQuery(params): ApiQuery<Params>,
) -> Result<Json<Page<WorkSummary>>, HttpError> {
    let (page, page_size) = paging(params.page.as_deref(), params.page_size.as_deref())?;
    let order = match params.order.as_deref() {
        None | Some("desc") => Order::Desc,
        Some("asc") => Order::Asc,
        Some(v) => {
            return Err(HttpError::invalid_request(format!(
                "order 只能是 asc 或 desc，收到 {v:?}"
            )));
        }
    };
    let query = ListQuery {
        user_id: user.id,
        scope: Scope::Library,
        sort: sort(&params)?,
        order,
        filter: filter(&params)?,
        limit: page_size,
        offset: u64::from(page - 1) * u64::from(page_size),
    };
    page_of(&state, query, page).await
}

/// Favorites or history: the user's own lists, in their fixed order.
pub async fn user_list(
    state: &AppState,
    user_id: i64,
    scope: Scope,
    params: &PageParams,
) -> Result<Json<Page<WorkSummary>>, HttpError> {
    let (page, page_size) = paging(params.page.as_deref(), params.page_size.as_deref())?;
    let query = ListQuery {
        user_id,
        scope,
        sort: Sort::Added,
        order: Order::Desc,
        filter: Filter::default(),
        limit: page_size,
        offset: u64::from(page - 1) * u64::from(page_size),
    };
    page_of(state, query, page).await
}

async fn page_of(
    state: &AppState,
    query: ListQuery,
    page: u32,
) -> Result<Json<Page<WorkSummary>>, HttpError> {
    let db = state.db();
    let total = works::count(db, &query).await?;
    let items = works::list(db, &query).await.map_err(HttpError::internal)?;
    Ok(Json(Page {
        items,
        page,
        page_size: query.limit,
        total: total.try_into().map_err(HttpError::internal)?,
    }))
}

/// Work IDs in paths are positive integers. Anything else names no work,
/// so it gets the same 404 as an unknown ID.
pub fn work_id(raw: &str) -> Result<i64, HttpError> {
    raw.parse::<i64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(HttpError::work_not_found)
}

pub async fn detail(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    ApiPath(id): ApiPath<String>,
) -> Result<Json<WorkDetail>, HttpError> {
    let id = work_id(&id)?;
    detail::get(state.db(), user.id, id, user.is_admin)
        .await
        .map_err(detail_error)?
        .map(Json)
        .ok_or_else(HttpError::work_not_found)
}

pub(crate) fn detail_error(e: DetailError) -> HttpError {
    match e {
        DetailError::TooManyTracks(n) => HttpError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            ErrorCode::TooLarge,
            format!(
                "这部作品有 {n} 个文件，超过了 {} 个的上限",
                detail::MAX_TRACKS
            ),
        ),
        e => HttpError::internal(e),
    }
}
