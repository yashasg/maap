use super::{
    check_bearer_auth, error_response, json_response, page_params as paging, parse_ms_param,
    status_response, with_rust_header, AppState,
};
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::Response,
    routing::{delete, get, post},
    Json, Router,
};
use chrono::{DateTime, Datelike, Duration, TimeZone, Timelike, Utc};
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
        .route("/api/auth/subscription", get(handle_auth_subscription))
        .route("/api/2fa/status", get(handle_two_factor_status))
        .route("/api/me", get(handle_me))
        .route("/api/me/password", post(handle_change_password))
        .route("/api/me/websites", get(handle_me_websites))
        .route("/api/me/teams", get(handle_empty_page))
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
            get(handle_website_shares),
        )
        .route(
            "/api/websites/{website_id}/annotations",
            get(handle_annotations).post(handle_create_annotation),
        )
        .route(
            "/api/websites/{website_id}/annotations/{annotation_id}",
            delete(handle_delete_annotation),
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
        .route("/api/links", get(handle_empty_page))
        .route("/api/links/charts", get(handle_empty_charts))
        .route("/api/pixels", get(handle_empty_page))
        .route("/api/pixels/charts", get(handle_empty_charts))
        .route("/api/boards", get(handle_empty_page))
        .route("/api/teams", get(handle_empty_page))
        .route("/api/admin/users", get(handle_admin_users))
        .route("/api/admin/websites", get(handle_admin_websites))
        .route("/api/admin/teams", get(handle_admin_teams))
        .route(
            "/api/users/{user_id}",
            get(handle_get_user).post(handle_update_user),
        )
        .route("/telemetry.js", get(handle_telemetry_script))
        .route("/api/scripts/telemetry", get(handle_telemetry_script))
}

fn database(state: &AppState) -> Result<&PgPool, Response> {
    state
        .db
        .as_ref()
        .ok_or_else(|| error_response(StatusCode::SERVICE_UNAVAILABLE, "Database unavailable"))
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

async fn authorize_website(
    pool: &PgPool,
    headers: &HeaderMap,
    website_id: Uuid,
    path: &str,
) -> Result<(Uuid, String), Response> {
    let auth = authenticate(pool, headers, path).await?;
    if auth.1 == "admin" {
        return Ok(auth);
    }
    let owned = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM website WHERE website_id = $1 AND user_id = $2 AND deleted_at IS NULL)",
    )
    .bind(website_id)
    .bind(auth.0)
    .fetch_one(pool)
    .await
    .unwrap_or(false);
    if !owned {
        return Err(error_response(StatusCode::UNAUTHORIZED, "Unauthorized"));
    }
    Ok(auth)
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

fn auth_user_json(row: &PgRow) -> Value {
    let username = row.get::<String, _>("username");
    let role = row.get::<String, _>("role");
    let password = row.get::<String, _>("password");
    json!({
        "id": row.get::<Uuid, _>("user_id"),
        "username": &username,
        "role": &role,
        "createdAt": row.get::<Option<DateTime<Utc>>, _>("created_at"),
        "isAdmin": role == "admin",
        "passwordChangeRequired": username == "admin"
            && bcrypt::verify("umami", &password).unwrap_or(false),
        "teams": []
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
        "date": row.get::<DateTime<Utc>, _>("date"),
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

async fn fetch_auth_user(pool: &PgPool, user_id: Uuid) -> Result<Option<PgRow>, sqlx::Error> {
    sqlx::query(
        r#"
        SELECT user_id, username, password, role, created_at
        FROM "user"
        WHERE user_id = $1 AND deleted_at IS NULL
        "#,
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

async fn handle_auth_verify(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (user_id, _) = match authenticate(pool, &headers, "/api/auth/verify").await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    match fetch_auth_user(pool, user_id).await {
        Ok(Some(row)) => json_response(auth_user_json(&row)),
        Ok(None) => status_response(StatusCode::NOT_FOUND),
        Err(_) => status_response(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn handle_me(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (user_id, _) = match authenticate(pool, &headers, "/api/me").await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    match fetch_auth_user(pool, user_id).await {
        Ok(Some(row)) => json_response(auth_user_json(&row)),
        Ok(None) => status_response(StatusCode::NOT_FOUND),
        Err(_) => status_response(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn handle_auth_logout() -> Response {
    json_response(json!({ "ok": true }))
}

async fn handle_auth_subscription() -> Response {
    json_response(json!({
        "hasSubscription": false,
        "status": null,
        "planId": null,
        "isOwner": true,
        "features": {}
    }))
}

async fn handle_two_factor_status(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (user_id, _) = match authenticate(pool, &headers, "/api/2fa/status").await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let is_enabled =
        sqlx::query_scalar::<_, bool>("SELECT is_enabled FROM two_factor_auth WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten()
            .unwrap_or(false);
    json_response(json!({
        "isEnabled": is_enabled,
        "isRequired": false,
        "requiredReason": null,
        "isConfigured": true,
        "globalRequired": false
    }))
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
    let (user_id, _) = match authenticate(pool, &headers, "/api/me/password").await {
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
    if sqlx::query(r#"UPDATE "user" SET password = $1, updated_at = NOW() WHERE user_id = $2"#)
        .bind(hash)
        .bind(user_id)
        .execute(pool)
        .await
        .is_err()
    {
        return status_response(StatusCode::INTERNAL_SERVER_ERROR);
    }
    json_response(json!({ "ok": true }))
}

async fn website_page(
    pool: &PgPool,
    user_id: Uuid,
    params: &HashMap<String, String>,
) -> Result<Value, sqlx::Error> {
    let (page, page_size, offset) = paging(params);
    let search = params
        .get("search")
        .map(|value| format!("%{}%", value.trim()))
        .filter(|value| value != "%%");
    let count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)::bigint
        FROM website
        WHERE user_id = $1
          AND deleted_at IS NULL
          AND ($2::text IS NULL OR name ILIKE $2 OR COALESCE(domain, '') ILIKE $2)
        "#,
    )
    .bind(user_id)
    .bind(search.as_deref())
    .fetch_one(pool)
    .await?;
    let query = format!(
        "{WEBSITE_SELECT}
         WHERE w.user_id = $1
           AND w.deleted_at IS NULL
           AND ($2::text IS NULL OR w.name ILIKE $2 OR COALESCE(w.domain, '') ILIKE $2)
         ORDER BY w.name
         LIMIT $3 OFFSET $4"
    );
    let rows = sqlx::query(&query)
        .bind(user_id)
        .bind(search.as_deref())
        .bind(page_size)
        .bind(offset)
        .fetch_all(pool)
        .await?;
    let data = rows.iter().map(website_json).collect::<Vec<_>>();
    Ok(json!({
        "data": data,
        "count": count,
        "page": page,
        "pageSize": page_size
    }))
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
        Err(_) => status_response(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn handle_websites(
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
        Err(_) => status_response(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn handle_empty_page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    if let Err(response) = authenticate(pool, &headers, "/api").await {
        return response;
    }
    json_response(json!({
        "data": [],
        "count": 0,
        "page": 1,
        "pageSize": 20
    }))
}

async fn handle_empty_charts(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    if let Err(response) = authenticate(pool, &headers, "/api").await {
        return response;
    }
    json_response(json!({ "data": {} }))
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
        WHERE board_id = $1 AND type = 'dashboard'
        "#,
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(row)) => json_response(board_json(&row)),
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
    let name = body.get("name").and_then(Value::as_str);
    let description = body.get("description").and_then(Value::as_str);
    let parameters = body
        .get("parameters")
        .cloned()
        .unwrap_or_else(|| json!({}))
        .to_string();
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
        RETURNING board_id, type, name, description, parameters::text AS parameters,
                  user_id, team_id, created_at, updated_at
        "#,
    )
    .bind(user_id)
    .bind(name)
    .bind(description)
    .bind(parameters)
    .fetch_one(pool)
    .await
    {
        Ok(row) => json_response(board_json(&row)),
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
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (user_id, role) = match authenticate(pool, &headers, "/api/websites/charts").await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
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

    let allowed_ids = if role == "admin" {
        ids
    } else {
        match sqlx::query_scalar::<_, Uuid>(
            "SELECT website_id FROM website WHERE website_id = ANY($1::uuid[]) AND user_id = $2 AND deleted_at IS NULL",
        )
        .bind(&ids)
        .bind(user_id)
        .fetch_all(pool)
        .await
        {
            Ok(values) => values,
            Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
        }
    };

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
    .fetch_all(pool)
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
    .fetch_all(pool)
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

async fn fetch_website(pool: &PgPool, website_id: Uuid) -> Result<Option<PgRow>, sqlx::Error> {
    let query = format!("{WEBSITE_SELECT} WHERE w.website_id = $1 AND w.deleted_at IS NULL");
    sqlx::query(&query)
        .bind(website_id)
        .fetch_optional(pool)
        .await
}

async fn handle_get_website(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    headers: HeaderMap,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    if let Err(response) = authorize_website(pool, &headers, website_id, "/api/websites").await {
        return response;
    }
    match fetch_website(pool, website_id).await {
        Ok(Some(row)) => json_response(website_json(&row)),
        Ok(None) => status_response(StatusCode::NOT_FOUND),
        Err(_) => status_response(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn handle_update_website(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    if let Err(response) = authorize_website(pool, &headers, website_id, "/api/websites").await {
        return response;
    }
    let name = body.get("name").and_then(Value::as_str);
    let domain_present = body.get("domain").is_some();
    let domain = body.get("domain").and_then(Value::as_str);
    let share_present = body.get("shareId").is_some();
    let share_id = body.get("shareId").and_then(Value::as_str);
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
    };
    let updated = sqlx::query(
        r#"
        UPDATE website
        SET name = COALESCE($1, name),
            domain = CASE WHEN $2 THEN $3 ELSE domain END,
            updated_at = NOW()
        WHERE website_id = $4 AND deleted_at IS NULL
        "#,
    )
    .bind(name)
    .bind(domain_present)
    .bind(domain)
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
    match fetch_website(pool, website_id).await {
        Ok(Some(row)) => json_response(website_json(&row)),
        Ok(None) => status_response(StatusCode::NOT_FOUND),
        Err(_) => status_response(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn delete_analytics(pool: &PgPool, website_id: Uuid) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    for statement in [
        "DELETE FROM event_data WHERE website_id = $1",
        "DELETE FROM session_data WHERE website_id = $1",
        "DELETE FROM session_link WHERE website_id = $1",
        "DELETE FROM revenue WHERE website_id = $1",
        "DELETE FROM website_event WHERE website_id = $1",
        "DELETE FROM session WHERE website_id = $1",
    ] {
        sqlx::query(statement)
            .bind(website_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await
}

async fn handle_delete_website(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    headers: HeaderMap,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    if let Err(response) = authorize_website(pool, &headers, website_id, "/api/websites").await {
        return response;
    }
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
    };
    for statement in [
        "DELETE FROM event_data WHERE website_id = $1",
        "DELETE FROM session_data WHERE website_id = $1",
        "DELETE FROM session_link WHERE website_id = $1",
        "DELETE FROM revenue WHERE website_id = $1",
        "DELETE FROM website_event WHERE website_id = $1",
        "DELETE FROM session WHERE website_id = $1",
        "DELETE FROM share WHERE entity_id = $1",
        "DELETE FROM website WHERE website_id = $1",
    ] {
        if sqlx::query(statement)
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
    json_response(json!({ "ok": true }))
}

async fn handle_reset_website(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    headers: HeaderMap,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    if let Err(response) = authorize_website(pool, &headers, website_id, "/api/websites").await {
        return response;
    }
    if delete_analytics(pool, website_id).await.is_err()
        || sqlx::query(
            "UPDATE website SET reset_at = NOW(), updated_at = NOW() WHERE website_id = $1",
        )
        .bind(website_id)
        .execute(pool)
        .await
        .is_err()
    {
        return status_response(StatusCode::INTERNAL_SERVER_ERROR);
    }
    json_response(json!({ "ok": true }))
}

async fn handle_website_shares(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    headers: HeaderMap,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    if let Err(response) = authorize_website(pool, &headers, website_id, "/api/websites").await {
        return response;
    }
    let rows = match sqlx::query(
        r#"
        SELECT share_id, entity_id, name, share_type, slug,
               parameters::text AS parameters, created_at, updated_at
        FROM share
        WHERE entity_id = $1
        ORDER BY created_at DESC
        "#,
    )
    .bind(website_id)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
    };
    let data = rows.iter().map(share_json).collect::<Vec<_>>();
    json_response(json!({ "count": data.len(), "data": data }))
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
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    if let Err(response) = authorize_website(pool, &headers, website_id, "/api/websites").await {
        return response;
    }
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
    .fetch_one(pool)
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
    .fetch_all(pool)
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
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (user_id, _) = match authorize_website(pool, &headers, website_id, "/api/websites").await {
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
    .fetch_one(pool)
    .await
    {
        Ok(row) => json_response(annotation_json(&row)),
        Err(_) => status_response(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn handle_delete_annotation(
    State(state): State<AppState>,
    Path((website_id, annotation_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    if let Err(response) = authorize_website(pool, &headers, website_id, "/api/websites").await {
        return response;
    }
    match sqlx::query("DELETE FROM annotation WHERE annotation_id = $1 AND website_id = $2")
        .bind(annotation_id)
        .bind(website_id)
        .execute(pool)
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
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    if let Err(response) = authorize_website(pool, &headers, website_id, "/api/websites").await {
        return response;
    }
    let (page, page_size, offset) = paging(&params);
    let search = params
        .get("search")
        .map(|value| format!("%{}%", value.trim()))
        .filter(|value| value != "%%");
    let segment_type = params.get("type").map(String::as_str);
    let count = match sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)::bigint
        FROM segment
        WHERE website_id = $1
          AND ($2::text IS NULL OR type = $2)
          AND ($3::text IS NULL OR name ILIKE $3)
        "#,
    )
    .bind(website_id)
    .bind(segment_type)
    .bind(search.as_deref())
    .fetch_one(pool)
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
          AND ($2::text IS NULL OR type = $2)
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
    .fetch_all(pool)
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
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    if let Err(response) = authorize_website(pool, &headers, website_id, "/api/websites").await {
        return response;
    }
    let segment_type = body.get("type").and_then(Value::as_str).unwrap_or("");
    let name = body
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let parameters = body
        .get("parameters")
        .cloned()
        .unwrap_or_else(|| json!({}))
        .to_string();
    if segment_type.is_empty() || name.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "Invalid segment");
    }
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
    .bind(segment_type)
    .bind(name)
    .bind(parameters)
    .fetch_one(pool)
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
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    if let Err(response) = authorize_website(pool, &headers, website_id, "/api/websites").await {
        return response;
    }
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
    .fetch_optional(pool)
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
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    if let Err(response) = authorize_website(pool, &headers, website_id, "/api/websites").await {
        return response;
    }
    let segment_type = body.get("type").and_then(Value::as_str);
    let name = body.get("name").and_then(Value::as_str);
    let parameters = body.get("parameters").map(Value::to_string);
    match sqlx::query(
        r#"
        UPDATE segment
        SET type = COALESCE($1, type),
            name = COALESCE($2, name),
            parameters = CASE WHEN $3::text IS NULL THEN parameters ELSE $3::jsonb END,
            updated_at = NOW()
        WHERE segment_id = $4 AND website_id = $5
        RETURNING segment_id, website_id, type, name, parameters::text AS parameters,
                  created_at, updated_at
        "#,
    )
    .bind(segment_type)
    .bind(name)
    .bind(parameters)
    .bind(segment_id)
    .bind(website_id)
    .fetch_optional(pool)
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
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    if let Err(response) = authorize_website(pool, &headers, website_id, "/api/websites").await {
        return response;
    }
    match sqlx::query("DELETE FROM segment WHERE segment_id = $1 AND website_id = $2")
        .bind(segment_id)
        .bind(website_id)
        .execute(pool)
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
    let (page, page_size, offset) = paging(&params);
    let count = match sqlx::query_scalar::<_, i64>(
        r#"SELECT COUNT(*)::bigint FROM "user" WHERE deleted_at IS NULL"#,
    )
    .fetch_one(pool)
    .await
    {
        Ok(count) => count,
        Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
    };
    let rows = match sqlx::query(
        r#"
        SELECT u.user_id, u.username, u.role, u.created_at, u.two_factor_required,
               COUNT(w.website_id)::bigint AS website_count
        FROM "user" u
        LEFT JOIN website w ON w.user_id = u.user_id AND w.deleted_at IS NULL
        WHERE u.deleted_at IS NULL
        GROUP BY u.user_id
        ORDER BY u.created_at DESC
        LIMIT $1 OFFSET $2
        "#,
    )
    .bind(page_size)
    .bind(offset)
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
        "page": page,
        "pageSize": page_size
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
    let (page, page_size, offset) = paging(&params);
    let count = match sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)::bigint FROM website WHERE deleted_at IS NULL",
    )
    .fetch_one(pool)
    .await
    {
        Ok(count) => count,
        Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
    };
    let query = r#"
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
        ORDER BY w.created_at DESC
        LIMIT $1 OFFSET $2
        "#;
    let rows = match sqlx::query(query)
        .bind(page_size)
        .bind(offset)
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
        "page": page,
        "pageSize": page_size
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
    let (page, page_size, offset) = paging(&params);
    let count = match sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)::bigint FROM team WHERE deleted_at IS NULL",
    )
    .fetch_one(pool)
    .await
    {
        Ok(count) => count,
        Err(_) => return status_response(StatusCode::INTERNAL_SERVER_ERROR),
    };
    let rows = match sqlx::query(
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
        GROUP BY t.team_id
        ORDER BY t.created_at DESC
        LIMIT $1 OFFSET $2
        "#,
    )
    .bind(page_size)
    .bind(offset)
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
        "page": page,
        "pageSize": page_size
    }))
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

async fn handle_telemetry_script() -> Response {
    let mut response = Response::new(Body::from(""));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/javascript; charset=utf-8"),
    );
    with_rust_header(response)
}
