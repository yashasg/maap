use super::{
    check_bearer_auth, error_response, json_response, page_params as paging, parse_ms_param,
    status_response, AppState,
};
use crate::access::{Access, ShareDataVisibility, ShareSection};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::{delete, get, post},
    Json, Router,
};
use chrono::{DateTime, Datelike, Duration, TimeZone, Timelike, Utc};
use rand::{distributions::Alphanumeric, Rng};
use serde_json::{json, Map, Value};
use sqlx::{postgres::PgRow, PgPool, Row};
use std::collections::HashMap;
use uuid::Uuid;

pub fn register_dashboard_routes(router: Router<AppState>) -> Router<AppState> {
    router
        .route(
            "/api/auth/verify",
            get(handle_auth_verify).post(handle_auth_verify),
        )
        .route("/api/auth/logout", post(handle_auth_logout))
        .route("/api/auth/sso", post(handle_sso))
        .route("/api/auth/subscription", get(handle_auth_subscription))
        .route("/api/me", get(handle_me))
        .route("/api/me/password", post(handle_change_password))
        .route("/api/me/websites", get(handle_me_websites))
        .route(
            "/api/me/api-keys",
            get(handle_api_keys).post(super::handle_create_api_key),
        )
        .route("/api/me/api-keys/{key_id}", delete(handle_delete_api_key))
        .route(
            "/api/dashboard",
            get(handle_get_dashboard).post(handle_upsert_dashboard),
        )
        .route(
            "/api/websites",
            get(handle_websites).post(super::handle_create_website),
        )
        .route("/api/websites/charts", get(handle_website_charts))
        .route(
            "/api/websites/{website_id}",
            get(handle_get_website)
                .post(handle_update_website)
                .delete(handle_delete_website),
        )
        .route(
            "/api/websites/{website_id}/reset",
            post(handle_reset_website),
        )
        .route(
            "/api/websites/{website_id}/shares",
            get(handle_website_shares).post(handle_create_website_share),
        )
        .route(
            "/api/websites/{website_id}/annotations",
            get(handle_annotations).post(handle_create_annotation),
        )
        .route(
            "/api/websites/{website_id}/annotations/{annotation_id}",
            get(get_annotation)
                .post(update_annotation)
                .delete(handle_delete_annotation),
        )
        .route(
            "/api/websites/{website_id}/segments",
            get(handle_segments).post(handle_create_segment),
        )
        .route(
            "/api/websites/{website_id}/segments/{segment_id}",
            get(handle_get_segment)
                .post(handle_update_segment)
                .delete(handle_delete_segment),
        )
        .route("/api/admin/users", get(handle_admin_users))
        .route("/api/admin/websites", get(handle_admin_websites))
        .route("/api/admin/teams", get(handle_admin_teams))
        .route("/api/users", post(handle_create_user))
        .route(
            "/api/users/{user_id}",
            get(handle_get_user)
                .post(handle_update_user)
                .delete(handle_delete_user),
        )
        .route("/api/users/{user_id}/websites", get(handle_user_websites))
        .route("/telemetry.js", get(telemetry_script))
        .route("/api/scripts/telemetry", get(telemetry_script))
}

fn database(state: &AppState) -> Result<&PgPool, Response> {
    state
        .db
        .as_ref()
        .ok_or_else(|| error_response(StatusCode::SERVICE_UNAVAILABLE, "Database unavailable"))
}

async fn handle_sso(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(r) => return r,
    };
    if let Err(r) = authenticate(pool, &headers, "/api/auth/sso").await {
        return r;
    }
    // The Rust backend is PostgreSQL-only; the Node SSO token store requires Redis.
    let status = StatusCode::INTERNAL_SERVER_ERROR;
    super::with_rust_header(axum::response::IntoResponse::into_response((
        status,
        Json(json!({"error":{"status":500,"message":"Redis is disabled","code":"server-error"}})),
    )))
}

async fn authenticate(
    pool: &PgPool,
    headers: &HeaderMap,
    path: &str,
) -> Result<(Uuid, String), Response> {
    check_bearer_auth(pool, headers, path)
        .await
        .ok_or_else(|| error_response(StatusCode::UNAUTHORIZED, "Unauthorized"))
}

async fn authenticate_admin(
    pool: &PgPool,
    headers: &HeaderMap,
    path: &str,
) -> Result<Uuid, Response> {
    let (user_id, role) = authenticate(pool, headers, path).await?;
    if role != "admin" {
        return Err(error_response(StatusCode::UNAUTHORIZED, "Unauthorized"));
    }
    Ok(user_id)
}

fn json_text(value: Option<String>) -> Value {
    value
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null)
}

fn user_json(row: &PgRow) -> Value {
    json!({
        "id": row.get::<Uuid, _>("user_id"),
        "username": row.get::<String, _>("username"),
        "role": row.get::<String, _>("role"),
        "createdAt": row.get::<Option<DateTime<Utc>>, _>("created_at"),
        "twoFactorRequired": row.try_get::<bool, _>("two_factor_required").unwrap_or(false)
    })
}

fn website_json(row: &PgRow) -> Value {
    json!({
        "id": row.get::<Uuid, _>("website_id"),
        "name": row.get::<String, _>("name"),
        "domain": row.get::<Option<String>, _>("domain"),
        "shareId": row.get::<Option<String>, _>("share_id"),
        "resetAt": row.get::<Option<DateTime<Utc>>, _>("reset_at"),
        "userId": row.get::<Option<Uuid>, _>("user_id"),
        "teamId": row.get::<Option<Uuid>, _>("team_id"),
        "createdBy": row.get::<Option<Uuid>, _>("created_by"),
        "createdAt": row.get::<Option<DateTime<Utc>>, _>("created_at"),
        "updatedAt": row.get::<Option<DateTime<Utc>>, _>("updated_at"),
        "recorderEnabled": row.get::<bool, _>("recorder_enabled"),
        "replayConfig": json_text(row.get::<Option<String>, _>("replay_config"))
    })
}

fn board_json(row: &PgRow) -> Value {
    json!({
        "id": row.get::<Uuid, _>("board_id"),
        "type": row.get::<String, _>("type"),
        "name": row.get::<String, _>("name"),
        "description": row.get::<String, _>("description"),
        "parameters": json_text(row.get::<Option<String>, _>("parameters")),
        "userId": row.get::<Option<Uuid>, _>("user_id"),
        "teamId": row.get::<Option<Uuid>, _>("team_id"),
        "createdAt": row.get::<Option<DateTime<Utc>>, _>("created_at"),
        "updatedAt": row.get::<Option<DateTime<Utc>>, _>("updated_at")
    })
}

fn annotation_json(row: &PgRow) -> Value {
    json!({
        "id": row.get::<Uuid, _>("annotation_id"),
        "websiteId": row.get::<Uuid, _>("website_id"),
        "userId": row.get::<Option<Uuid>, _>("user_id"),
        "date": row.get::<DateTime<Utc>, _>("date").to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "allDay": row.get::<bool, _>("all_day"),
        "note": row.get::<String, _>("note"),
        "createdAt": row.get::<Option<DateTime<Utc>>, _>("created_at"),
        "updatedAt": row.get::<Option<DateTime<Utc>>, _>("updated_at")
    })
}

fn segment_json(row: &PgRow) -> Value {
    json!({
        "id": row.get::<Uuid, _>("segment_id"),
        "websiteId": row.get::<Uuid, _>("website_id"),
        "type": row.get::<String, _>("type"),
        "name": row.get::<String, _>("name"),
        "parameters": json_text(row.get::<Option<String>, _>("parameters")),
        "createdAt": row.get::<Option<DateTime<Utc>>, _>("created_at"),
        "updatedAt": row.get::<Option<DateTime<Utc>>, _>("updated_at")
    })
}

fn share_json(row: &PgRow) -> Value {
    json!({
        "id": row.get::<Uuid, _>("share_id"),
        "entityId": row.get::<Uuid, _>("entity_id"),
        "name": row.get::<String, _>("name"),
        "shareType": row.get::<i32, _>("share_type"),
        "slug": row.get::<String, _>("slug"),
        "parameters": json_text(row.get::<Option<String>, _>("parameters")),
        "createdAt": row.get::<Option<DateTime<Utc>>, _>("created_at"),
        "updatedAt": row.get::<Option<DateTime<Utc>>, _>("updated_at")
    })
}

const WEBSITE_SELECT: &str = r#"
    SELECT
        w.website_id, w.name, w.domain, w.reset_at, w.user_id, w.team_id,
        w.created_by, w.created_at, w.updated_at, w.recorder_enabled,
        w.replay_config::text AS replay_config, s.slug AS share_id
    FROM website w
    LEFT JOIN LATERAL (
        SELECT slug
        FROM share
        WHERE entity_id = w.website_id AND share_type = 1
        ORDER BY created_at
        LIMIT 1
    ) s ON TRUE
"#;

async fn handle_auth_verify(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    super::auth::verify_auth_response(pool, &headers).await
}

async fn handle_me(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    super::auth::current_auth_response(pool, &headers).await
}

async fn handle_auth_logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    if let Err(response) = authenticate(pool, &headers, "/api/auth/logout").await {
        return response;
    }
    json_response(json!({ "ok": true }))
}

async fn handle_auth_subscription(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (user, role) = match authenticate(pool, &headers, "/api/auth/subscription").await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if let Some(team) = params.get("teamId") {
        let Ok(team) = Uuid::parse_str(team) else {
            return status_response(StatusCode::BAD_REQUEST);
        };
        if role != "admin" {
            match sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM team_user tu JOIN team t USING(team_id) WHERE t.deleted_at IS NULL AND tu.team_id=$1 AND tu.user_id=$2)")
                .bind(team).bind(user).fetch_one(pool).await {
                Ok(true)=>(), Ok(false)=>return status_response(StatusCode::UNAUTHORIZED), Err(e)=>return error_response(StatusCode::INTERNAL_SERVER_ERROR,e.to_string())
            }
        }
    }
    let mut res = json_response(
        json!({"isPro":false,"isBusiness":false,"isNoBilling":false,"hasSubscription":false,"unlimitedWebsites":false}),
    );
    res.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("private, no-store, max-age=0"),
    );
    res
}

async fn handle_change_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (user_id, role) = match authenticate(pool, &headers, "/api/me/password").await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let current_password = body
        .get("currentPassword")
        .and_then(Value::as_str)
        .unwrap_or("");
    let new_password = body
        .get("newPassword")
        .and_then(Value::as_str)
        .unwrap_or("");
    if new_password.len() < 8 {
        return error_response(StatusCode::BAD_REQUEST, "New password is too short");
    }
    let stored = match sqlx::query_scalar::<_, String>(
        r#"SELECT password FROM "user" WHERE user_id = $1 AND deleted_at IS NULL"#,
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(password)) => password,
        Ok(None) => return status_response(StatusCode::NOT_FOUND),
        Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
    };
    if !bcrypt::verify(current_password, &stored).unwrap_or(false) {
        return error_response(StatusCode::BAD_REQUEST, "Current password is incorrect");
    }
    let hash = match bcrypt::hash(new_password, 10) {
        Ok(hash) => hash,
        Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
    };
    let updated = match sqlx::query(
        r#"
        UPDATE "user"
        SET password = $1, updated_at = NOW()
        WHERE user_id = $2 AND deleted_at IS NULL
        RETURNING user_id, username, role, created_at
        "#,
    )
    .bind(&hash)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => return status_response(StatusCode::NOT_FOUND),
        Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
    };
    let claims = json!({
        "userId": user_id,
        "role": role,
        "pwd": super::sha512_hex(&[&hash])
    });
    let token = match super::auth::create_secure_token(&claims, &super::secret_hash()) {
        Some(token) => token,
        None => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
    };
    json_response(json!({
        "id": updated.get::<Uuid, _>("user_id"),
        "username": updated.get::<String, _>("username"),
        "role": updated.get::<String, _>("role"),
        "createdAt": updated.get::<Option<DateTime<Utc>>, _>("created_at"),
        "token": token
    }))
}

struct ListOptions {
    page: i64,
    page_size: i64,
    offset: i64,
    order_by: &'static str,
    direction: &'static str,
}

fn list_options(
    params: &HashMap<String, String>,
    fields: &'static [(&'static str, &'static str)],
    default_order: &'static str,
    default_descending: bool,
) -> Result<ListOptions, Response> {
    let parse_positive = |name: &str, default| match params.get(name) {
        Some(value) => value
            .parse::<i64>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, "Invalid paging")),
        None => Ok(default),
    };
    let page = parse_positive("page", 1)?;
    let page_size = parse_positive("pageSize", 20)?.min(500);
    let descending = match params.get("sortDescending").map(String::as_str) {
        Some("true") => true,
        Some("false") => false,
        Some(_) => return Err(error_response(StatusCode::BAD_REQUEST, "Invalid sorting")),
        None => default_descending,
    };
    let order_by = params
        .get("orderBy")
        .and_then(|requested| {
            fields
                .iter()
                .find_map(|(name, column)| (requested == name).then_some(*column))
        })
        .unwrap_or(default_order);
    Ok(ListOptions {
        page,
        page_size,
        offset: (page - 1).saturating_mul(page_size),
        order_by,
        direction: if descending { "DESC" } else { "ASC" },
    })
}

async fn website_page(
    pool: &PgPool,
    user_id: Uuid,
    params: &HashMap<String, String>,
) -> Result<Value, Response> {
    let options = list_options(
        params,
        &[
            ("name", "w.name"),
            ("domain", "w.domain"),
            ("createdAt", "w.created_at"),
        ],
        "w.name",
        false,
    )?;
    let search = params
        .get("search")
        .map(|value| format!("%{}%", value.trim()))
        .filter(|value| value != "%%");
    let include_teams = include_team_access(params);
    let count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)::bigint
        FROM website w
        WHERE w.deleted_at IS NULL
          AND (
            w.user_id = $1
            OR (
              $2
              AND EXISTS (
                SELECT 1
                FROM team t
                JOIN team_user tu ON tu.team_id = t.team_id
                WHERE t.team_id = w.team_id
                  AND t.deleted_at IS NULL
                  AND tu.user_id = $1
              )
            )
          )
          AND ($3::text IS NULL OR w.name ILIKE $3 OR COALESCE(w.domain, '') ILIKE $3)
        "#,
    )
    .bind(user_id)
    .bind(include_teams)
    .bind(search.as_deref())
    .fetch_one(pool)
    .await
    .map_err(|_| status_response(StatusCode::INTERNAL_SERVER_ERROR))?;
    let query = format!(
        "{WEBSITE_SELECT}
         WHERE w.deleted_at IS NULL
           AND (
             w.user_id = $1
             OR (
               $2
               AND EXISTS (
                 SELECT 1
                 FROM team t
                 JOIN team_user tu ON tu.team_id = t.team_id
                 WHERE t.team_id = w.team_id
                   AND t.deleted_at IS NULL
                   AND tu.user_id = $1
               )
             )
           )
           AND ($3::text IS NULL OR w.name ILIKE $3 OR COALESCE(w.domain, '') ILIKE $3)
         ORDER BY {} {}
         LIMIT $4 OFFSET $5",
        options.order_by, options.direction
    );
    let rows = sqlx::query(&query)
        .bind(user_id)
        .bind(include_teams)
        .bind(search.as_deref())
        .bind(options.page_size)
        .bind(options.offset)
        .fetch_all(pool)
        .await
        .map_err(|_| status_response(StatusCode::INTERNAL_SERVER_ERROR))?;
    let data = rows.iter().map(website_json).collect::<Vec<_>>();
    Ok(json!({
        "data": data,
        "count": count,
        "page": options.page,
        "pageSize": options.page_size
    }))
}

fn include_team_access(params: &HashMap<String, String>) -> bool {
    params
        .get("includeTeams")
        .is_some_and(|value| value.eq_ignore_ascii_case("true"))
}

async fn handle_me_websites(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (user_id, _) = match authenticate(pool, &headers, "/api/me/websites").await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    match website_page(pool, user_id, &params).await {
        Ok(value) => json_response(value),
        Err(response) => response,
    }
}

pub(super) async fn handle_websites(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (user_id, _) = match authenticate(pool, &headers, "/api/websites").await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    match website_page(pool, user_id, &params).await {
        Ok(value) => json_response(value),
        Err(response) => response,
    }
}

async fn handle_user_websites(
    State(state): State<AppState>,
    Path(target_user_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (auth_user_id, role) = match authenticate(pool, &headers, "/api/users").await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    if auth_user_id != target_user_id && role != "admin" {
        return error_response(StatusCode::UNAUTHORIZED, "Unauthorized");
    }
    match website_page(pool, target_user_id, &params).await {
        Ok(value) => json_response(value),
        Err(response) => response,
    }
}

async fn telemetry_script() -> impl axum::response::IntoResponse {
    let enabled = std::env::var("NODE_ENV").as_deref() == Ok("production")
        && ["DISABLE_TELEMETRY", "PRIVATE_MODE"]
            .iter()
            .all(|key| std::env::var(key).unwrap_or_default().is_empty());
    let js = if enabled {
        let package: Value =
            serde_json::from_str(include_str!("../../package.json")).expect("package.json");
        let version = package["version"].as_str().expect("package version");
        format!("(()=>{{const i=document.createElement('img');i.src='https://i.umami.is/a.png?v={version}';i.style='width:0;height:0;position:absolute;pointer-events:none;';document.body.appendChild(i)}})();")
    } else {
        "/* telemetry disabled */".to_string()
    };
    ([(axum::http::header::CONTENT_TYPE, "text/javascript")], js)
}

async fn handle_api_keys(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (user_id, _) = match authenticate(pool, &headers, "/api/me/api-keys").await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let rows = match sqlx::query(
        r#"
        SELECT api_key_id, name, key_prefix, last_used_at, created_at
        FROM api_key
        WHERE user_id = $1
        ORDER BY created_at DESC
        "#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
    };
    json_response(Value::Array(
        rows.iter()
            .map(|row| {
                json!({
                    "id": row.get::<Uuid, _>("api_key_id"),
                    "name": row.get::<String, _>("name"),
                    "keyPrefix": row.get::<String, _>("key_prefix"),
                    "lastUsedAt": row.get::<Option<DateTime<Utc>>, _>("last_used_at"),
                    "createdAt": row.get::<Option<DateTime<Utc>>, _>("created_at")
                })
            })
            .collect(),
    ))
}

async fn handle_delete_api_key(
    State(state): State<AppState>,
    Path(key_id): Path<Uuid>,
    headers: HeaderMap,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (user_id, _) = match authenticate(pool, &headers, "/api/me/api-keys").await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    match sqlx::query("DELETE FROM api_key WHERE api_key_id = $1 AND user_id = $2")
        .bind(key_id)
        .bind(user_id)
        .execute(pool)
        .await
    {
        Ok(result) if result.rows_affected() > 0 => json_response(json!({ "ok": true })),
        Ok(_) => status_response(StatusCode::NOT_FOUND),
        Err(_) => status_response(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn handle_get_dashboard(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (user_id, _) = match authenticate(pool, &headers, "/api/dashboard").await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    match sqlx::query(
        r#"
        SELECT board_id, type, name, description, parameters::text AS parameters,
               user_id, team_id, created_at, updated_at
        FROM board
        WHERE board_id = $1
        "#,
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(row)) if row.get::<Option<Uuid>, _>("user_id") == Some(user_id) => {
            json_response(board_json(&row))
        }
        Ok(Some(_)) => status_response(StatusCode::UNAUTHORIZED),
        Ok(None) => json_response(Value::Null),
        Err(_) => status_response(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn handle_upsert_dashboard(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (user_id, _) = match authenticate(pool, &headers, "/api/dashboard").await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let Some(body) = body.as_object() else {
        return error_response(StatusCode::BAD_REQUEST, "Invalid dashboard");
    };
    let name = match body.get("name") {
        Some(Value::String(value)) if value.len() <= 200 => Some(value.as_str()),
        Some(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid dashboard"),
        None => None,
    };
    let description = match body.get("description") {
        Some(Value::String(value)) if value.len() <= 500 => Some(value.as_str()),
        Some(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid dashboard"),
        None => None,
    };
    let parameters = match body.get("parameters") {
        Some(Value::Object(value)) => Value::Object(value.clone()),
        Some(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid dashboard"),
        None => json!({}),
    };
    match crate::boards::dashboard_reports_valid(pool, &parameters).await {
        Ok(true) => (),
        Ok(false) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "Board contains invalid saved reports.",
            )
        }
        Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
    }
    match sqlx::query(
        r#"
        INSERT INTO board
            (board_id, type, name, description, parameters, user_id, created_at, updated_at)
        VALUES
            ($1, 'dashboard', COALESCE($2, ''), COALESCE($3, ''), $4::jsonb, $1, NOW(), NOW())
        ON CONFLICT (board_id) DO UPDATE SET
            name = COALESCE($2, board.name),
            description = COALESCE($3, board.description),
            parameters = $4::jsonb,
            updated_at = NOW()
        WHERE board.user_id = $1
        RETURNING board_id, type, name, description, parameters::text AS parameters,
                  user_id, team_id, created_at, updated_at
        "#,
    )
    .bind(user_id)
    .bind(name)
    .bind(description)
    .bind(parameters.to_string())
    .fetch_optional(pool)
    .await
    {
        Ok(Some(row)) => json_response(board_json(&row)),
        Ok(None) => status_response(StatusCode::UNAUTHORIZED),
        Err(_) => status_response(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

fn bucket_start(date: DateTime<Utc>) -> DateTime<Utc> {
    let hour = (date.hour() / 12) * 12;
    Utc.with_ymd_and_hms(date.year(), date.month(), date.day(), hour, 0, 0)
        .single()
        .unwrap_or(date)
}

async fn handle_website_charts(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let ids = params
        .get("ids")
        .map(|value| {
            value
                .split(',')
                .filter_map(|item| Uuid::parse_str(item.trim()).ok())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if ids.is_empty() || ids.len() > 20 {
        return error_response(StatusCode::BAD_REQUEST, "Invalid website IDs");
    }
    let end_at = parse_ms_param(&params, "endAt").unwrap_or_else(Utc::now);
    let start_at = parse_ms_param(&params, "startAt").unwrap_or_else(|| end_at - Duration::days(7));
    if end_at < start_at {
        return error_response(StatusCode::BAD_REQUEST, "Invalid date range");
    }
    let share_request = headers.contains_key("x-umami-share-token")
        || headers.contains_key("x-umami-share-context");
    if !share_request {
        let pool = match database(&state) {
            Ok(pool) => pool,
            Err(response) => return response,
        };
        if let Err(response) = authenticate(pool, &headers, "/api/websites/charts").await {
            return response;
        }
    }
    let mut pool = None;
    let mut allowed_ids = Vec::new();
    let mut denied = None;
    for website_id in &ids {
        match crate::access::website_section_pool(
            &state,
            &headers,
            *website_id,
            &[ShareSection::Overview],
            ShareDataVisibility::Aggregate,
        )
        .await
        {
            Ok(scoped_pool) => {
                pool = Some(scoped_pool);
                allowed_ids.push(*website_id);
            }
            Err(response) if response.status() == StatusCode::UNAUTHORIZED => {
                denied = Some(response);
            }
            Err(response) => return response,
        }
    }
    if allowed_ids.is_empty() {
        return if share_request {
            denied.unwrap_or_else(|| error_response(StatusCode::UNAUTHORIZED, "Unauthorized"))
        } else {
            json_response(json!({ "data": {} }))
        };
    }
    let pool = pool.expect("website IDs were validated as non-empty");

    let mut buckets = Vec::new();
    let mut cursor = bucket_start(start_at);
    let end_bucket = bucket_start(end_at);
    while cursor <= end_bucket {
        buckets.push(cursor);
        cursor += Duration::hours(12);
    }

    let mut charts = Map::new();
    for id in &allowed_ids {
        charts.insert(
            id.to_string(),
            json!({
                "values": vec![0_i64; buckets.len()],
                "total": 0
            }),
        );
    }

    let rows = match sqlx::query(
        r#"
        SELECT website_id,
               date_trunc('day', created_at)
                 + floor(extract(hour FROM created_at) / 12) * interval '12 hours' AS bucket,
               COUNT(DISTINCT session_id)::bigint AS sessions
        FROM website_event
        WHERE website_id = ANY($1::uuid[])
          AND event_type NOT IN (2, 5)
          AND created_at BETWEEN $2 AND $3
        GROUP BY website_id, bucket
        ORDER BY website_id, bucket
        "#,
    )
    .bind(&allowed_ids)
    .bind(start_at)
    .bind(end_at)
    .fetch_all(&pool)
    .await
    {
        Ok(rows) => rows,
        Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
    };
    for row in rows {
        let website_id = row.get::<Uuid, _>("website_id").to_string();
        let bucket = row.get::<DateTime<Utc>, _>("bucket");
        let sessions = row.get::<i64, _>("sessions");
        if let Some(index) = buckets.iter().position(|value| *value == bucket) {
            if let Some(values) = charts
                .get_mut(&website_id)
                .and_then(Value::as_object_mut)
                .and_then(|chart| chart.get_mut("values"))
                .and_then(Value::as_array_mut)
            {
                values[index] = json!(sessions);
            }
        }
    }

    let totals = match sqlx::query(
        r#"
        SELECT website_id, COUNT(DISTINCT session_id)::bigint AS sessions
        FROM website_event
        WHERE website_id = ANY($1::uuid[])
          AND event_type NOT IN (2, 5)
          AND created_at BETWEEN $2 AND $3
        GROUP BY website_id
        "#,
    )
    .bind(&allowed_ids)
    .bind(start_at)
    .bind(end_at)
    .fetch_all(&pool)
    .await
    {
        Ok(rows) => rows,
        Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
    };
    for row in totals {
        let website_id = row.get::<Uuid, _>("website_id").to_string();
        if let Some(chart) = charts.get_mut(&website_id).and_then(Value::as_object_mut) {
            chart.insert("total".to_string(), json!(row.get::<i64, _>("sessions")));
        }
    }
    json_response(json!({ "data": charts }))
}

const SEGMENT_OPERATORS: &[&str] = &[
    "eq", "neq", "s", "ns", "c", "dnc", "re", "nre", "t", "f", "gt", "lt", "gte", "lte", "bf", "af",
];

fn segment_parameters(value: &Value) -> Option<Value> {
    let input = value.as_object()?;
    let mut output = Map::new();

    if let Some(value) = input.get("filters") {
        let filters = value.as_array()?;
        let mut normalized = Vec::with_capacity(filters.len());
        for filter in filters {
            let filter = filter.as_object()?;
            let name = filter.get("name")?.as_str()?;
            let operator = filter.get("operator")?.as_str()?;
            let value = filter.get("value")?.as_str()?;
            if !SEGMENT_OPERATORS.contains(&operator) {
                return None;
            }
            normalized.push(json!({ "name": name, "operator": operator, "value": value }));
        }
        output.insert("filters".to_string(), Value::Array(normalized));
    }

    if let Some(value) = input.get("sessionPropertyFilters") {
        let filters = value.as_array()?;
        let mut normalized = Vec::with_capacity(filters.len());
        for filter in filters {
            let filter = filter.as_object()?;
            let property_name = filter.get("propertyName")?.as_str()?;
            let data_type = filter.get("dataType")?.as_i64()?;
            let operator = filter.get("operator")?.as_str()?;
            let value = filter.get("value")?.as_str()?;
            if property_name.is_empty()
                || property_name.chars().count() > 500
                || !(1..=5).contains(&data_type)
                || !SEGMENT_OPERATORS.contains(&operator)
            {
                return None;
            }
            normalized.push(json!({
                "propertyName": property_name,
                "dataType": data_type,
                "operator": operator,
                "value": value
            }));
        }
        output.insert(
            "sessionPropertyFilters".to_string(),
            Value::Array(normalized),
        );
    }

    if let Some(value) = input.get("match") {
        let value = value.as_str()?;
        if !matches!(value, "all" | "any") {
            return None;
        }
        output.insert("match".to_string(), json!(value));
    }

    if let Some(value) = input.get("dateRange") {
        output.insert("dateRange".to_string(), json!(value.as_str()?));
    }

    if let Some(value) = input.get("action") {
        let action = value.as_object()?;
        let action_type = action.get("type")?.as_str()?;
        let action_value = action.get("value")?.as_str()?;
        if !matches!(action_type, "path" | "event") {
            return None;
        }
        output.insert(
            "action".to_string(),
            json!({ "type": action_type, "value": action_value }),
        );
    }

    Some(Value::Object(output))
}

fn segment_input(body: &Value) -> Option<(String, String, String)> {
    let body = body.as_object()?;
    let segment_type = body.get("type")?.as_str()?;
    if !matches!(segment_type, "segment" | "cohort") {
        return None;
    }
    let name = body.get("name")?.as_str()?;
    if name.is_empty() || name.chars().count() > 200 {
        return None;
    }
    let parameters = segment_parameters(body.get("parameters")?)?;
    if segment_type == "cohort" {
        let parameters = parameters.as_object()?;
        if parameters
            .get("sessionPropertyFilters")
            .and_then(Value::as_array)
            .is_some_and(|filters| !filters.is_empty())
            || parameters
                .get("dateRange")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
            || parameters
                .get("action")
                .and_then(Value::as_object)
                .and_then(|action| action.get("value"))
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
        {
            return None;
        }
    }
    Some((
        segment_type.to_string(),
        name.to_string(),
        parameters.to_string(),
    ))
}

async fn fetch_website(pool: &PgPool, website_id: Uuid) -> Result<Option<PgRow>, sqlx::Error> {
    let query = format!("{WEBSITE_SELECT} WHERE w.website_id = $1 AND w.deleted_at IS NULL");
    sqlx::query(&query)
        .bind(website_id)
        .fetch_optional(pool)
        .await
}

fn normalize_recorder_config(value: &Value) -> Map<String, Value> {
    let Some(config) = value.as_object() else {
        return Map::new();
    };
    let mut normalized = Map::new();
    for key in ["replayEnabled", "heatmapEnabled"] {
        if config.get(key).and_then(Value::as_bool) == Some(true) {
            normalized.insert(key.to_string(), json!(true));
        }
    }
    for key in ["sampleRate", "heatmapSampleRate"] {
        if let Some(value) = config.get(key).and_then(Value::as_f64) {
            normalized.insert(key.to_string(), json!(value));
        }
    }
    if matches!(
        config.get("maskLevel").and_then(Value::as_str),
        Some("strict" | "moderate")
    ) {
        normalized.insert(
            "maskLevel".to_string(),
            config.get("maskLevel").cloned().unwrap(),
        );
    }
    if let Some(value) = config.get("maxDuration").and_then(Value::as_f64) {
        normalized.insert("maxDuration".to_string(), json!(value.round() as i64));
    }
    if let Some(value) = config.get("blockSelector").and_then(Value::as_str) {
        normalized.insert("blockSelector".to_string(), json!(value));
    }
    normalized
}

fn update_recorder_config(current: Option<String>, input: &Value) -> Result<(String, bool), ()> {
    let mut merged = if input.is_null() {
        Map::new()
    } else {
        let current = current
            .as_deref()
            .and_then(|value| serde_json::from_str(value).ok())
            .unwrap_or(Value::Null);
        normalize_recorder_config(&current)
    };
    if !input.is_null() {
        let update = input.as_object().ok_or(())?;
        for (key, value) in update {
            let valid = match key.as_str() {
                "replayEnabled" | "heatmapEnabled" => value.is_boolean(),
                "sampleRate" | "heatmapSampleRate" => value
                    .as_f64()
                    .is_some_and(|value| (0.0..=1.0).contains(&value)),
                "maskLevel" => matches!(value.as_str(), Some("strict" | "moderate")),
                "maxDuration" => value
                    .as_f64()
                    .is_some_and(|value| value > 0.0 && value.fract() == 0.0),
                "blockSelector" => value.is_string(),
                _ => continue,
            };
            if !valid {
                return Err(());
            }
            merged.insert(key.clone(), value.clone());
        }
    }
    let normalized = normalize_recorder_config(&Value::Object(merged));
    let enabled = normalized
        .get("replayEnabled")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || normalized
            .get("heatmapEnabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    Ok((Value::Object(normalized).to_string(), enabled))
}

async fn handle_get_website(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    headers: HeaderMap,
) -> Response {
    let pool = match crate::access::website_metadata_pool(&state, &headers, website_id).await {
        Ok(pool) => pool,
        Err(response) => {
            let has_share_credentials = headers.contains_key("x-umami-share-token")
                || headers.contains_key("x-umami-share-context");
            let Some(database) = state.db.as_ref() else {
                return response;
            };
            if !has_share_credentials
                && matches!(
                    authenticate(database, &headers, "/api/websites").await,
                    Ok((_, ref role)) if role == "admin"
                )
                && matches!(fetch_website(database, website_id).await, Ok(None))
            {
                return json_response(Value::Null);
            }
            return response;
        }
    };
    match fetch_website(&pool, website_id).await {
        Ok(Some(row)) => json_response(website_json(&row)),
        Ok(None) => json_response(Value::Null),
        Err(_) => status_response(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn handle_update_website(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let pool = match crate::access::website_pool(&state, &headers, website_id, Access::Write).await
    {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let name = body.get("name").and_then(Value::as_str);
    let domain_present = body.get("domain").is_some();
    let domain = body.get("domain").and_then(Value::as_str);
    let share_present = body.get("shareId").is_some();
    let share_id = body.get("shareId").and_then(Value::as_str);
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
    };
    let recorder = if let Some(input) = body.get("replayConfig") {
        let current = match sqlx::query_scalar::<_, Option<String>>(
            "SELECT replay_config::text FROM website WHERE website_id = $1 AND deleted_at IS NULL",
        )
        .bind(website_id)
        .fetch_optional(&mut *tx)
        .await
        {
            Ok(Some(current)) => current,
            Ok(None) => return status_response(StatusCode::NOT_FOUND),
            Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
        };
        match update_recorder_config(current, input) {
            Ok(recorder) => Some(recorder),
            Err(()) => return error_response(StatusCode::BAD_REQUEST, "Invalid recorder config"),
        }
    } else {
        None
    };
    let updated = sqlx::query(
        r#"
        UPDATE website
        SET name = COALESCE($1, name),
            domain = CASE WHEN $2 THEN $3 ELSE domain END,
            replay_config = CASE WHEN $4::text IS NULL THEN replay_config ELSE $4::jsonb END,
            recorder_enabled = CASE WHEN $4::text IS NULL THEN recorder_enabled ELSE $5 END,
            updated_at = NOW()
        WHERE website_id = $6 AND deleted_at IS NULL
        "#,
    )
    .bind(name)
    .bind(domain_present)
    .bind(domain)
    .bind(recorder.as_ref().map(|value| value.0.as_str()))
    .bind(recorder.as_ref().is_some_and(|value| value.1))
    .bind(website_id)
    .execute(&mut *tx)
    .await;
    match updated {
        Ok(result) if result.rows_affected() > 0 => {}
        Ok(_) => return status_response(StatusCode::NOT_FOUND),
        Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
    }

    if share_present {
        if let Some(slug) = share_id.filter(|value| !value.trim().is_empty()) {
            let result = sqlx::query(
                r#"
                INSERT INTO share
                    (share_id, entity_id, name, share_type, slug, parameters, created_at, updated_at)
                SELECT
                    $1, website_id, name, 1, $3,
                    '{"overview":true,"events":true}'::jsonb, NOW(), NOW()
                FROM website
                WHERE website_id = $2
                ON CONFLICT (slug) DO UPDATE SET
                    name = EXCLUDED.name,
                    share_type = 1,
                    updated_at = NOW()
                WHERE share.entity_id = EXCLUDED.entity_id
                "#,
            )
            .bind(Uuid::new_v4())
            .bind(website_id)
            .bind(slug)
            .execute(&mut *tx)
            .await;
            if !matches!(result, Ok(result) if result.rows_affected() > 0) {
                return error_response(StatusCode::BAD_REQUEST, "That share ID is already taken");
            }
            if sqlx::query(
                "DELETE FROM share WHERE entity_id = $1 AND share_type = 1 AND slug <> $2",
            )
            .bind(website_id)
            .bind(slug)
            .execute(&mut *tx)
            .await
            .is_err()
            {
                return status_response(StatusCode::INTERNAL_SERVER_ERROR);
            }
        } else if sqlx::query("DELETE FROM share WHERE entity_id = $1 AND share_type = 1")
            .bind(website_id)
            .execute(&mut *tx)
            .await
            .is_err()
        {
            return status_response(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }
    if tx.commit().await.is_err() {
        return status_response(StatusCode::INTERNAL_SERVER_ERROR);
    }
    match fetch_website(&pool, website_id).await {
        Ok(Some(row)) => json_response(website_json(&row)),
        Ok(None) => status_response(StatusCode::NOT_FOUND),
        Err(_) => status_response(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

const WEBSITE_DATA_DELETE_STATEMENTS: &[&str] = &[
    "DELETE FROM session_replay_saved WHERE website_id = $1",
    "DELETE FROM session_replay WHERE website_id = $1",
    "DELETE FROM heatmap_event WHERE website_id = $1",
    "DELETE FROM revenue WHERE website_id = $1",
    "DELETE FROM error_event WHERE website_id = $1",
    "DELETE FROM error_issue WHERE website_id = $1",
    "DELETE FROM error_rate_limit WHERE website_id = $1",
    "DELETE FROM event_data WHERE website_id = $1",
    "DELETE FROM event_data USING website_event WHERE event_data.website_event_id = website_event.event_id AND website_event.website_id = $1",
    "DELETE FROM session_data WHERE website_id = $1",
    "DELETE FROM session_link WHERE website_id = $1",
    "DELETE FROM website_event WHERE website_id = $1",
    "DELETE FROM session WHERE website_id = $1",
];

const WEBSITE_DELETE_STATEMENTS: &[&str] = &[
    "DELETE FROM report WHERE website_id = $1",
    "DELETE FROM segment WHERE website_id = $1",
    "DELETE FROM annotation WHERE website_id = $1",
    "DELETE FROM share WHERE entity_id = $1",
];

async fn delete_website_data(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    website_id: Uuid,
) -> Result<(), sqlx::Error> {
    for statement in WEBSITE_DATA_DELETE_STATEMENTS {
        sqlx::query(statement)
            .bind(website_id)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

async fn handle_delete_website(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    headers: HeaderMap,
) -> Response {
    let pool = match crate::access::website_pool(&state, &headers, website_id, Access::Delete).await
    {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
    };
    let result = async {
        sqlx::query("SET LOCAL statement_timeout = '30s'")
            .execute(&mut *tx)
            .await?;
        delete_website_data(&mut tx, website_id).await?;
        for statement in WEBSITE_DELETE_STATEMENTS {
            sqlx::query(statement)
                .bind(website_id)
                .execute(&mut *tx)
                .await?;
        }
        let deleted = if std::env::var_os("CLOUD_MODE").is_some() {
            sqlx::query(
                "UPDATE website SET deleted_at = NOW(), updated_at = NOW() WHERE website_id = $1 AND deleted_at IS NULL",
            )
            .bind(website_id)
            .execute(&mut *tx)
            .await?
        } else {
            sqlx::query("DELETE FROM website WHERE website_id = $1 AND deleted_at IS NULL")
                .bind(website_id)
                .execute(&mut *tx)
                .await?
        };
        if deleted.rows_affected() == 0 {
            return Err(sqlx::Error::RowNotFound);
        }
        tx.commit().await
    }
    .await;
    if result.is_err() {
        return status_response(StatusCode::INTERNAL_SERVER_ERROR);
    }
    json_response(json!({ "ok": true }))
}

async fn handle_reset_website(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    headers: HeaderMap,
) -> Response {
    let pool = match crate::access::website_pool(&state, &headers, website_id, Access::Write).await
    {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
    };
    let result = async {
        sqlx::query("SET LOCAL statement_timeout = '30s'")
            .execute(&mut *tx)
            .await?;
        delete_website_data(&mut tx, website_id).await?;
        let updated = sqlx::query(
            "UPDATE website SET reset_at = NOW(), updated_at = NOW() WHERE website_id = $1 AND deleted_at IS NULL",
        )
        .bind(website_id)
        .execute(&mut *tx)
        .await?;
        if updated.rows_affected() == 0 {
            return Err(sqlx::Error::RowNotFound);
        }
        tx.commit().await
    }
    .await;
    if result.is_err() {
        return status_response(StatusCode::INTERNAL_SERVER_ERROR);
    }
    json_response(json!({ "ok": true }))
}

async fn handle_website_shares(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match crate::access::website_pool(&state, &headers, website_id, Access::Read).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (page, page_size, offset) = paging(&params);
    let search = params
        .get("search")
        .map(|value| format!("%{}%", value.trim()))
        .filter(|value| value != "%%");
    let count = match sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)::bigint
        FROM share
        WHERE entity_id = $1
          AND ($2::text IS NULL OR name ILIKE $2 OR slug ILIKE $2)
        "#,
    )
    .bind(website_id)
    .bind(search.as_deref())
    .fetch_one(&pool)
    .await
    {
        Ok(count) => count,
        Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
    };
    let rows = match sqlx::query(
        r#"
        SELECT share_id, entity_id, name, share_type, slug,
               parameters::text AS parameters, created_at, updated_at
        FROM share
        WHERE entity_id = $1
          AND ($2::text IS NULL OR name ILIKE $2 OR slug ILIKE $2)
        ORDER BY created_at DESC
        LIMIT $3 OFFSET $4
        "#,
    )
    .bind(website_id)
    .bind(search.as_deref())
    .bind(page_size)
    .bind(offset)
    .fetch_all(&pool)
    .await
    {
        Ok(rows) => rows,
        Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
    };
    let data = rows.iter().map(share_json).collect::<Vec<_>>();
    json_response(json!({
        "data": data,
        "count": count,
        "page": page,
        "pageSize": page_size
    }))
}

async fn handle_create_website_share(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let pool = match crate::access::website_pool(&state, &headers, website_id, Access::Write).await
    {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let name = match body.get("name").and_then(Value::as_str) {
        Some(name) if name.len() <= 200 => name,
        _ => return error_response(StatusCode::BAD_REQUEST, "Invalid share"),
    };
    let parameters = match body.get("parameters") {
        Some(Value::Object(parameters)) => Value::Object(parameters.clone()),
        Some(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid share"),
        None => json!({}),
    };
    let slug: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(16)
        .map(char::from)
        .collect();
    match sqlx::query(
        r#"
        INSERT INTO share
            (share_id, entity_id, name, share_type, slug, parameters, created_at, updated_at)
        VALUES ($1, $2, $3, 1, $4, $5::jsonb, NOW(), NOW())
        RETURNING share_id, entity_id, name, share_type, slug,
                  parameters::text AS parameters, created_at, updated_at
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(website_id)
    .bind(name)
    .bind(slug)
    .bind(parameters.to_string())
    .fetch_one(&pool)
    .await
    {
        Ok(row) => json_response(share_json(&row)),
        Err(_) => status_response(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

fn parse_body_datetime(value: Option<&Value>) -> Option<DateTime<Utc>> {
    match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .and_then(|milliseconds| Utc.timestamp_millis_opt(milliseconds).single()),
        Some(Value::String(text)) => DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|date| date.with_timezone(&Utc)),
        _ => None,
    }
}

async fn handle_annotations(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match crate::access::website_filter_pool(&state, &headers, website_id).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (page, page_size, offset) = paging(&params);
    let search = params
        .get("search")
        .map(|value| format!("%{}%", value.trim()))
        .filter(|value| value != "%%");
    let start_at = parse_ms_param(&params, "startAt");
    let end_at = parse_ms_param(&params, "endAt");
    let count = match sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)::bigint
        FROM annotation
        WHERE website_id = $1
          AND ($2::text IS NULL OR note ILIKE $2)
          AND ($3::timestamptz IS NULL OR date >= $3)
          AND ($4::timestamptz IS NULL OR date <= $4)
        "#,
    )
    .bind(website_id)
    .bind(search.as_deref())
    .bind(start_at)
    .bind(end_at)
    .fetch_one(&pool)
    .await
    {
        Ok(count) => count,
        Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
    };
    let rows = match sqlx::query(
        r#"
        SELECT annotation_id, website_id, user_id, date, all_day, note, created_at, updated_at
        FROM annotation
        WHERE website_id = $1
          AND ($2::text IS NULL OR note ILIKE $2)
          AND ($3::timestamptz IS NULL OR date >= $3)
          AND ($4::timestamptz IS NULL OR date <= $4)
        ORDER BY date DESC
        LIMIT $5 OFFSET $6
        "#,
    )
    .bind(website_id)
    .bind(search.as_deref())
    .bind(start_at)
    .bind(end_at)
    .bind(page_size)
    .bind(offset)
    .fetch_all(&pool)
    .await
    {
        Ok(rows) => rows,
        Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
    };
    let data = rows.iter().map(annotation_json).collect::<Vec<_>>();
    json_response(json!({
        "data": data,
        "count": count,
        "page": page,
        "pageSize": page_size
    }))
}

async fn handle_create_annotation(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let pool = match crate::access::website_pool(&state, &headers, website_id, Access::Write).await
    {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (user_id, _) = match authenticate(&pool, &headers, "/api/websites").await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let date = match parse_body_datetime(body.get("date")) {
        Some(date) => date,
        None => return error_response(StatusCode::BAD_REQUEST, "Invalid date"),
    };
    let all_day = body.get("allDay").and_then(Value::as_bool).unwrap_or(true);
    let note = body
        .get("note")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if note.is_empty() || note.len() > 500 {
        return error_response(StatusCode::BAD_REQUEST, "Invalid note");
    }
    match sqlx::query(
        r#"
        INSERT INTO annotation
            (annotation_id, website_id, user_id, date, all_day, note, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, $6, NOW(), NOW())
        RETURNING annotation_id, website_id, user_id, date, all_day, note, created_at, updated_at
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(website_id)
    .bind(user_id)
    .bind(date)
    .bind(all_day)
    .bind(note)
    .fetch_one(&pool)
    .await
    {
        Ok(row) => json_response(annotation_json(&row)),
        Err(_) => status_response(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn get_annotation(
    State(state): State<AppState>,
    Path((website, id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Response {
    let pool = match crate::access::website_filter_pool(&state, &headers, website).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    annotation_response(
        sqlx::query("SELECT * FROM annotation WHERE website_id=$1 AND annotation_id=$2")
            .bind(website)
            .bind(id)
            .fetch_optional(&pool)
            .await,
    )
}

fn annotation_response(row: Result<Option<PgRow>, sqlx::Error>) -> Response {
    match row {
        Ok(Some(row)) => json_response(annotation_json(&row)),
        Ok(None) => status_response(StatusCode::NOT_FOUND),
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn update_annotation(
    State(state): State<AppState>,
    Path((website, id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let pool = match crate::access::website_pool(&state, &headers, website, Access::Write).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some(date) = parse_body_datetime(body.get("date")) else {
        return error_response(StatusCode::BAD_REQUEST, "Invalid date");
    };
    let Some(note) = body["note"]
        .as_str()
        .filter(|s| !s.is_empty() && s.chars().count() <= 500)
    else {
        return error_response(StatusCode::BAD_REQUEST, "Invalid note");
    };
    let all_day = match body.get("allDay") {
        None => true,
        Some(Value::Bool(v)) => *v,
        _ => return error_response(StatusCode::BAD_REQUEST, "Invalid allDay"),
    };
    annotation_response(sqlx::query("UPDATE annotation SET date=$3,note=$4,all_day=$5,updated_at=now() WHERE website_id=$1 AND annotation_id=$2 RETURNING *").bind(website).bind(id).bind(date).bind(note).bind(all_day).fetch_optional(&pool).await)
}

async fn handle_delete_annotation(
    State(state): State<AppState>,
    Path((website_id, annotation_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Response {
    let pool = match crate::access::website_pool(&state, &headers, website_id, Access::Write).await
    {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    match sqlx::query("DELETE FROM annotation WHERE annotation_id = $1 AND website_id = $2")
        .bind(annotation_id)
        .bind(website_id)
        .execute(&pool)
        .await
    {
        Ok(result) if result.rows_affected() > 0 => json_response(json!({ "ok": true })),
        Ok(_) => status_response(StatusCode::NOT_FOUND),
        Err(_) => status_response(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn handle_segments(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let segment_type = match params.get("type").map(String::as_str) {
        Some(segment_type @ ("segment" | "cohort")) => segment_type,
        _ => return error_response(StatusCode::BAD_REQUEST, "Invalid segment type"),
    };
    let pool = match crate::access::website_filter_pool(&state, &headers, website_id).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (page, page_size, offset) = paging(&params);
    let search = params
        .get("search")
        .map(|value| format!("%{}%", value.trim()))
        .filter(|value| value != "%%");
    let count = match sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)::bigint
        FROM segment
        WHERE website_id = $1
          AND type = $2
          AND ($3::text IS NULL OR name ILIKE $3)
        "#,
    )
    .bind(website_id)
    .bind(segment_type)
    .bind(search.as_deref())
    .fetch_one(&pool)
    .await
    {
        Ok(count) => count,
        Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
    };
    let rows = match sqlx::query(
        r#"
        SELECT segment_id, website_id, type, name, parameters::text AS parameters,
               created_at, updated_at
        FROM segment
        WHERE website_id = $1
          AND type = $2
          AND ($3::text IS NULL OR name ILIKE $3)
        ORDER BY name
        LIMIT $4 OFFSET $5
        "#,
    )
    .bind(website_id)
    .bind(segment_type)
    .bind(search.as_deref())
    .bind(page_size)
    .bind(offset)
    .fetch_all(&pool)
    .await
    {
        Ok(rows) => rows,
        Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
    };
    let data = rows.iter().map(segment_json).collect::<Vec<_>>();
    json_response(json!({
        "data": data,
        "count": count,
        "page": page,
        "pageSize": page_size
    }))
}

async fn handle_create_segment(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let (segment_type, name, parameters) = match segment_input(&body) {
        Some(input) => input,
        None => return error_response(StatusCode::BAD_REQUEST, "Invalid segment"),
    };
    let pool = match crate::access::website_pool(&state, &headers, website_id, Access::Write).await
    {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    match sqlx::query(
        r#"
        INSERT INTO segment
            (segment_id, website_id, type, name, parameters, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5::jsonb, NOW(), NOW())
        RETURNING segment_id, website_id, type, name, parameters::text AS parameters,
                  created_at, updated_at
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(website_id)
    .bind(&segment_type)
    .bind(&name)
    .bind(parameters)
    .fetch_one(&pool)
    .await
    {
        Ok(row) => json_response(segment_json(&row)),
        Err(_) => status_response(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn handle_get_segment(
    State(state): State<AppState>,
    Path((website_id, segment_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Response {
    let pool = match crate::access::website_filter_pool(&state, &headers, website_id).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    match sqlx::query(
        r#"
        SELECT segment_id, website_id, type, name, parameters::text AS parameters,
               created_at, updated_at
        FROM segment
        WHERE segment_id = $1 AND website_id = $2
        "#,
    )
    .bind(segment_id)
    .bind(website_id)
    .fetch_optional(&pool)
    .await
    {
        Ok(Some(row)) => json_response(segment_json(&row)),
        Ok(None) => status_response(StatusCode::NOT_FOUND),
        Err(_) => status_response(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn handle_update_segment(
    State(state): State<AppState>,
    Path((website_id, segment_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let (segment_type, name, parameters) = match segment_input(&body) {
        Some(input) => input,
        None => return error_response(StatusCode::BAD_REQUEST, "Invalid segment"),
    };
    let pool = match crate::access::website_pool(&state, &headers, website_id, Access::Write).await
    {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    match sqlx::query(
        r#"
        UPDATE segment
        SET type = $1,
            name = $2,
            parameters = $3::jsonb,
            updated_at = NOW()
        WHERE segment_id = $4 AND website_id = $5
        RETURNING segment_id, website_id, type, name, parameters::text AS parameters,
                  created_at, updated_at
        "#,
    )
    .bind(&segment_type)
    .bind(&name)
    .bind(parameters)
    .bind(segment_id)
    .bind(website_id)
    .fetch_optional(&pool)
    .await
    {
        Ok(Some(row)) => json_response(segment_json(&row)),
        Ok(None) => status_response(StatusCode::NOT_FOUND),
        Err(_) => status_response(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn handle_delete_segment(
    State(state): State<AppState>,
    Path((website_id, segment_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Response {
    let pool = match crate::access::website_pool(&state, &headers, website_id, Access::Delete).await
    {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    match sqlx::query("DELETE FROM segment WHERE segment_id = $1 AND website_id = $2")
        .bind(segment_id)
        .bind(website_id)
        .execute(&pool)
        .await
    {
        Ok(result) if result.rows_affected() > 0 => json_response(json!({ "ok": true })),
        Ok(_) => status_response(StatusCode::NOT_FOUND),
        Err(_) => status_response(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn handle_admin_users(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    if let Err(response) = authenticate_admin(pool, &headers, "/api/admin/users").await {
        return response;
    }
    let options = match list_options(
        &params,
        &[
            ("username", "u.username"),
            ("role", "u.role"),
            ("createdAt", "u.created_at"),
        ],
        "u.created_at",
        true,
    ) {
        Ok(options) => options,
        Err(response) => return response,
    };
    let search = params
        .get("search")
        .map(|value| format!("%{}%", value.trim()))
        .filter(|value| value != "%%");
    let count = match sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)::bigint
        FROM "user"
        WHERE deleted_at IS NULL
          AND ($1::text IS NULL OR username ILIKE $1)
        "#,
    )
    .bind(search.as_deref())
    .fetch_one(pool)
    .await
    {
        Ok(count) => count,
        Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
    };
    let query = format!(
        r#"
        SELECT u.user_id, u.username, u.role, u.created_at, u.two_factor_required,
               COUNT(w.website_id)::bigint AS website_count
        FROM "user" u
        LEFT JOIN website w ON w.user_id = u.user_id AND w.deleted_at IS NULL
        WHERE u.deleted_at IS NULL
          AND ($1::text IS NULL OR u.username ILIKE $1)
        GROUP BY u.user_id
        ORDER BY {} {}
        LIMIT $2 OFFSET $3
        "#,
        options.order_by, options.direction
    );
    let rows = match sqlx::query(&query)
        .bind(search.as_deref())
        .bind(options.page_size)
        .bind(options.offset)
        .fetch_all(pool)
        .await
    {
        Ok(rows) => rows,
        Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
    };
    let data = rows
        .iter()
        .map(|row| {
            let mut value = user_json(row);
            if let Some(object) = value.as_object_mut() {
                object.insert(
                    "_count".to_string(),
                    json!({ "websites": row.get::<i64, _>("website_count") }),
                );
            }
            value
        })
        .collect::<Vec<_>>();
    json_response(json!({
        "data": data,
        "count": count,
        "page": options.page,
        "pageSize": options.page_size
    }))
}

async fn handle_admin_websites(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    if let Err(response) = authenticate_admin(pool, &headers, "/api/admin/websites").await {
        return response;
    }
    let options = match list_options(
        &params,
        &[
            ("name", "w.name"),
            ("domain", "w.domain"),
            ("createdAt", "w.created_at"),
        ],
        "w.created_at",
        true,
    ) {
        Ok(options) => options,
        Err(response) => return response,
    };
    let search = params
        .get("search")
        .map(|value| format!("%{}%", value.trim()))
        .filter(|value| value != "%%");
    let count = match sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)::bigint
        FROM website
        WHERE deleted_at IS NULL
          AND ($1::text IS NULL OR name ILIKE $1 OR COALESCE(domain, '') ILIKE $1)
        "#,
    )
    .bind(search.as_deref())
    .fetch_one(pool)
    .await
    {
        Ok(count) => count,
        Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
    };
    let query = format!(
        r#"
        SELECT
            w.website_id, w.name, w.domain, w.reset_at, w.user_id, w.team_id,
            w.created_by, w.created_at, w.updated_at, w.recorder_enabled,
            w.replay_config::text AS replay_config, s.slug AS share_id,
            u.username AS user_username,
            owner.user_id AS team_owner_user_id,
            owner.role AS team_owner_role
        FROM website w
        LEFT JOIN LATERAL (
            SELECT slug
            FROM share
            WHERE entity_id = w.website_id AND share_type = 1
            ORDER BY created_at
            LIMIT 1
        ) s ON TRUE
        LEFT JOIN "user" u ON u.user_id = w.user_id AND u.deleted_at IS NULL
        LEFT JOIN LATERAL (
            SELECT user_id, role
            FROM team_user
            WHERE team_id = w.team_id AND role = 'team-owner'
            ORDER BY created_at
            LIMIT 1
        ) owner ON TRUE
        WHERE w.deleted_at IS NULL
          AND ($1::text IS NULL OR w.name ILIKE $1 OR COALESCE(w.domain, '') ILIKE $1)
        ORDER BY {} {}
        LIMIT $2 OFFSET $3
        "#,
        options.order_by, options.direction
    );
    let rows = match sqlx::query(&query)
        .bind(search.as_deref())
        .bind(options.page_size)
        .bind(options.offset)
        .fetch_all(pool)
        .await
    {
        Ok(rows) => rows,
        Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
    };
    let data = rows
        .iter()
        .map(|row| {
            let mut value = website_json(row);
            if let Some(object) = value.as_object_mut() {
                let user_id = row.get::<Option<Uuid>, _>("user_id");
                let username = row.get::<Option<String>, _>("user_username");
                object.insert(
                    "user".to_string(),
                    match (user_id, username) {
                        (Some(id), Some(username)) => json!({ "id": id, "username": username }),
                        _ => Value::Null,
                    },
                );
                let team_id = row.get::<Option<Uuid>, _>("team_id");
                let owner_id = row.get::<Option<Uuid>, _>("team_owner_user_id");
                let owner_role = row.get::<Option<String>, _>("team_owner_role");
                object.insert(
                    "team".to_string(),
                    match (team_id, owner_id, owner_role) {
                        (Some(_), Some(user_id), Some(role)) => {
                            json!({ "members": [{ "userId": user_id, "role": role }] })
                        }
                        (Some(_), _, _) => json!({ "members": [] }),
                        _ => Value::Null,
                    },
                );
            }
            value
        })
        .collect::<Vec<_>>();
    json_response(json!({
        "data": data,
        "count": count,
        "page": options.page,
        "pageSize": options.page_size
    }))
}

async fn handle_admin_teams(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    if let Err(response) = authenticate_admin(pool, &headers, "/api/admin/teams").await {
        return response;
    }
    let options = match list_options(
        &params,
        &[("name", "t.name"), ("createdAt", "t.created_at")],
        "t.created_at",
        true,
    ) {
        Ok(options) => options,
        Err(response) => return response,
    };
    let search = params
        .get("search")
        .map(|value| format!("%{}%", value.trim()))
        .filter(|value| value != "%%");
    let count = match sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)::bigint
        FROM team
        WHERE deleted_at IS NULL
          AND ($1::text IS NULL OR name ILIKE $1)
        "#,
    )
    .bind(search.as_deref())
    .fetch_one(pool)
    .await
    {
        Ok(count) => count,
        Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
    };
    let query = format!(
        r#"
        SELECT t.team_id, t.name, t.access_code, t.logo_url, t.two_factor_required,
               t.created_at, t.updated_at,
               COUNT(DISTINCT w.website_id)::bigint AS website_count,
               COUNT(DISTINCT tu.team_user_id)::bigint AS member_count
        FROM team t
        LEFT JOIN website w ON w.team_id = t.team_id AND w.deleted_at IS NULL
        LEFT JOIN team_user tu ON tu.team_id = t.team_id
        LEFT JOIN "user" u ON u.user_id = tu.user_id AND u.deleted_at IS NULL
        WHERE t.deleted_at IS NULL
          AND ($1::text IS NULL OR t.name ILIKE $1)
        GROUP BY t.team_id
        ORDER BY {} {}
        LIMIT $2 OFFSET $3
        "#,
        options.order_by, options.direction
    );
    let rows = match sqlx::query(&query)
        .bind(search.as_deref())
        .bind(options.page_size)
        .bind(options.offset)
        .fetch_all(pool)
        .await
    {
        Ok(rows) => rows,
        Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
    };
    let mut data = Vec::with_capacity(rows.len());
    for row in rows {
        let team_id = row.get::<Uuid, _>("team_id");
        let member_rows = match sqlx::query(
            r#"
            SELECT tu.team_user_id, tu.user_id, tu.role, tu.created_at, tu.updated_at,
                   u.username
            FROM team_user tu
            JOIN "user" u ON u.user_id = tu.user_id AND u.deleted_at IS NULL
            WHERE tu.team_id = $1
            ORDER BY tu.created_at
            "#,
        )
        .bind(team_id)
        .fetch_all(pool)
        .await
        {
            Ok(values) => values,
            Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
        };
        let members = member_rows
            .iter()
            .map(|member| {
                json!({
                    "id": member.get::<Uuid, _>("team_user_id"),
                    "teamId": team_id,
                    "userId": member.get::<Uuid, _>("user_id"),
                    "role": member.get::<String, _>("role"),
                    "createdAt": member.get::<Option<DateTime<Utc>>, _>("created_at"),
                    "updatedAt": member.get::<Option<DateTime<Utc>>, _>("updated_at"),
                    "user": {
                        "id": member.get::<Uuid, _>("user_id"),
                        "username": member.get::<String, _>("username")
                    }
                })
            })
            .collect::<Vec<_>>();
        data.push(json!({
            "id": team_id,
            "name": row.get::<String, _>("name"),
            "accessCode": row.get::<Option<String>, _>("access_code"),
            "logoUrl": row.get::<Option<String>, _>("logo_url"),
            "twoFactorRequired": row.get::<bool, _>("two_factor_required"),
            "createdAt": row.get::<Option<DateTime<Utc>>, _>("created_at"),
            "updatedAt": row.get::<Option<DateTime<Utc>>, _>("updated_at"),
            "members": members,
            "_count": {
                "websites": row.get::<i64, _>("website_count"),
                "members": row.get::<i64, _>("member_count")
            }
        }));
    }
    json_response(json!({
        "data": data,
        "count": count,
        "page": options.page,
        "pageSize": options.page_size
    }))
}

async fn handle_create_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    if let Err(response) = authenticate_admin(pool, &headers, "/api/users").await {
        return response;
    }
    let id = match body.get("id") {
        Some(Value::String(value)) => match Uuid::parse_str(value) {
            Ok(id) => id,
            Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid user ID"),
        },
        Some(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid user ID"),
        None => Uuid::new_v4(),
    };
    let username = body
        .get("username")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_lowercase();
    let password = body.get("password").and_then(Value::as_str).unwrap_or("");
    let role = body.get("role").and_then(Value::as_str).unwrap_or("");
    if username.len() > 255
        || !(8..=255).contains(&password.len())
        || !matches!(role, "admin" | "user" | "view-only")
    {
        return error_response(StatusCode::BAD_REQUEST, "Invalid user");
    }
    let exists = match sqlx::query_scalar::<_, bool>(
        r#"SELECT EXISTS(SELECT 1 FROM "user" WHERE username = $1)"#,
    )
    .bind(&username)
    .fetch_one(pool)
    .await
    {
        Ok(exists) => exists,
        Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
    };
    if exists {
        return error_response(StatusCode::BAD_REQUEST, "User already exists");
    }
    let password = match bcrypt::hash(password, 10) {
        Ok(password) => password,
        Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
    };
    match sqlx::query(
        r#"
        INSERT INTO "user"
            (user_id, username, password, role, created_at, updated_at)
        VALUES ($1, $2, $3, $4, NOW(), NOW())
        RETURNING user_id, username, role
        "#,
    )
    .bind(id)
    .bind(username)
    .bind(password)
    .bind(role)
    .fetch_one(pool)
    .await
    {
        Ok(row) => json_response(json!({
            "id": row.get::<Uuid, _>("user_id"),
            "username": row.get::<String, _>("username"),
            "role": row.get::<String, _>("role")
        })),
        Err(error) if error.to_string().to_lowercase().contains("unique") => {
            error_response(StatusCode::BAD_REQUEST, "User already exists")
        }
        Err(_) => status_response(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn handle_get_user(
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    headers: HeaderMap,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (viewer_id, role) = match authenticate(pool, &headers, "/api/users").await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    if role != "admin" && viewer_id != user_id {
        return error_response(StatusCode::UNAUTHORIZED, "Unauthorized");
    }
    match sqlx::query(
        r#"
        SELECT user_id, username, role, created_at, two_factor_required
        FROM "user"
        WHERE user_id = $1 AND deleted_at IS NULL
        "#,
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(row)) => json_response(user_json(&row)),
        Ok(None) => status_response(StatusCode::NOT_FOUND),
        Err(_) => status_response(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn handle_update_user(
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (viewer_id, role) = match authenticate(pool, &headers, "/api/users").await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    if role != "admin" && viewer_id != user_id {
        return error_response(StatusCode::UNAUTHORIZED, "Unauthorized");
    }
    let username = if role == "admin" {
        body.get("username")
            .and_then(Value::as_str)
            .map(|value| value.trim().to_lowercase())
    } else {
        None
    };
    let new_role = if role == "admin" {
        body.get("role").and_then(Value::as_str)
    } else {
        None
    };
    let password = if role == "admin" {
        match body.get("password").and_then(Value::as_str) {
            Some(value) if value.len() >= 8 => match bcrypt::hash(value, 10) {
                Ok(hash) => Some(hash),
                Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
            },
            Some(_) => return error_response(StatusCode::BAD_REQUEST, "Password is too short"),
            None => None,
        }
    } else {
        None
    };
    match sqlx::query(
        r#"
        UPDATE "user"
        SET username = COALESCE($1, username),
            role = COALESCE($2, role),
            password = COALESCE($3, password),
            updated_at = NOW()
        WHERE user_id = $4 AND deleted_at IS NULL
        RETURNING user_id, username, role, created_at, two_factor_required
        "#,
    )
    .bind(username)
    .bind(new_role)
    .bind(password)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(row)) => json_response(user_json(&row)),
        Ok(None) => status_response(StatusCode::NOT_FOUND),
        Err(error) if error.to_string().to_lowercase().contains("unique") => {
            error_response(StatusCode::BAD_REQUEST, "User already exists")
        }
        Err(_) => status_response(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn handle_delete_user(
    State(state): State<AppState>,
    Path(user): Path<Uuid>,
    headers: HeaderMap,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    match authenticate_admin(pool, &headers, "/api/users").await {
        Err(response) => return response,
        Ok(me) if me == user => {
            return error_response(StatusCode::BAD_REQUEST, "You cannot delete yourself.")
        }
        _ => (),
    }
    let result: Result<(), sqlx::Error> = async {
        let mut tx = pool.begin().await?;
        let owned_teams: Vec<Uuid> = sqlx::query_scalar("SELECT team_id FROM team_user WHERE role='team-owner' AND user_id=$1")
            .bind(user).fetch_all(&mut *tx).await?;
        let websites: Vec<Uuid> = sqlx::query_scalar("SELECT website_id FROM website WHERE user_id=$1 FOR UPDATE")
            .bind(user).fetch_all(&mut *tx).await?;
        for id in websites {
            delete_website_data(&mut tx,id).await?;
            for stmt in WEBSITE_DELETE_STATEMENTS {
                sqlx::query(stmt).bind(id).execute(&mut *tx).await?;
            }
        }
        for (table, key) in [("link","link_id"),("pixel","pixel_id"),("board","board_id")] {
            let sql = format!("DELETE FROM share WHERE entity_id IN (SELECT {key} FROM {table} WHERE user_id=$1 OR team_id=ANY($2))");
            sqlx::query(&sql).bind(user).bind(&owned_teams).execute(&mut *tx).await?;
            sqlx::query(&format!("DELETE FROM {table} WHERE user_id=$1 OR team_id=ANY($2)"))
                .bind(user).bind(&owned_teams).execute(&mut *tx).await?;
        }
        sqlx::query("DELETE FROM team_user WHERE user_id=$1 OR team_id=ANY($2)").bind(user).bind(&owned_teams).execute(&mut *tx).await?;
        // Node removes owned teams and clears the team relation on surviving websites.
        sqlx::query("UPDATE website SET team_id=NULL WHERE team_id=ANY($1)").bind(&owned_teams).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM team WHERE team_id=ANY($1)").bind(&owned_teams).execute(&mut *tx).await?;
        for table in ["api_key","two_factor_auth","two_factor_backup_code","two_factor_otp_used","two_factor_rate_limit","report","website"] {
            sqlx::query(&format!("DELETE FROM {table} WHERE user_id=$1")).bind(user).execute(&mut *tx).await?;
        }
        sqlx::query("UPDATE website SET created_by=NULL WHERE created_by=$1").bind(user).execute(&mut *tx).await?;
        if sqlx::query("DELETE FROM \"user\" WHERE user_id=$1").bind(user).execute(&mut *tx).await?.rows_affected()==0 {
            return Err(sqlx::Error::RowNotFound);
        }
        tx.commit().await
    }.await;
    match result {
        Ok(()) => json_response(json!({"ok":true})),
        Err(sqlx::Error::RowNotFound) => status_response(StatusCode::NOT_FOUND),
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_queries_cover_node_dependents() {
        for table in [
            "session_replay_saved",
            "session_replay",
            "heatmap_event",
            "revenue",
            "error_event",
            "error_issue",
            "error_rate_limit",
            "event_data",
            "session_data",
            "session_link",
            "website_event",
            "session",
        ] {
            assert!(
                WEBSITE_DATA_DELETE_STATEMENTS
                    .iter()
                    .any(|statement| statement.contains(&format!("DELETE FROM {table}"))),
                "missing lifecycle cleanup for {table}"
            );
        }
        assert!(WEBSITE_DATA_DELETE_STATEMENTS.iter().any(|statement| {
            statement.contains("event_data.website_event_id = website_event.event_id")
        }));
    }

    #[test]
    fn delete_only_queries_preserve_reset_configuration() {
        assert_eq!(
            WEBSITE_DELETE_STATEMENTS,
            [
                "DELETE FROM report WHERE website_id = $1",
                "DELETE FROM segment WHERE website_id = $1",
                "DELETE FROM annotation WHERE website_id = $1",
                "DELETE FROM share WHERE entity_id = $1",
            ]
        );
    }

    #[test]
    fn include_teams_requires_explicit_true() {
        let mut params = HashMap::new();
        assert!(!include_team_access(&params));
        params.insert("includeTeams".to_string(), "TRUE".to_string());
        assert!(include_team_access(&params));
        params.insert("includeTeams".to_string(), "false".to_string());
        assert!(!include_team_access(&params));
    }

    #[test]
    fn list_options_validate_paging_and_sort_direction() {
        let mut params = HashMap::new();
        params.insert("page".to_string(), "0".to_string());
        assert!(list_options(&params, &[("name", "name")], "created_at", true).is_err());

        params.insert("page".to_string(), "2".to_string());
        params.insert("pageSize".to_string(), "10".to_string());
        params.insert("orderBy".to_string(), "name".to_string());
        params.insert("sortDescending".to_string(), "false".to_string());
        let options = list_options(&params, &[("name", "name")], "created_at", true).unwrap();
        assert_eq!(options.offset, 10);
        assert_eq!(options.order_by, "name");
        assert_eq!(options.direction, "ASC");

        params.insert("sortDescending".to_string(), "yes".to_string());
        assert!(list_options(&params, &[("name", "name")], "created_at", true).is_err());
    }

    #[test]
    fn segment_input_matches_node_validation() {
        let (_, name, parameters) = segment_input(&json!({
            "type": "segment",
            "name": " Pricing ",
            "parameters": {
                "filters": [{
                    "name": "path",
                    "operator": "eq",
                    "value": "/pricing",
                    "ignored": true
                }],
                "match": "all",
                "ignored": true
            }
        }))
        .unwrap();
        assert_eq!(name, " Pricing ");
        assert_eq!(
            serde_json::from_str::<Value>(&parameters).unwrap(),
            json!({
                "filters": [{ "name": "path", "operator": "eq", "value": "/pricing" }],
                "match": "all"
            })
        );
        assert!(segment_input(&json!({
            "type": "segment",
            "name": "Pricing",
            "parameters": {
                "filters": [{ "name": "path", "operator": "nope", "value": "/pricing" }]
            }
        }))
        .is_none());
        assert!(segment_input(&json!({
            "type": "cohort",
            "name": "Signups",
            "parameters": { "dateRange": "30day" }
        }))
        .is_none());
        assert!(segment_input(&json!({
            "type": "cohort",
            "name": "Signups",
            "parameters": {
                "dateRange": "30day",
                "action": { "type": "event", "value": "signup" }
            }
        }))
        .is_some());
        assert!(segment_input(&json!({
            "type": "segment",
            "name": "x".repeat(201),
            "parameters": {}
        }))
        .is_none());
    }

    #[test]
    fn recorder_updates_merge_validate_and_disable() {
        let current = Some(
            json!({
                "replayEnabled": true,
                "heatmapEnabled": true,
                "sampleRate": 1
            })
            .to_string(),
        );
        let (config, enabled) =
            update_recorder_config(current, &json!({ "replayEnabled": false })).unwrap();
        let config: Value = serde_json::from_str(&config).unwrap();
        assert_eq!(config.get("replayEnabled"), None);
        assert_eq!(config["heatmapEnabled"], true);
        assert!(enabled);

        let (config, enabled) = update_recorder_config(None, &Value::Null).unwrap();
        assert_eq!(config, "{}");
        assert!(!enabled);
        assert!(update_recorder_config(None, &json!({ "sampleRate": 2 })).is_err());
        assert!(update_recorder_config(None, &json!({ "maxDuration": 1.5 })).is_err());
    }
}
