use super::{check_bearer_auth, error_response, json_response, with_rust_header, AppState};
use crate::access::{verified_share, ShareScope, VerifiedShare};
use crate::collection::{handle_send, SendEnvelope, SendPayload};
use axum::{
    body::Body,
    extract::{rejection::JsonRejection, OriginalUri, Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Duration, TimeZone, Utc};
use rand::{distributions::Alphanumeric, Rng};
use serde::{de::DeserializeOwned, Deserialize, Deserializer};
use serde_json::{json, Map, Value};
use sqlx::{postgres::PgRow, PgPool, Row};
use std::collections::HashMap;
use url::Url;
use uuid::Uuid;

const DEFAULT_PAGE_SIZE: i64 = 20;
const MAX_PAGE_SIZE: i64 = 500;
const LINK_SHARE_TYPE: i32 = 2;
const PIXEL_SHARE_TYPE: i32 = 3;
const LINK_EVENT_TYPE: i32 = 3;
const PIXEL_EVENT_TYPE: i32 = 4;
const PIXEL_GIF: &[u8] = &[
    71, 73, 70, 56, 57, 97, 1, 0, 1, 0, 128, 0, 0, 255, 255, 255, 0, 0, 0, 33, 249, 4, 1, 0, 0, 0,
    0, 44, 0, 0, 0, 0, 1, 0, 1, 0, 0, 2, 2, 68, 1, 0, 59,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    Link,
    Pixel,
}

impl Kind {
    fn api_path(self) -> &'static str {
        match self {
            Self::Link => "/api/links",
            Self::Pixel => "/api/pixels",
        }
    }

    fn table(self) -> &'static str {
        match self {
            Self::Link => "link",
            Self::Pixel => "pixel",
        }
    }

    fn id_column(self) -> &'static str {
        match self {
            Self::Link => "link_id",
            Self::Pixel => "pixel_id",
        }
    }

    fn share_type(self) -> i32 {
        match self {
            Self::Link => LINK_SHARE_TYPE,
            Self::Pixel => PIXEL_SHARE_TYPE,
        }
    }

    fn event_type(self) -> i32 {
        match self {
            Self::Link => LINK_EVENT_TYPE,
            Self::Pixel => PIXEL_EVENT_TYPE,
        }
    }

    fn select(self) -> &'static str {
        match self {
            Self::Link => {
                r#"SELECT link_id AS id, name, url, slug, user_id, team_id,
                          created_at, updated_at, deleted_at
                   FROM link"#
            }
            Self::Pixel => {
                r#"SELECT pixel_id AS id, name, slug, user_id, team_id,
                          created_at, updated_at, deleted_at
                   FROM pixel"#
            }
        }
    }

    fn search_clause(self) -> &'static str {
        match self {
            Self::Link => {
                r#"($2::text IS NULL OR name ILIKE $2 ESCAPE '\' OR url ILIKE $2 ESCAPE '\' OR slug ILIKE $2 ESCAPE '\')"#
            }
            Self::Pixel => {
                r#"($2::text IS NULL OR name ILIKE $2 ESCAPE '\' OR slug ILIKE $2 ESCAPE '\')"#
            }
        }
    }

    fn order_column(self, value: Option<&str>) -> (&'static str, Option<&'static str>) {
        match (self, value) {
            (_, Some("name")) => ("name", Some("name")),
            (_, Some("slug")) => ("slug", Some("slug")),
            (Self::Link, Some("url")) => ("url", Some("url")),
            (_, Some("createdAt")) => ("created_at", Some("createdAt")),
            _ => ("created_at", None),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Access {
    Read,
    Write,
    Delete,
}

#[derive(Clone, Debug)]
struct UserAuth {
    id: Uuid,
    role: String,
}

#[derive(Debug)]
enum ReadAuth {
    User(UserAuth),
    Share(VerifiedShare),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateLinkInput {
    name: String,
    url: String,
    slug: String,
    #[serde(default)]
    team_id: Option<Uuid>,
    #[serde(default)]
    id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreatePixelInput {
    name: String,
    slug: String,
    #[serde(default)]
    team_id: Option<Uuid>,
    #[serde(default)]
    id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateLinkInput {
    #[serde(default, deserialize_with = "deserialize_present")]
    name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present")]
    url: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present")]
    slug: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdatePixelInput {
    #[serde(default, deserialize_with = "deserialize_present")]
    name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present")]
    slug: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateShareInput {
    name: String,
    #[serde(default, deserialize_with = "deserialize_present")]
    parameters: Option<Map<String, Value>>,
}

#[derive(Debug)]
struct ListParams {
    page: i64,
    page_size: i64,
    search: Option<String>,
    order_by: Option<String>,
    descending: bool,
}

pub(super) fn register_routes(router: Router<AppState>) -> Router<AppState> {
    router
        .route("/api/links", get(list_links).post(create_link))
        .route("/api/links/charts", get(link_charts))
        .route(
            "/api/links/{entity_id}",
            get(get_link).post(update_link).delete(delete_link),
        )
        .route(
            "/api/links/{entity_id}/shares",
            get(list_link_shares).post(create_link_share),
        )
        .route("/api/pixels", get(list_pixels).post(create_pixel))
        .route("/api/pixels/charts", get(pixel_charts))
        .route(
            "/api/pixels/{entity_id}",
            get(get_pixel).post(update_pixel).delete(delete_pixel),
        )
        .route(
            "/api/pixels/{entity_id}/shares",
            get(list_pixel_shares).post(create_pixel_share),
        )
        .route("/q/{slug}", get(collect_link))
        .route("/p/{slug}", get(collect_pixel))
}

fn deserialize_present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    T::deserialize(deserializer).map(Some)
}

fn database(state: &AppState) -> Result<&PgPool, Response> {
    state
        .db
        .as_ref()
        .ok_or_else(|| error_response(StatusCode::SERVICE_UNAVAILABLE, "Database unavailable"))
}

async fn bearer(pool: &PgPool, headers: &HeaderMap, path: &str) -> Result<UserAuth, Response> {
    check_bearer_auth(pool, headers, path)
        .await
        .map(|(id, role)| UserAuth { id, role })
        .ok_or_else(|| error_response(StatusCode::UNAUTHORIZED, "Unauthorized"))
}

async fn read_auth(pool: &PgPool, headers: &HeaderMap, path: &str) -> Result<ReadAuth, Response> {
    if let Some((id, role)) = check_bearer_auth(pool, headers, path).await {
        return Ok(ReadAuth::User(UserAuth { id, role }));
    }
    if let Some(share) = verified_share(pool, headers).await? {
        return Ok(ReadAuth::Share(share));
    }
    Err(error_response(StatusCode::UNAUTHORIZED, "Unauthorized"))
}

fn share_allows(scope: &ShareScope, kind: Kind, entity_id: Uuid) -> bool {
    match kind {
        Kind::Link => scope.link_ids.contains(&entity_id),
        Kind::Pixel => scope.pixel_ids.contains(&entity_id),
    }
}

fn team_role_allows(role: &str, access: Access) -> bool {
    match access {
        Access::Read => matches!(
            role,
            "team-owner" | "team-manager" | "team-member" | "team-view-only"
        ),
        Access::Write | Access::Delete => {
            matches!(role, "team-owner" | "team-manager" | "team-member")
        }
    }
}

async fn user_allows(
    pool: &PgPool,
    user: &UserAuth,
    kind: Kind,
    entity_id: Uuid,
    access: Access,
) -> Result<bool, sqlx::Error> {
    if user.role == "admin" {
        return Ok(true);
    }
    let query = format!(
        r#"SELECT e.user_id, tu.role AS team_role
            FROM {} e
            LEFT JOIN team_user tu ON tu.team_id = e.team_id AND tu.user_id = $2
            WHERE e.{} = $1"#,
        kind.table(),
        kind.id_column()
    );
    let row = sqlx::query(&query)
        .bind(entity_id)
        .bind(user.id)
        .fetch_optional(pool)
        .await?;
    Ok(row.is_some_and(|row| {
        row.get::<Option<Uuid>, _>("user_id") == Some(user.id)
            || row
                .get::<Option<String>, _>("team_role")
                .is_some_and(|role| team_role_allows(&role, access))
    }))
}

async fn read_allows(
    pool: &PgPool,
    auth: &ReadAuth,
    kind: Kind,
    entity_id: Uuid,
) -> Result<bool, sqlx::Error> {
    match auth {
        ReadAuth::User(user) => user_allows(pool, user, kind, entity_id, Access::Read).await,
        ReadAuth::Share(share) => Ok(share_allows(&share.scope, kind, entity_id)),
    }
}

async fn can_create(
    pool: &PgPool,
    user: &UserAuth,
    team_id: Option<Uuid>,
) -> Result<bool, sqlx::Error> {
    if user.role == "admin" {
        return Ok(true);
    }
    if user.role != "user" {
        return Ok(false);
    }
    let Some(team_id) = team_id else {
        return Ok(true);
    };
    let role = sqlx::query_scalar::<_, String>(
        "SELECT role FROM team_user WHERE team_id = $1 AND user_id = $2",
    )
    .bind(team_id)
    .bind(user.id)
    .fetch_optional(pool)
    .await?;
    Ok(role.is_some_and(|role| team_role_allows(&role, Access::Write)))
}

fn chars_between(value: &str, min: usize, max: usize) -> bool {
    let len = value.chars().count();
    (min..=max).contains(&len)
}

fn validate_name(name: &str) -> bool {
    name.chars().count() <= 100
}

fn validate_slug(slug: &str) -> bool {
    chars_between(slug, 8, 100)
}

fn validate_url(value: &str) -> Option<String> {
    let value = value.trim();
    if value.chars().count() > 500 || Url::parse(value).is_err() {
        return None;
    }
    Some(value.to_string())
}

fn parse_uuid(value: &str) -> Result<Uuid, Response> {
    Uuid::parse_str(value)
        .map_err(|_| error_response(StatusCode::BAD_REQUEST, "Invalid resource ID"))
}

fn json_body<T>(body: Result<Json<T>, JsonRejection>) -> Result<T, Response> {
    body.map(|Json(value)| value)
        .map_err(|_| error_response(StatusCode::BAD_REQUEST, "Invalid request body"))
}

fn parse_positive(params: &HashMap<String, String>, key: &str) -> Result<Option<i64>, Response> {
    let Some(value) = params.get(key) else {
        return Ok(None);
    };
    value
        .parse::<i64>()
        .ok()
        .filter(|value| *value > 0)
        .map(Some)
        .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, format!("Invalid {key}")))
}

fn parse_list_params(params: &HashMap<String, String>, kind: Kind) -> Result<ListParams, Response> {
    let page = parse_positive(params, "page")?.unwrap_or(1);
    let page_size = parse_positive(params, "pageSize")?
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .min(MAX_PAGE_SIZE);
    let _ = parse_positive(params, "maxResults")?;
    let descending = match params.get("sortDescending").map(String::as_str) {
        None | Some("false") => false,
        Some("true") => true,
        Some(_) => {
            return Err(error_response(
                StatusCode::BAD_REQUEST,
                "Invalid sortDescending",
            ))
        }
    };
    let (_, allowed_order) = kind.order_column(params.get("orderBy").map(String::as_str));
    Ok(ListParams {
        page,
        page_size,
        search: params
            .get("search")
            .cloned()
            .filter(|value| !value.is_empty()),
        order_by: allowed_order.map(str::to_string),
        descending: allowed_order.is_some() && descending,
    })
}

fn search_pattern(value: Option<&str>) -> Option<String> {
    value.map(|value| {
        let escaped = value
            .replace('\\', r"\\")
            .replace('%', r"\%")
            .replace('_', r"\_");
        format!("%{escaped}%")
    })
}

fn entity_json(kind: Kind, row: &PgRow) -> Value {
    let mut object = Map::new();
    object.insert("id".to_string(), json!(row.get::<Uuid, _>("id")));
    object.insert("name".to_string(), json!(row.get::<String, _>("name")));
    object.insert("slug".to_string(), json!(row.get::<String, _>("slug")));
    object.insert(
        "userId".to_string(),
        json!(row.get::<Option<Uuid>, _>("user_id")),
    );
    object.insert(
        "teamId".to_string(),
        json!(row.get::<Option<Uuid>, _>("team_id")),
    );
    object.insert(
        "createdAt".to_string(),
        json!(row.get::<Option<DateTime<Utc>>, _>("created_at")),
    );
    object.insert(
        "updatedAt".to_string(),
        json!(row.get::<Option<DateTime<Utc>>, _>("updated_at")),
    );
    object.insert(
        "deletedAt".to_string(),
        json!(row.get::<Option<DateTime<Utc>>, _>("deleted_at")),
    );
    if kind == Kind::Link {
        object.insert("url".to_string(), json!(row.get::<String, _>("url")));
    }
    Value::Object(object)
}

fn share_json(row: &PgRow) -> Value {
    let parameters = row
        .get::<String, _>("parameters")
        .parse::<Value>()
        .unwrap_or_else(|_| json!({}));
    json!({
        "id": row.get::<Uuid, _>("share_id"),
        "entityId": row.get::<Uuid, _>("entity_id"),
        "name": row.get::<String, _>("name"),
        "shareType": row.get::<i32, _>("share_type"),
        "slug": row.get::<String, _>("slug"),
        "parameters": parameters,
        "createdAt": row.get::<Option<DateTime<Utc>>, _>("created_at"),
        "updatedAt": row.get::<Option<DateTime<Utc>>, _>("updated_at")
    })
}

async fn list_entities(
    state: AppState,
    headers: HeaderMap,
    params: HashMap<String, String>,
    kind: Kind,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let user = match bearer(pool, &headers, kind.api_path()).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let params = match parse_list_params(&params, kind) {
        Ok(params) => params,
        Err(response) => return response,
    };
    let (order_column, _) = kind.order_column(params.order_by.as_deref());
    let direction = if params.descending { "DESC" } else { "ASC" };
    let search = search_pattern(params.search.as_deref());
    let offset = (params.page - 1).saturating_mul(params.page_size);
    let query = format!(
        "{} WHERE user_id = $1 AND deleted_at IS NULL AND {} ORDER BY {} {} LIMIT $3 OFFSET $4",
        kind.select(),
        kind.search_clause(),
        order_column,
        direction
    );
    let count_query = format!(
        "SELECT COUNT(*)::bigint AS count FROM {} WHERE user_id = $1 AND deleted_at IS NULL AND {}",
        kind.table(),
        kind.search_clause()
    );
    let rows = match sqlx::query(&query)
        .bind(user.id)
        .bind(&search)
        .bind(params.page_size)
        .bind(offset)
        .fetch_all(pool)
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to list {}: {error}", kind.table()),
            )
        }
    };
    let count = match sqlx::query_scalar::<_, i64>(&count_query)
        .bind(user.id)
        .bind(&search)
        .fetch_one(pool)
        .await
    {
        Ok(count) => count,
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to count {}: {error}", kind.table()),
            )
        }
    };
    let mut result = Map::new();
    result.insert(
        "data".to_string(),
        Value::Array(
            rows.iter()
                .map(|row| entity_json(kind, row))
                .collect::<Vec<_>>(),
        ),
    );
    result.insert("count".to_string(), json!(count));
    result.insert("page".to_string(), json!(params.page));
    result.insert("pageSize".to_string(), json!(params.page_size));
    if let Some(order_by) = params.order_by {
        result.insert("orderBy".to_string(), json!(order_by));
    }
    if let Some(search) = params.search {
        result.insert("search".to_string(), json!(search));
    }
    json_response(Value::Object(result))
}

async fn list_links(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    list_entities(state, headers, params, Kind::Link).await
}

async fn list_pixels(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    list_entities(state, headers, params, Kind::Pixel).await
}

async fn entity_id_in_use(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    entity_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(entity_id.to_string())
        .execute(&mut **tx)
        .await?;
    sqlx::query_scalar::<_, bool>(
        r#"SELECT EXISTS(
             SELECT 1 FROM website WHERE website_id = $1
             UNION ALL SELECT 1 FROM link WHERE link_id = $1
             UNION ALL SELECT 1 FROM pixel WHERE pixel_id = $1
             UNION ALL SELECT 1 FROM board WHERE board_id = $1
             UNION ALL SELECT 1 FROM "user" WHERE user_id = $1
           )"#,
    )
    .bind(entity_id)
    .fetch_one(&mut **tx)
    .await
}

fn unique_slug_error(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|error| error.code())
        .as_deref()
        == Some("23505")
        && error
            .as_database_error()
            .and_then(|error| error.constraint())
            .is_some_and(|constraint| constraint.contains("slug"))
}

async fn insert_entity(
    pool: &PgPool,
    kind: Kind,
    user: &UserAuth,
    id: Uuid,
    name: &str,
    slug: &str,
    url: Option<&str>,
    team_id: Option<Uuid>,
) -> Result<PgRow, Response> {
    let mut tx = pool.begin().await.map_err(|error| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to start create transaction: {error}"),
        )
    })?;
    if entity_id_in_use(&mut tx, id).await.map_err(|error| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to validate resource ID: {error}"),
        )
    })? {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "ID already in use.",
        ));
    }
    let user_id = team_id.is_none().then_some(user.id);
    let result = match kind {
        Kind::Link => {
            sqlx::query(
                r#"INSERT INTO link
                     (link_id, name, url, slug, user_id, team_id, created_at, updated_at)
                   VALUES ($1, $2, $3, $4, $5, $6, NOW(), NOW())
                   RETURNING link_id AS id, name, url, slug, user_id, team_id,
                             created_at, updated_at, deleted_at"#,
            )
            .bind(id)
            .bind(name)
            .bind(url.expect("link URL was validated"))
            .bind(slug)
            .bind(user_id)
            .bind(team_id)
            .fetch_one(&mut *tx)
            .await
        }
        Kind::Pixel => {
            sqlx::query(
                r#"INSERT INTO pixel
                     (pixel_id, name, slug, user_id, team_id, created_at, updated_at)
                   VALUES ($1, $2, $3, $4, $5, NOW(), NOW())
                   RETURNING pixel_id AS id, name, slug, user_id, team_id,
                             created_at, updated_at, deleted_at"#,
            )
            .bind(id)
            .bind(name)
            .bind(slug)
            .bind(user_id)
            .bind(team_id)
            .fetch_one(&mut *tx)
            .await
        }
    };
    let row = match result {
        Ok(row) => row,
        Err(error) if unique_slug_error(&error) => {
            return Err(error_response(
                StatusCode::BAD_REQUEST,
                "That slug is already taken.",
            ))
        }
        Err(error) => {
            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to create {}: {error}", kind.table()),
            ))
        }
    };
    tx.commit().await.map_err(|error| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to commit create transaction: {error}"),
        )
    })?;
    Ok(row)
}

async fn create_link(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<CreateLinkInput>, JsonRejection>,
) -> Response {
    let input = match json_body(body) {
        Ok(input) => input,
        Err(response) => return response,
    };
    let Some(url) = validate_url(&input.url) else {
        return error_response(StatusCode::BAD_REQUEST, "Invalid URL.");
    };
    create_entity(
        state,
        headers,
        Kind::Link,
        input.id,
        input.name,
        input.slug,
        Some(url),
        input.team_id,
    )
    .await
}

async fn create_pixel(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<CreatePixelInput>, JsonRejection>,
) -> Response {
    let input = match json_body(body) {
        Ok(input) => input,
        Err(response) => return response,
    };
    create_entity(
        state,
        headers,
        Kind::Pixel,
        input.id,
        input.name,
        input.slug,
        None,
        input.team_id,
    )
    .await
}

async fn create_entity(
    state: AppState,
    headers: HeaderMap,
    kind: Kind,
    id: Option<Uuid>,
    name: String,
    slug: String,
    url: Option<String>,
    team_id: Option<Uuid>,
) -> Response {
    if !validate_name(&name) || !validate_slug(&slug) {
        return error_response(StatusCode::BAD_REQUEST, "Invalid request body");
    }
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let user = match bearer(pool, &headers, kind.api_path()).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match can_create(pool, &user, team_id).await {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::UNAUTHORIZED, "Unauthorized"),
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to verify create access: {error}"),
            )
        }
    }
    let id = id.unwrap_or_else(Uuid::new_v4);
    match insert_entity(pool, kind, &user, id, &name, &slug, url.as_deref(), team_id).await {
        Ok(row) => json_response(entity_json(kind, &row)),
        Err(response) => response,
    }
}

async fn fetch_entity(
    pool: &PgPool,
    kind: Kind,
    entity_id: Uuid,
) -> Result<Option<PgRow>, sqlx::Error> {
    let query = format!("{} WHERE {} = $1", kind.select(), kind.id_column());
    sqlx::query(&query)
        .bind(entity_id)
        .fetch_optional(pool)
        .await
}

async fn get_entity(
    state: AppState,
    headers: HeaderMap,
    entity_id: String,
    kind: Kind,
) -> Response {
    let entity_id = match parse_uuid(&entity_id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let auth = match read_auth(pool, &headers, kind.api_path()).await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    match read_allows(pool, &auth, kind, entity_id).await {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::UNAUTHORIZED, "Unauthorized"),
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to verify read access: {error}"),
            )
        }
    }
    match fetch_entity(pool, kind, entity_id).await {
        Ok(Some(row)) => json_response(entity_json(kind, &row)),
        Ok(None) => json_response(Value::Null),
        Err(error) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to load {}: {error}", kind.table()),
        ),
    }
}

async fn get_link(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(entity_id): Path<String>,
) -> Response {
    get_entity(state, headers, entity_id, Kind::Link).await
}

async fn get_pixel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(entity_id): Path<String>,
) -> Response {
    get_entity(state, headers, entity_id, Kind::Pixel).await
}

async fn update_link(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(entity_id): Path<String>,
    body: Result<Json<UpdateLinkInput>, JsonRejection>,
) -> Response {
    let input = match json_body(body) {
        Ok(input) => input,
        Err(response) => return response,
    };
    let url = match input.url {
        Some(value) => match validate_url(&value) {
            Some(value) => Some(value),
            None => return error_response(StatusCode::BAD_REQUEST, "Invalid URL."),
        },
        None => None,
    };
    update_entity(
        state,
        headers,
        entity_id,
        Kind::Link,
        input.name,
        input.slug,
        url,
    )
    .await
}

async fn update_pixel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(entity_id): Path<String>,
    body: Result<Json<UpdatePixelInput>, JsonRejection>,
) -> Response {
    let input = match json_body(body) {
        Ok(input) => input,
        Err(response) => return response,
    };
    update_entity(
        state,
        headers,
        entity_id,
        Kind::Pixel,
        input.name,
        input.slug,
        None,
    )
    .await
}

async fn update_entity(
    state: AppState,
    headers: HeaderMap,
    entity_id: String,
    kind: Kind,
    name: Option<String>,
    slug: Option<String>,
    url: Option<String>,
) -> Response {
    if name.as_deref().is_some_and(|value| !validate_name(value))
        || slug.as_deref().is_some_and(|value| !validate_slug(value))
    {
        return error_response(StatusCode::BAD_REQUEST, "Invalid request body");
    }
    let entity_id = match parse_uuid(&entity_id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let user = match bearer(pool, &headers, kind.api_path()).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match user_allows(pool, &user, kind, entity_id, Access::Write).await {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::UNAUTHORIZED, "Unauthorized"),
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to verify update access: {error}"),
            )
        }
    }
    let result = match kind {
        Kind::Link => {
            sqlx::query(
                r#"UPDATE link SET
                     name = COALESCE($2, name),
                     slug = COALESCE($3, slug),
                     url = COALESCE($4, url),
                     updated_at = NOW()
                   WHERE link_id = $1
                   RETURNING link_id AS id, name, url, slug, user_id, team_id,
                             created_at, updated_at, deleted_at"#,
            )
            .bind(entity_id)
            .bind(name)
            .bind(slug)
            .bind(url)
            .fetch_optional(pool)
            .await
        }
        Kind::Pixel => {
            sqlx::query(
                r#"UPDATE pixel SET
                     name = COALESCE($2, name),
                     slug = COALESCE($3, slug),
                     updated_at = NOW()
                   WHERE pixel_id = $1
                   RETURNING pixel_id AS id, name, slug, user_id, team_id,
                             created_at, updated_at, deleted_at"#,
            )
            .bind(entity_id)
            .bind(name)
            .bind(slug)
            .fetch_optional(pool)
            .await
        }
    };
    match result {
        Ok(Some(row)) => json_response(entity_json(kind, &row)),
        Ok(None) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "Resource not found"),
        Err(error) if unique_slug_error(&error) => {
            error_response(StatusCode::BAD_REQUEST, "That slug is already taken.")
        }
        Err(error) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to update {}: {error}", kind.table()),
        ),
    }
}

async fn delete_link(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(entity_id): Path<String>,
) -> Response {
    delete_entity(state, headers, entity_id, Kind::Link).await
}

async fn delete_pixel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(entity_id): Path<String>,
) -> Response {
    delete_entity(state, headers, entity_id, Kind::Pixel).await
}

async fn delete_entity(
    state: AppState,
    headers: HeaderMap,
    entity_id: String,
    kind: Kind,
) -> Response {
    let entity_id = match parse_uuid(&entity_id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let user = match bearer(pool, &headers, kind.api_path()).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match user_allows(pool, &user, kind, entity_id, Access::Delete).await {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::UNAUTHORIZED, "Unauthorized"),
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to verify delete access: {error}"),
            )
        }
    }
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to start delete transaction: {error}"),
            )
        }
    };
    if let Err(error) = sqlx::query("DELETE FROM share WHERE entity_id = $1")
        .bind(entity_id)
        .execute(&mut *tx)
        .await
    {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to delete resource shares: {error}"),
        );
    }
    let query = format!(
        "DELETE FROM {} WHERE {} = $1",
        kind.table(),
        kind.id_column()
    );
    if let Err(error) = sqlx::query(&query).bind(entity_id).execute(&mut *tx).await {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to delete {}: {error}", kind.table()),
        );
    }
    if let Err(error) = tx.commit().await {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to commit delete transaction: {error}"),
        );
    }
    json_response(json!({ "ok": true }))
}

fn validate_share_query(params: &HashMap<String, String>) -> Result<(i64, i64), Response> {
    for key in ["segment", "cohort"] {
        if params
            .get(key)
            .is_some_and(|value| Uuid::parse_str(value).is_err())
        {
            return Err(error_response(
                StatusCode::BAD_REQUEST,
                format!("Invalid {key}"),
            ));
        }
    }
    if params.get("eventType").is_some_and(|value| {
        value
            .parse::<i64>()
            .ok()
            .filter(|value| *value > 0)
            .is_none()
    }) {
        return Err(error_response(StatusCode::BAD_REQUEST, "Invalid eventType"));
    }
    if params
        .get("match")
        .is_some_and(|value| !matches!(value.as_str(), "all" | "any"))
    {
        return Err(error_response(StatusCode::BAD_REQUEST, "Invalid match"));
    }
    let page = parse_positive(params, "page")?.unwrap_or(1);
    let page_size = parse_positive(params, "pageSize")?
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .min(MAX_PAGE_SIZE);
    let _ = parse_positive(params, "maxResults")?;
    Ok((page, page_size))
}

async fn list_shares(
    state: AppState,
    headers: HeaderMap,
    entity_id: String,
    params: HashMap<String, String>,
    kind: Kind,
) -> Response {
    let entity_id = match parse_uuid(&entity_id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let (page, page_size) = match validate_share_query(&params) {
        Ok(values) => values,
        Err(response) => return response,
    };
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let auth = match read_auth(pool, &headers, kind.api_path()).await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    match read_allows(pool, &auth, kind, entity_id).await {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::UNAUTHORIZED, "Unauthorized"),
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to verify share read access: {error}"),
            )
        }
    }
    let offset = (page - 1).saturating_mul(page_size);
    let rows = match sqlx::query(
        r#"SELECT share_id, entity_id, name, share_type, slug,
                  parameters::text AS parameters, created_at, updated_at
           FROM share
           WHERE entity_id = $1
           ORDER BY created_at DESC
           LIMIT $2 OFFSET $3"#,
    )
    .bind(entity_id)
    .bind(page_size)
    .bind(offset)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to list shares: {error}"),
            )
        }
    };
    let count = match sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)::bigint FROM share WHERE entity_id = $1",
    )
    .bind(entity_id)
    .fetch_one(pool)
    .await
    {
        Ok(count) => count,
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to count shares: {error}"),
            )
        }
    };
    json_response(json!({
        "data": rows.iter().map(share_json).collect::<Vec<_>>(),
        "count": count,
        "page": page,
        "pageSize": page_size
    }))
}

async fn list_link_shares(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(entity_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    list_shares(state, headers, entity_id, params, Kind::Link).await
}

async fn list_pixel_shares(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(entity_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    list_shares(state, headers, entity_id, params, Kind::Pixel).await
}

async fn create_link_share(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(entity_id): Path<String>,
    body: Result<Json<CreateShareInput>, JsonRejection>,
) -> Response {
    create_share(state, headers, entity_id, body, Kind::Link).await
}

async fn create_pixel_share(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(entity_id): Path<String>,
    body: Result<Json<CreateShareInput>, JsonRejection>,
) -> Response {
    create_share(state, headers, entity_id, body, Kind::Pixel).await
}

async fn create_share(
    state: AppState,
    headers: HeaderMap,
    entity_id: String,
    body: Result<Json<CreateShareInput>, JsonRejection>,
    kind: Kind,
) -> Response {
    let input = match json_body(body) {
        Ok(input) => input,
        Err(response) => return response,
    };
    if input.name.chars().count() > 200 {
        return error_response(StatusCode::BAD_REQUEST, "Invalid request body");
    }
    let entity_id = match parse_uuid(&entity_id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let user = match bearer(pool, &headers, kind.api_path()).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match user_allows(pool, &user, kind, entity_id, Access::Write).await {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::UNAUTHORIZED, "Unauthorized"),
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to verify share create access: {error}"),
            )
        }
    }
    let id = Uuid::new_v4();
    let slug: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(16)
        .map(char::from)
        .collect();
    let parameters = Value::Object(input.parameters.unwrap_or_default());
    let row = match sqlx::query(
        r#"INSERT INTO share
             (share_id, entity_id, name, share_type, slug, parameters, created_at, updated_at)
           VALUES ($1, $2, $3, $4, $5, $6, NOW(), NOW())
           RETURNING share_id, entity_id, name, share_type, slug,
                     parameters::text AS parameters, created_at, updated_at"#,
    )
    .bind(id)
    .bind(entity_id)
    .bind(input.name)
    .bind(kind.share_type())
    .bind(slug)
    .bind(parameters)
    .fetch_one(pool)
    .await
    {
        Ok(row) => row,
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to create share: {error}"),
            )
        }
    };
    json_response(share_json(&row))
}

fn parse_chart_ids(params: &HashMap<String, String>) -> Result<Vec<Uuid>, Response> {
    let ids = params
        .get("ids")
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(Uuid::parse_str)
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()
        .map_err(|_| error_response(StatusCode::BAD_REQUEST, "Invalid resource IDs"))?
        .unwrap_or_default();
    if ids.is_empty() || ids.len() > 20 {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "Invalid resource IDs",
        ));
    }
    Ok(ids)
}

fn parse_ms(
    params: &HashMap<String, String>,
    key: &str,
) -> Result<Option<DateTime<Utc>>, Response> {
    let Some(value) = params.get(key) else {
        return Ok(None);
    };
    let millis = value
        .parse::<i64>()
        .map_err(|_| error_response(StatusCode::BAD_REQUEST, format!("Invalid {key}")))?;
    Utc.timestamp_millis_opt(millis)
        .single()
        .map(Some)
        .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, format!("Invalid {key}")))
}

async fn charts(
    state: AppState,
    headers: HeaderMap,
    params: HashMap<String, String>,
    kind: Kind,
) -> Response {
    let ids = match parse_chart_ids(&params) {
        Ok(ids) => ids,
        Err(response) => return response,
    };
    let start = match parse_ms(&params, "startAt") {
        Ok(value) => value,
        Err(response) => return response,
    };
    let end = match parse_ms(&params, "endAt") {
        Ok(value) => value,
        Err(response) => return response,
    };
    let now = Utc::now();
    let (start, end) = match (start, end) {
        (Some(start), Some(end)) => (start, end),
        _ => (now - Duration::days(7), now),
    };
    if end < start {
        return error_response(StatusCode::BAD_REQUEST, "Invalid date range");
    }
    let timezone = params.get("timezone").map(String::as_str).unwrap_or("UTC");
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let timezone_exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM pg_timezone_names WHERE name = $1)",
    )
    .bind(timezone)
    .fetch_one(pool)
    .await;
    match timezone_exists {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::BAD_REQUEST, "Invalid timezone"),
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to validate timezone: {error}"),
            )
        }
    }
    let auth = match read_auth(pool, &headers, kind.api_path()).await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let mut allowed = Vec::new();
    for id in ids {
        match read_allows(pool, &auth, kind, id).await {
            Ok(true) if !allowed.contains(&id) => allowed.push(id),
            Ok(true) => {}
            Ok(false) => {}
            Err(error) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Unable to verify chart access: {error}"),
                )
            }
        }
    }
    if allowed.is_empty() {
        return json_response(json!({ "data": {} }));
    }
    let rows = match sqlx::query(
        r#"
        WITH bounds AS (
          SELECT
            date_trunc('day', $2::timestamptz AT TIME ZONE $4)
              + floor(extract(hour FROM $2::timestamptz AT TIME ZONE $4) / 12)
                * interval '12 hours' AS start_bucket,
            date_trunc('day', $3::timestamptz AT TIME ZONE $4)
              + floor(extract(hour FROM $3::timestamptz AT TIME ZONE $4) / 12)
                * interval '12 hours' AS end_bucket
        ),
        ids AS (
          SELECT unnest($1::uuid[]) AS entity_id
        ),
        buckets AS (
          SELECT generate_series(start_bucket, end_bucket, interval '12 hours') AS bucket
          FROM bounds
        ),
        counts AS (
          SELECT website_id AS entity_id,
                 date_trunc('day', created_at AT TIME ZONE $4)
                   + floor(extract(hour FROM created_at AT TIME ZONE $4) / 12)
                     * interval '12 hours' AS bucket,
                 COUNT(DISTINCT session_id)::bigint AS sessions
          FROM website_event
          WHERE website_id = ANY($1::uuid[])
            AND event_type = $5
            AND created_at BETWEEN $2 AND $3
          GROUP BY website_id, bucket
        ),
        totals AS (
          SELECT website_id AS entity_id, COUNT(DISTINCT session_id)::bigint AS total
          FROM website_event
          WHERE website_id = ANY($1::uuid[])
            AND event_type = $5
            AND created_at BETWEEN $2 AND $3
          GROUP BY website_id
        )
        SELECT ids.entity_id,
               ARRAY_AGG(COALESCE(counts.sessions, 0) ORDER BY buckets.bucket)::bigint[] AS values,
               COALESCE(totals.total, 0)::bigint AS total
        FROM ids
        CROSS JOIN buckets
        LEFT JOIN counts
          ON counts.entity_id = ids.entity_id AND counts.bucket = buckets.bucket
        LEFT JOIN totals ON totals.entity_id = ids.entity_id
        GROUP BY ids.entity_id, totals.total
        "#,
    )
    .bind(&allowed)
    .bind(start)
    .bind(end)
    .bind(timezone)
    .bind(kind.event_type())
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to load charts: {error}"),
            )
        }
    };
    let mut data = Map::new();
    for row in rows {
        data.insert(
            row.get::<Uuid, _>("entity_id").to_string(),
            json!({
                "values": row.get::<Vec<i64>, _>("values"),
                "total": row.get::<i64, _>("total")
            }),
        );
    }
    json_response(json!({ "data": data }))
}

async fn link_charts(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    charts(state, headers, params, Kind::Link).await
}

async fn pixel_charts(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    charts(state, headers, params, Kind::Pixel).await
}

fn request_url(headers: &HeaderMap, uri: &axum::http::Uri) -> String {
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("http");
    let host = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get(header::HOST))
        .and_then(|value| value.to_str().ok())
        .unwrap_or("localhost");
    format!("{scheme}://{host}{uri}")
}

async fn record_public_event(
    state: &AppState,
    headers: &HeaderMap,
    uri: &axum::http::Uri,
    kind: Kind,
    entity_id: Uuid,
) {
    let mut payload = SendPayload {
        url: Some(request_url(headers, uri)),
        referrer: headers
            .get(header::REFERER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string),
        ..Default::default()
    };
    match kind {
        Kind::Link => payload.link = Some(entity_id.to_string()),
        Kind::Pixel => payload.pixel = Some(entity_id.to_string()),
    }
    let _ = handle_send(
        State(state.clone()),
        headers.clone(),
        Json(SendEnvelope {
            kind: "event".to_string(),
            payload,
        }),
    )
    .await;
}

async fn collect_link(
    State(state): State<AppState>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    Path(slug): Path<String>,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let row =
        match sqlx::query("SELECT link_id, url FROM link WHERE slug = $1 AND deleted_at IS NULL")
            .bind(slug)
            .fetch_optional(pool)
            .await
        {
            Ok(Some(row)) => row,
            Ok(None) => return error_response(StatusCode::NOT_FOUND, "Not found"),
            Err(error) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Unable to load link: {error}"),
                )
            }
        };
    let entity_id = row.get::<Uuid, _>("link_id");
    let url = match Url::parse(&row.get::<String, _>("url")) {
        Ok(url) => url.to_string(),
        Err(_) => return error_response(StatusCode::NOT_FOUND, "Not found"),
    };
    record_public_event(&state, &headers, &uri, Kind::Link, entity_id).await;
    with_rust_header(Redirect::temporary(&url).into_response())
}

async fn collect_pixel(
    State(state): State<AppState>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    Path(slug): Path<String>,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let entity_id = match sqlx::query_scalar::<_, Uuid>(
        "SELECT pixel_id FROM pixel WHERE slug = $1 AND deleted_at IS NULL",
    )
    .bind(slug)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(id)) => id,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Not found"),
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to load pixel: {error}"),
            )
        }
    };
    record_public_event(&state, &headers, &uri, Kind::Pixel, entity_id).await;
    let mut response = Response::new(Body::from(PIXEL_GIF));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("image/gif"));
    response
        .headers_mut()
        .insert(header::CONTENT_LENGTH, HeaderValue::from_static("43"));
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-store, must-revalidate"),
    );
    response
        .headers_mut()
        .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response
        .headers_mut()
        .insert(header::EXPIRES, HeaderValue::from_static("0"));
    with_rust_header(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_trackable_inputs() {
        assert!(validate_slug("abcdefgh"));
        assert!(!validate_slug("short"));
        assert_eq!(
            validate_url(" https://example.com/docs "),
            Some("https://example.com/docs".to_string())
        );
        assert_eq!(
            validate_url("myapp://open"),
            Some("myapp://open".to_string())
        );
        assert_eq!(validate_url("/relative"), None);
    }

    #[test]
    fn role_boundaries_match_node_permissions() {
        assert!(team_role_allows("team-view-only", Access::Read));
        assert!(!team_role_allows("team-view-only", Access::Write));
        assert!(team_role_allows("team-member", Access::Delete));
    }

    #[test]
    fn share_scope_is_entity_specific() {
        let id = Uuid::new_v4();
        let other = Uuid::new_v4();
        let scope = ShareScope {
            link_ids: vec![id],
            ..Default::default()
        };
        assert!(share_allows(&scope, Kind::Link, id));
        assert!(!share_allows(&scope, Kind::Link, other));
        assert!(!share_allows(&scope, Kind::Pixel, other));
    }

    #[test]
    fn routes_register_without_conflicts() {
        let _ = register_routes(Router::new());
    }
}
