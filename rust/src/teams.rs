use super::{check_bearer_auth, error_response, json_response, AppState};
use axum::{
    extract::{rejection::JsonRejection, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use rand::{distributions::Alphanumeric, Rng};
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Map, Value};
use sqlx::{postgres::PgRow, PgPool, Postgres, Row, Transaction};
use std::{collections::HashMap, env};
use uuid::Uuid;

const OWNER: &str = "team-owner";
const MANAGER: &str = "team-manager";
const MEMBER: &str = "team-member";
const VIEW_ONLY: &str = "team-view-only";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateTeam {
    name: String,
    owner_id: Option<Uuid>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateTeam {
    name: Option<String>,
    access_code: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JoinTeam {
    access_code: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddMember {
    user_id: Uuid,
    role: String,
}

#[derive(Deserialize)]
struct UpdateMember {
    role: String,
}

#[derive(Clone, Copy)]
enum TeamPermission {
    View,
    Update,
    Delete,
}

struct Page {
    number: i64,
    size: i64,
    offset: i64,
    order_by: Option<String>,
    descending: bool,
    search: Option<String>,
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
) -> Result<(Uuid, bool), Response> {
    check_bearer_auth(pool, headers, path)
        .await
        .map(|(user_id, role)| (user_id, role == "admin"))
        .ok_or_else(|| error_response(StatusCode::UNAUTHORIZED, "Unauthorized"))
}

fn api_error(status: StatusCode, code: &str, message: &str) -> Response {
    super::with_rust_header(
        (
            status,
            Json(json!({
                "error": {
                    "code": code,
                    "message": message,
                    "status": status.as_u16()
                }
            })),
        )
            .into_response(),
    )
}

fn parse_body<T: DeserializeOwned>(
    body: Result<Json<Value>, JsonRejection>,
    fields: &[&str],
) -> Result<T, Response> {
    let Json(value) =
        body.map_err(|_| error_response(StatusCode::BAD_REQUEST, "Invalid request"))?;
    if fields
        .iter()
        .any(|field| value.get(*field).is_some_and(Value::is_null))
    {
        return Err(error_response(StatusCode::BAD_REQUEST, "Invalid request"));
    }
    serde_json::from_value(value)
        .map_err(|_| error_response(StatusCode::BAD_REQUEST, "Invalid request"))
}

fn valid_team_role(role: &str) -> bool {
    matches!(role, MEMBER | VIEW_ONLY | MANAGER)
}

fn role_rank(role: &str) -> i32 {
    match role {
        VIEW_ONLY => 0,
        MEMBER => 1,
        MANAGER => 2,
        OWNER => 3,
        _ => -1,
    }
}

fn role_allows(role: Option<&str>, permission: TeamPermission) -> bool {
    match permission {
        TeamPermission::View => role.is_some(),
        TeamPermission::Update => matches!(role, Some(OWNER | MANAGER)),
        TeamPermission::Delete => role == Some(OWNER),
    }
}

fn exposes_access_code(role: Option<&str>) -> bool {
    matches!(role, Some(OWNER | MANAGER))
}

fn removing_last_owner(target_role: &str, owner_count: usize) -> bool {
    target_role == OWNER && owner_count <= 1
}

async fn team_role(
    pool: &PgPool,
    team_id: Uuid,
    user_id: Uuid,
) -> Result<Option<String>, Response> {
    sqlx::query_scalar(
        r#"
        SELECT tu.role
        FROM team_user tu
        JOIN team t ON t.team_id = tu.team_id
        WHERE tu.team_id = $1 AND tu.user_id = $2 AND t.deleted_at IS NULL
        "#,
    )
    .bind(team_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(|_| error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"))
}

async fn authorize_team(
    pool: &PgPool,
    user_id: Uuid,
    admin: bool,
    team_id: Uuid,
    permission: TeamPermission,
) -> Result<Option<String>, Response> {
    if admin {
        return Ok(None);
    }
    let role = team_role(pool, team_id, user_id).await?;
    if role_allows(role.as_deref(), permission) {
        Ok(role)
    } else {
        Err(error_response(StatusCode::UNAUTHORIZED, "Unauthorized"))
    }
}

fn page(
    params: &HashMap<String, String>,
    allowed_sort: &[&str],
    allow_search: bool,
) -> Result<Page, Response> {
    let positive = |key: &str, default| -> Result<i64, Response> {
        match params.get(key) {
            None => Ok(default),
            Some(value) => value
                .parse::<i64>()
                .ok()
                .filter(|value| *value > 0)
                .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, "Invalid query")),
        }
    };
    let number = positive("page", 1)?;
    let size = positive("pageSize", 20)?;
    let descending = match params.get("sortDescending").map(String::as_str) {
        None | Some("false") => false,
        Some("true") => true,
        Some(_) => return Err(error_response(StatusCode::BAD_REQUEST, "Invalid query")),
    };
    let order_by = params
        .get("orderBy")
        .filter(|value| allowed_sort.contains(&value.as_str()))
        .cloned();
    Ok(Page {
        number,
        size,
        offset: (number - 1) * size,
        order_by,
        descending,
        search: allow_search
            .then(|| params.get("search").cloned())
            .flatten(),
    })
}

fn paged_json(data: Vec<Value>, count: i64, page: &Page) -> Value {
    let mut value = Map::from_iter([
        ("data".to_string(), Value::Array(data)),
        ("count".to_string(), json!(count)),
        ("page".to_string(), json!(page.number)),
        ("pageSize".to_string(), json!(page.size)),
    ]);
    if let Some(order_by) = &page.order_by {
        value.insert("orderBy".to_string(), json!(order_by));
    }
    if let Some(search) = &page.search {
        value.insert("search".to_string(), json!(search));
    }
    Value::Object(value)
}

fn team_json(row: &PgRow, show_access_code: bool) -> Value {
    let mut value = json!({
        "id": row.get::<Uuid, _>("team_id"),
        "name": row.get::<String, _>("name"),
        "accessCode": row.get::<Option<String>, _>("access_code"),
        "logoUrl": row.get::<Option<String>, _>("logo_url"),
        "twoFactorRequired": row.get::<bool, _>("two_factor_required"),
        "createdAt": row.get::<Option<DateTime<Utc>>, _>("created_at"),
        "updatedAt": row.get::<Option<DateTime<Utc>>, _>("updated_at")
    });
    if !show_access_code {
        value.as_object_mut().unwrap().remove("accessCode");
    }
    value
}

fn member_json(row: &PgRow, include_user: bool) -> Value {
    let mut value = json!({
        "id": row.get::<Uuid, _>("team_user_id"),
        "teamId": row.get::<Uuid, _>("team_id"),
        "userId": row.get::<Uuid, _>("user_id"),
        "role": row.get::<String, _>("role"),
        "createdAt": row.get::<Option<DateTime<Utc>>, _>("created_at"),
        "updatedAt": row.get::<Option<DateTime<Utc>>, _>("updated_at")
    });
    if include_user {
        value.as_object_mut().unwrap().insert(
            "user".to_string(),
            json!({
                "id": row.get::<Uuid, _>("user_id"),
                "username": row.get::<String, _>("username")
            }),
        );
    }
    value
}

fn parse_json_text(value: Option<String>) -> Value {
    value
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or(Value::Null)
}

async fn member_rows(pool: &PgPool, team_id: Uuid) -> Result<Vec<PgRow>, Response> {
    sqlx::query(
        r#"
        SELECT tu.team_user_id, tu.team_id, tu.user_id, tu.role, tu.created_at, tu.updated_at,
               u.username
        FROM team_user tu
        JOIN "user" u ON u.user_id = tu.user_id
        WHERE tu.team_id = $1
        ORDER BY tu.created_at ASC
        "#,
    )
    .bind(team_id)
    .fetch_all(pool)
    .await
    .map_err(|_| error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"))
}

async fn list_user_teams(
    pool: &PgPool,
    target_user_id: Uuid,
    params: &HashMap<String, String>,
) -> Result<Value, Response> {
    let page = page(params, &["name", "createdAt"], false)?;
    let count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)::bigint
        FROM team t
        JOIN team_user actor ON actor.team_id = t.team_id AND actor.user_id = $1
        WHERE t.deleted_at IS NULL
        "#,
    )
    .bind(target_user_id)
    .fetch_one(pool)
    .await
    .map_err(|_| error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let order = match page.order_by.as_deref() {
        Some("name") => "t.name",
        Some("createdAt") => "t.created_at",
        _ => "t.created_at",
    };
    let direction = if page.descending { "DESC" } else { "ASC" };
    let sql = format!(
        r#"
        SELECT t.team_id, t.name, t.access_code, t.logo_url, t.two_factor_required,
               t.created_at, t.updated_at, actor.role AS actor_role,
               (SELECT COUNT(*)::bigint FROM website w
                WHERE w.team_id = t.team_id AND w.deleted_at IS NULL) AS website_count,
               (SELECT COUNT(*)::bigint
                FROM team_user counted
                JOIN "user" u ON u.user_id = counted.user_id AND u.deleted_at IS NULL
                WHERE counted.team_id = t.team_id) AS member_count
        FROM team t
        JOIN team_user actor ON actor.team_id = t.team_id AND actor.user_id = $1
        WHERE t.deleted_at IS NULL
        ORDER BY {order} {direction}
        LIMIT $2 OFFSET $3
        "#
    );
    let rows = sqlx::query(&sql)
        .bind(target_user_id)
        .bind(page.size)
        .bind(page.offset)
        .fetch_all(pool)
        .await
        .map_err(|_| error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let mut data = Vec::with_capacity(rows.len());
    for row in rows {
        let team_id = row.get::<Uuid, _>("team_id");
        let role = row.get::<String, _>("actor_role");
        let members = member_rows(pool, team_id)
            .await?
            .iter()
            .map(|row| member_json(row, true))
            .collect::<Vec<_>>();
        let mut team = team_json(&row, exposes_access_code(Some(&role)));
        let object = team.as_object_mut().unwrap();
        object.insert("members".to_string(), Value::Array(members));
        object.insert(
            "_count".to_string(),
            json!({
                "websites": row.get::<i64, _>("website_count"),
                "members": row.get::<i64, _>("member_count")
            }),
        );
        data.push(team);
    }
    Ok(paged_json(data, count, &page))
}

async fn handle_list_teams(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (user_id, _) = match authenticate(pool, &headers, "/api/teams").await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    match list_user_teams(pool, user_id, &params).await {
        Ok(value) => json_response(value),
        Err(response) => response,
    }
}

async fn handle_create_team(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<Value>, JsonRejection>,
) -> Response {
    let body = match parse_body::<CreateTeam>(body, &["name", "ownerId"]) {
        Ok(body) if body.name.chars().count() <= 50 => body,
        _ => return error_response(StatusCode::BAD_REQUEST, "Invalid request"),
    };
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (user_id, admin) = match authenticate(pool, &headers, "/api/teams").await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    if !admin {
        let role = match sqlx::query_scalar::<_, String>(
            r#"SELECT role FROM "user" WHERE user_id = $1 AND deleted_at IS NULL"#,
        )
        .bind(user_id)
        .fetch_optional(pool)
        .await
        {
            Ok(role) => role,
            Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
        };
        if role.as_deref() != Some("user") {
            return error_response(StatusCode::UNAUTHORIZED, "Unauthorized");
        }
    }
    let owner_id = if admin {
        body.owner_id.unwrap_or(user_id)
    } else {
        user_id
    };
    let team_id = Uuid::new_v4();
    let member_id = Uuid::new_v4();
    let access_code = format!(
        "team_{}",
        rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(16)
            .map(char::from)
            .collect::<String>()
    );
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
    };
    let team = match sqlx::query(
        r#"
        INSERT INTO team (team_id, name, access_code, updated_at)
        VALUES ($1, $2, $3, CURRENT_TIMESTAMP)
        RETURNING team_id, name, access_code, logo_url, two_factor_required, created_at, updated_at
        "#,
    )
    .bind(team_id)
    .bind(&body.name)
    .bind(&access_code)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(row) => row,
        Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
    };
    let member = match sqlx::query(
        r#"
        INSERT INTO team_user (team_user_id, team_id, user_id, role, updated_at)
        VALUES ($1, $2, $3, $4, CURRENT_TIMESTAMP)
        RETURNING team_user_id, team_id, user_id, role, created_at, updated_at
        "#,
    )
    .bind(member_id)
    .bind(team_id)
    .bind(owner_id)
    .bind(OWNER)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(row) => row,
        Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
    };
    if tx.commit().await.is_err() {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error");
    }
    json_response(json!([team_json(&team, true), member_json(&member, false)]))
}

async fn handle_get_team(
    State(state): State<AppState>,
    Path(team_id): Path<Uuid>,
    headers: HeaderMap,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (user_id, admin) = match authenticate(pool, &headers, "/api/teams").await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let actor_role = match authorize_team(pool, user_id, admin, team_id, TeamPermission::View).await
    {
        Ok(role) => role,
        Err(response) => return response,
    };
    let row = match sqlx::query(
        r#"
        SELECT team_id, name, access_code, logo_url, two_factor_required, created_at, updated_at
        FROM team
        WHERE team_id = $1 AND deleted_at IS NULL
        "#,
    )
    .bind(team_id)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Team not found."),
        Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
    };
    let members = match member_rows(pool, team_id).await {
        Ok(rows) => rows
            .iter()
            .map(|row| member_json(row, false))
            .collect::<Vec<_>>(),
        Err(response) => return response,
    };
    let show_access_code = admin || exposes_access_code(actor_role.as_deref());
    let mut team = team_json(&row, show_access_code);
    team.as_object_mut()
        .unwrap()
        .insert("members".to_string(), Value::Array(members));
    json_response(team)
}

async fn handle_update_team(
    State(state): State<AppState>,
    Path(team_id): Path<Uuid>,
    headers: HeaderMap,
    body: Result<Json<Value>, JsonRejection>,
) -> Response {
    let body = match parse_body::<UpdateTeam>(body, &["name", "accessCode"]) {
        Ok(body)
            if body
                .name
                .as_ref()
                .map_or(true, |name| name.chars().count() <= 50)
                && body
                    .access_code
                    .as_ref()
                    .map_or(true, |code| code.chars().count() <= 50) =>
        {
            body
        }
        _ => return error_response(StatusCode::BAD_REQUEST, "Invalid request"),
    };
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (user_id, admin) = match authenticate(pool, &headers, "/api/teams").await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    if let Err(response) =
        authorize_team(pool, user_id, admin, team_id, TeamPermission::Update).await
    {
        return response;
    }
    let row = match sqlx::query(
        r#"
        UPDATE team
        SET name = COALESCE($2, name),
            access_code = COALESCE($3, access_code),
            updated_at = CURRENT_TIMESTAMP
        WHERE team_id = $1 AND deleted_at IS NULL
        RETURNING team_id, name, access_code, logo_url, two_factor_required, created_at, updated_at
        "#,
    )
    .bind(team_id)
    .bind(body.name)
    .bind(body.access_code)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Team not found."),
        Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
    };
    json_response(team_json(&row, true))
}

async fn delete_team_data(
    tx: &mut Transaction<'_, Postgres>,
    team_id: Uuid,
) -> Result<(), sqlx::Error> {
    let website_tables = [
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
    ];
    for table in website_tables {
        sqlx::query(&format!(
            "DELETE FROM {table} WHERE website_id IN (SELECT website_id FROM website WHERE team_id = $1)"
        ))
        .bind(team_id)
        .execute(&mut **tx)
        .await?;
    }
    sqlx::query(
        r#"
        DELETE FROM event_data
        WHERE website_event_id IN (
            SELECT event_id FROM website_event
            WHERE website_id IN (SELECT website_id FROM website WHERE team_id = $1)
        )
        "#,
    )
    .bind(team_id)
    .execute(&mut **tx)
    .await?;
    for table in [
        "website_event",
        "session",
        "report",
        "segment",
        "annotation",
    ] {
        sqlx::query(&format!(
            "DELETE FROM {table} WHERE website_id IN (SELECT website_id FROM website WHERE team_id = $1)"
        ))
        .bind(team_id)
        .execute(&mut **tx)
        .await?;
    }
    sqlx::query(
        "DELETE FROM share WHERE entity_id IN (SELECT website_id FROM website WHERE team_id = $1)",
    )
    .bind(team_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        r#"
        DELETE FROM share WHERE entity_id IN (
            SELECT link_id FROM link WHERE team_id = $1
            UNION SELECT pixel_id FROM pixel WHERE team_id = $1
            UNION SELECT board_id FROM board WHERE team_id = $1
        )
        "#,
    )
    .bind(team_id)
    .execute(&mut **tx)
    .await?;

    if env::var("CLOUD_MODE").is_ok_and(|value| !value.is_empty()) {
        sqlx::query(
            "UPDATE website SET deleted_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP WHERE team_id = $1 AND deleted_at IS NULL",
        )
        .bind(team_id)
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            "UPDATE link SET deleted_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP WHERE team_id = $1 AND deleted_at IS NULL",
        )
        .bind(team_id)
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            "UPDATE pixel SET deleted_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP WHERE team_id = $1 AND deleted_at IS NULL",
        )
        .bind(team_id)
        .execute(&mut **tx)
        .await?;
        sqlx::query("DELETE FROM board WHERE team_id = $1")
            .bind(team_id)
            .execute(&mut **tx)
            .await?;
        sqlx::query(
            "UPDATE team SET deleted_at = CURRENT_TIMESTAMP, access_code = NULL, updated_at = CURRENT_TIMESTAMP WHERE team_id = $1",
        )
        .bind(team_id)
        .execute(&mut **tx)
        .await?;
    } else {
        for table in ["website", "team_user", "link", "pixel", "board", "team"] {
            sqlx::query(&format!("DELETE FROM {table} WHERE team_id = $1"))
                .bind(team_id)
                .execute(&mut **tx)
                .await?;
        }
    }
    Ok(())
}

async fn handle_delete_team(
    State(state): State<AppState>,
    Path(team_id): Path<Uuid>,
    headers: HeaderMap,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (user_id, admin) = match authenticate(pool, &headers, "/api/teams").await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
    };
    let exists = sqlx::query_scalar::<_, Uuid>(
        "SELECT team_id FROM team WHERE team_id = $1 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(team_id)
    .fetch_optional(&mut *tx)
    .await;
    match exists {
        Ok(Some(_)) => {}
        Ok(None) if admin => return error_response(StatusCode::NOT_FOUND, "Team not found."),
        Ok(None) => return error_response(StatusCode::UNAUTHORIZED, "Unauthorized"),
        Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
    }
    if !admin {
        let role = match sqlx::query_scalar::<_, String>(
            "SELECT role FROM team_user WHERE team_id = $1 AND user_id = $2 FOR UPDATE",
        )
        .bind(team_id)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await
        {
            Ok(role) => role,
            Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
        };
        if !role_allows(role.as_deref(), TeamPermission::Delete) {
            return error_response(StatusCode::UNAUTHORIZED, "Unauthorized");
        }
    }
    if delete_team_data(&mut tx, team_id).await.is_err() || tx.commit().await.is_err() {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error");
    }
    json_response(json!({ "ok": true }))
}

async fn handle_join_team(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<Value>, JsonRejection>,
) -> Response {
    let body = match parse_body::<JoinTeam>(body, &["accessCode"]) {
        Ok(body) if body.access_code.chars().count() <= 50 => body,
        _ => return error_response(StatusCode::BAD_REQUEST, "Invalid request"),
    };
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (user_id, _) = match authenticate(pool, &headers, "/api/teams/join").await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
    };
    let team_id = match sqlx::query_scalar::<_, Uuid>(
        "SELECT team_id FROM team WHERE access_code = $1 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(&body.access_code)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(Some(team_id)) => team_id,
        Ok(None) => return api_error(StatusCode::NOT_FOUND, "team-not-found", "Team not found."),
        Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
    };
    match sqlx::query_scalar::<_, Uuid>(
        "SELECT team_user_id FROM team_user WHERE team_id = $1 AND user_id = $2 FOR UPDATE",
    )
    .bind(team_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(None) => {}
        Ok(Some(_)) => {
            return error_response(StatusCode::BAD_REQUEST, "User is already a team member.")
        }
        Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
    }
    let row = match sqlx::query(
        r#"
        INSERT INTO team_user (team_user_id, team_id, user_id, role, updated_at)
        VALUES ($1, $2, $3, $4, CURRENT_TIMESTAMP)
        RETURNING team_user_id, team_id, user_id, role, created_at, updated_at
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(team_id)
    .bind(user_id)
    .bind(MEMBER)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(row) => row,
        Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
    };
    if tx.commit().await.is_err() {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error");
    }
    json_response(member_json(&row, false))
}

async fn handle_list_members(
    State(state): State<AppState>,
    Path(team_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (user_id, admin) = match authenticate(pool, &headers, "/api/teams").await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    if let Err(response) = authorize_team(pool, user_id, admin, team_id, TeamPermission::View).await
    {
        return response;
    }
    let page = match page(&params, &[], true) {
        Ok(page) => page,
        Err(response) => return response,
    };
    let search = page.search.clone().unwrap_or_default();
    let count = match sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)::bigint
        FROM team_user tu
        JOIN "user" u ON u.user_id = tu.user_id AND u.deleted_at IS NULL
        WHERE tu.team_id = $1 AND ($2 = '' OR u.username ILIKE '%' || $2 || '%')
        "#,
    )
    .bind(team_id)
    .bind(&search)
    .fetch_one(pool)
    .await
    {
        Ok(count) => count,
        Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
    };
    let rows = match sqlx::query(
        r#"
        SELECT tu.team_user_id, tu.team_id, tu.user_id, tu.role, tu.created_at, tu.updated_at,
               u.username
        FROM team_user tu
        JOIN "user" u ON u.user_id = tu.user_id AND u.deleted_at IS NULL
        WHERE tu.team_id = $1 AND ($2 = '' OR u.username ILIKE '%' || $2 || '%')
        ORDER BY tu.created_at ASC
        LIMIT $3 OFFSET $4
        "#,
    )
    .bind(team_id)
    .bind(search)
    .bind(page.size)
    .bind(page.offset)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
    };
    json_response(paged_json(
        rows.iter().map(|row| member_json(row, true)).collect(),
        count,
        &page,
    ))
}

async fn lock_members(
    tx: &mut Transaction<'_, Postgres>,
    team_id: Uuid,
) -> Result<Vec<PgRow>, sqlx::Error> {
    sqlx::query(
        r#"
        SELECT team_user_id, team_id, user_id, role, created_at, updated_at
        FROM team_user
        WHERE team_id = $1
        ORDER BY team_user_id
        FOR UPDATE
        "#,
    )
    .bind(team_id)
    .fetch_all(&mut **tx)
    .await
}

async fn handle_add_member(
    State(state): State<AppState>,
    Path(team_id): Path<Uuid>,
    headers: HeaderMap,
    body: Result<Json<Value>, JsonRejection>,
) -> Response {
    let body = match parse_body::<AddMember>(body, &["userId", "role"]) {
        Ok(body) if valid_team_role(&body.role) => body,
        _ => return error_response(StatusCode::BAD_REQUEST, "Invalid request"),
    };
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (actor_id, admin) = match authenticate(pool, &headers, "/api/teams").await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
    };
    let team_exists = sqlx::query_scalar::<_, Uuid>(
        "SELECT team_id FROM team WHERE team_id = $1 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(team_id)
    .fetch_optional(&mut *tx)
    .await;
    match team_exists {
        Ok(Some(_)) => {}
        Ok(None) if admin => return error_response(StatusCode::BAD_REQUEST, "Team not found."),
        Ok(None) => return error_response(StatusCode::UNAUTHORIZED, "Unauthorized"),
        Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
    }
    let members = match lock_members(&mut tx, team_id).await {
        Ok(members) => members,
        Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
    };
    let actor_role = members
        .iter()
        .find(|row| row.get::<Uuid, _>("user_id") == actor_id)
        .map(|row| row.get::<String, _>("role"));
    if !admin && !role_allows(actor_role.as_deref(), TeamPermission::Update) {
        return error_response(StatusCode::UNAUTHORIZED, "Unauthorized");
    }
    if members
        .iter()
        .any(|row| row.get::<Uuid, _>("user_id") == body.user_id)
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            "User is already a member of the Team.",
        );
    }
    let row = match sqlx::query(
        r#"
        INSERT INTO team_user (team_user_id, team_id, user_id, role, updated_at)
        VALUES ($1, $2, $3, $4, CURRENT_TIMESTAMP)
        RETURNING team_user_id, team_id, user_id, role, created_at, updated_at
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(team_id)
    .bind(body.user_id)
    .bind(body.role)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(row) => row,
        Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
    };
    if tx.commit().await.is_err() {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error");
    }
    json_response(member_json(&row, false))
}

async fn handle_get_member(
    State(state): State<AppState>,
    Path((team_id, target_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (actor_id, admin) = match authenticate(pool, &headers, "/api/teams").await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    if let Err(response) =
        authorize_team(pool, actor_id, admin, team_id, TeamPermission::Update).await
    {
        return response;
    }
    match sqlx::query(
        r#"
        SELECT tu.team_user_id, tu.team_id, tu.user_id, tu.role, tu.created_at, tu.updated_at
        FROM team_user tu
        JOIN team t ON t.team_id = tu.team_id AND t.deleted_at IS NULL
        WHERE tu.team_id = $1 AND tu.user_id = $2
        "#,
    )
    .bind(team_id)
    .bind(target_id)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(row)) => json_response(member_json(&row, false)),
        Ok(None) => json_response(Value::Null),
        Err(_) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
    }
}

async fn handle_update_member(
    State(state): State<AppState>,
    Path((team_id, target_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    body: Result<Json<Value>, JsonRejection>,
) -> Response {
    let body = match parse_body::<UpdateMember>(body, &["role"]) {
        Ok(body) if valid_team_role(&body.role) => body,
        _ => return error_response(StatusCode::BAD_REQUEST, "Invalid request"),
    };
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (actor_id, admin) = match authenticate(pool, &headers, "/api/teams").await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
    };
    let members = match lock_members(&mut tx, team_id).await {
        Ok(members) => members,
        Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
    };
    let actor_role = members
        .iter()
        .find(|row| row.get::<Uuid, _>("user_id") == actor_id)
        .map(|row| row.get::<String, _>("role"));
    if !admin && !role_allows(actor_role.as_deref(), TeamPermission::Update) {
        return error_response(StatusCode::UNAUTHORIZED, "Unauthorized");
    }
    let Some(target) = members
        .iter()
        .find(|row| row.get::<Uuid, _>("user_id") == target_id)
    else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "The User does not exists on this team.",
        );
    };
    let target_role = target.get::<String, _>("role");
    if !admin {
        if role_rank(actor_role.as_deref().unwrap_or("")) <= role_rank(&target_role) {
            return error_response(StatusCode::UNAUTHORIZED, "Unauthorized");
        }
    }
    let owner_count = members
        .iter()
        .filter(|row| row.get::<String, _>("role") == OWNER)
        .count();
    if removing_last_owner(&target_role, owner_count) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "A team must have at least one owner.",
        );
    }
    let row = match sqlx::query(
        r#"
        UPDATE team_user SET role = $3, updated_at = CURRENT_TIMESTAMP
        WHERE team_id = $1 AND user_id = $2
        RETURNING team_user_id, team_id, user_id, role, created_at, updated_at
        "#,
    )
    .bind(team_id)
    .bind(target_id)
    .bind(body.role)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(row) => row,
        Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
    };
    if tx.commit().await.is_err() {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error");
    }
    json_response(member_json(&row, false))
}

async fn handle_delete_member(
    State(state): State<AppState>,
    Path((team_id, target_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (actor_id, admin) = match authenticate(pool, &headers, "/api/teams").await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
    };
    let members = match lock_members(&mut tx, team_id).await {
        Ok(members) => members,
        Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
    };
    let actor_role = members
        .iter()
        .find(|row| row.get::<Uuid, _>("user_id") == actor_id)
        .map(|row| row.get::<String, _>("role"));
    if !admin
        && actor_id != target_id
        && !role_allows(actor_role.as_deref(), TeamPermission::Update)
    {
        return error_response(StatusCode::UNAUTHORIZED, "Unauthorized");
    }
    let Some(target) = members
        .iter()
        .find(|row| row.get::<Uuid, _>("user_id") == target_id)
    else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "The User does not exists on this team.",
        );
    };
    let target_role = target.get::<String, _>("role");
    if !admin {
        if actor_id == target_id {
            if target_role == OWNER {
                return error_response(StatusCode::UNAUTHORIZED, "Unauthorized");
            }
        } else {
            if role_rank(actor_role.as_deref().unwrap_or("")) <= role_rank(&target_role) {
                return error_response(StatusCode::UNAUTHORIZED, "Unauthorized");
            }
        }
    }
    let owner_count = members
        .iter()
        .filter(|row| row.get::<String, _>("role") == OWNER)
        .count();
    if removing_last_owner(&target_role, owner_count) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "A team must have at least one owner.",
        );
    }
    if sqlx::query("DELETE FROM team_user WHERE team_id = $1 AND user_id = $2")
        .bind(team_id)
        .bind(target_id)
        .execute(&mut *tx)
        .await
        .is_err()
        || tx.commit().await.is_err()
    {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error");
    }
    json_response(json!({ "ok": true }))
}

async fn entity_auth<'a>(
    state: &'a AppState,
    headers: &HeaderMap,
    team_id: Uuid,
) -> Result<&'a PgPool, Response> {
    let pool = database(state)?;
    let (user_id, admin) = authenticate(pool, headers, "/api/teams").await?;
    authorize_team(pool, user_id, admin, team_id, TeamPermission::View).await?;
    Ok(pool)
}

async fn handle_team_websites(
    State(state): State<AppState>,
    Path(team_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match entity_auth(&state, &headers, team_id).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let page = match page(&params, &["name", "domain", "createdAt"], true) {
        Ok(page) => page,
        Err(response) => return response,
    };
    let search = page.search.clone().unwrap_or_default();
    let count = match sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)::bigint FROM website
        WHERE team_id = $1 AND deleted_at IS NULL
          AND ($2 = '' OR name ILIKE '%' || $2 || '%' OR domain ILIKE '%' || $2 || '%')
        "#,
    )
    .bind(team_id)
    .bind(&search)
    .fetch_one(pool)
    .await
    {
        Ok(count) => count,
        Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
    };
    let order = match page.order_by.as_deref() {
        Some("name") => "w.name",
        Some("domain") => "w.domain",
        Some("createdAt") => "w.created_at",
        _ => "w.created_at",
    };
    let direction = if page.descending { "DESC" } else { "ASC" };
    let sql = format!(
        r#"
        SELECT w.website_id, w.name, w.domain, w.reset_at, w.user_id, w.team_id, w.created_by,
               w.created_at, w.updated_at, w.recorder_enabled, w.replay_config::text,
               creator.username AS creator_username,
               (SELECT s.slug FROM share s WHERE s.entity_id = w.website_id
                ORDER BY s.created_at DESC LIMIT 1) AS share_id
        FROM website w
        LEFT JOIN "user" creator ON creator.user_id = w.created_by
        WHERE w.team_id = $1 AND w.deleted_at IS NULL
          AND ($2 = '' OR w.name ILIKE '%' || $2 || '%' OR w.domain ILIKE '%' || $2 || '%')
        ORDER BY {order} {direction}
        LIMIT $3 OFFSET $4
        "#
    );
    let rows = match sqlx::query(&sql)
        .bind(team_id)
        .bind(search)
        .bind(page.size)
        .bind(page.offset)
        .fetch_all(pool)
        .await
    {
        Ok(rows) => rows,
        Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
    };
    let data = rows
        .iter()
        .map(|row| {
            let creator_id = row.get::<Option<Uuid>, _>("created_by");
            let creator_name = row.get::<Option<String>, _>("creator_username");
            json!({
                "id": row.get::<Uuid, _>("website_id"),
                "name": row.get::<String, _>("name"),
                "domain": row.get::<Option<String>, _>("domain"),
                "shareId": row.get::<Option<String>, _>("share_id"),
                "resetAt": row.get::<Option<DateTime<Utc>>, _>("reset_at"),
                "userId": row.get::<Option<Uuid>, _>("user_id"),
                "teamId": row.get::<Option<Uuid>, _>("team_id"),
                "createdBy": creator_id,
                "createdAt": row.get::<Option<DateTime<Utc>>, _>("created_at"),
                "updatedAt": row.get::<Option<DateTime<Utc>>, _>("updated_at"),
                "recorderEnabled": row.get::<bool, _>("recorder_enabled"),
                "replayConfig": parse_json_text(row.get::<Option<String>, _>("replay_config")),
                "createUser": match (creator_id, creator_name) {
                    (Some(id), Some(username)) => json!({ "id": id, "username": username }),
                    _ => Value::Null
                }
            })
        })
        .collect();
    json_response(paged_json(data, count, &page))
}

async fn handle_team_boards(
    State(state): State<AppState>,
    Path(team_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match entity_auth(&state, &headers, team_id).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let page = match page(&params, &["name", "description", "type", "createdAt"], true) {
        Ok(page) => page,
        Err(response) => return response,
    };
    let search = page.search.clone().unwrap_or_default();
    let count = match sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)::bigint FROM board
        WHERE team_id = $1 AND type <> 'dashboard'
          AND ($2 = '' OR name ILIKE '%' || $2 || '%' OR description ILIKE '%' || $2 || '%')
        "#,
    )
    .bind(team_id)
    .bind(&search)
    .fetch_one(pool)
    .await
    {
        Ok(count) => count,
        Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
    };
    let order = match page.order_by.as_deref() {
        Some("name") => "name",
        Some("description") => "description",
        Some("type") => "type",
        Some("createdAt") => "created_at",
        _ => "created_at",
    };
    let direction = if page.descending { "DESC" } else { "ASC" };
    let rows = match sqlx::query(&format!(
        r#"
        SELECT board_id, type, name, description, parameters::text, user_id, team_id,
               created_at, updated_at
        FROM board
        WHERE team_id = $1 AND type <> 'dashboard'
          AND ($2 = '' OR name ILIKE '%' || $2 || '%' OR description ILIKE '%' || $2 || '%')
        ORDER BY {order} {direction}
        LIMIT $3 OFFSET $4
        "#
    ))
    .bind(team_id)
    .bind(search)
    .bind(page.size)
    .bind(page.offset)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
    };
    let data = rows
        .iter()
        .map(|row| {
            json!({
                "id": row.get::<Uuid, _>("board_id"),
                "type": row.get::<String, _>("type"),
                "name": row.get::<String, _>("name"),
                "description": row.get::<String, _>("description"),
                "parameters": parse_json_text(row.get::<Option<String>, _>("parameters")),
                "userId": row.get::<Option<Uuid>, _>("user_id"),
                "teamId": row.get::<Option<Uuid>, _>("team_id"),
                "createdAt": row.get::<Option<DateTime<Utc>>, _>("created_at"),
                "updatedAt": row.get::<Option<DateTime<Utc>>, _>("updated_at")
            })
        })
        .collect();
    json_response(paged_json(data, count, &page))
}

async fn handle_team_links(
    State(state): State<AppState>,
    Path(team_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match entity_auth(&state, &headers, team_id).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let page = match page(&params, &["name", "slug", "url", "createdAt"], true) {
        Ok(page) => page,
        Err(response) => return response,
    };
    let search = page.search.clone().unwrap_or_default();
    let count = match sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)::bigint FROM link
        WHERE team_id = $1
          AND ($2 = '' OR name ILIKE '%' || $2 || '%' OR url ILIKE '%' || $2 || '%'
               OR slug ILIKE '%' || $2 || '%')
        "#,
    )
    .bind(team_id)
    .bind(&search)
    .fetch_one(pool)
    .await
    {
        Ok(count) => count,
        Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
    };
    let order = match page.order_by.as_deref() {
        Some("name") => "name",
        Some("slug") => "slug",
        Some("url") => "url",
        Some("createdAt") => "created_at",
        _ => "created_at",
    };
    let direction = if page.descending { "DESC" } else { "ASC" };
    let rows = match sqlx::query(&format!(
        r#"
        SELECT link_id, name, url, slug, user_id, team_id, created_at, updated_at, deleted_at
        FROM link
        WHERE team_id = $1
          AND ($2 = '' OR name ILIKE '%' || $2 || '%' OR url ILIKE '%' || $2 || '%'
               OR slug ILIKE '%' || $2 || '%')
        ORDER BY {order} {direction}
        LIMIT $3 OFFSET $4
        "#
    ))
    .bind(team_id)
    .bind(search)
    .bind(page.size)
    .bind(page.offset)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
    };
    let data = rows
        .iter()
        .map(|row| {
            json!({
                "id": row.get::<Uuid, _>("link_id"),
                "name": row.get::<String, _>("name"),
                "url": row.get::<String, _>("url"),
                "slug": row.get::<String, _>("slug"),
                "userId": row.get::<Option<Uuid>, _>("user_id"),
                "teamId": row.get::<Option<Uuid>, _>("team_id"),
                "createdAt": row.get::<Option<DateTime<Utc>>, _>("created_at"),
                "updatedAt": row.get::<Option<DateTime<Utc>>, _>("updated_at"),
                "deletedAt": row.get::<Option<DateTime<Utc>>, _>("deleted_at")
            })
        })
        .collect();
    json_response(paged_json(data, count, &page))
}

async fn handle_team_pixels(
    State(state): State<AppState>,
    Path(team_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match entity_auth(&state, &headers, team_id).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let page = match page(&params, &["name", "slug", "createdAt"], true) {
        Ok(page) => page,
        Err(response) => return response,
    };
    let search = page.search.clone().unwrap_or_default();
    let count = match sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)::bigint FROM pixel
        WHERE team_id = $1
          AND ($2 = '' OR name ILIKE '%' || $2 || '%' OR slug ILIKE '%' || $2 || '%')
        "#,
    )
    .bind(team_id)
    .bind(&search)
    .fetch_one(pool)
    .await
    {
        Ok(count) => count,
        Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
    };
    let order = match page.order_by.as_deref() {
        Some("name") => "name",
        Some("slug") => "slug",
        Some("createdAt") => "created_at",
        _ => "created_at",
    };
    let direction = if page.descending { "DESC" } else { "ASC" };
    let rows = match sqlx::query(&format!(
        r#"
        SELECT pixel_id, name, slug, user_id, team_id, created_at, updated_at, deleted_at
        FROM pixel
        WHERE team_id = $1
          AND ($2 = '' OR name ILIKE '%' || $2 || '%' OR slug ILIKE '%' || $2 || '%')
        ORDER BY {order} {direction}
        LIMIT $3 OFFSET $4
        "#
    ))
    .bind(team_id)
    .bind(search)
    .bind(page.size)
    .bind(page.offset)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
    };
    let data = rows
        .iter()
        .map(|row| {
            json!({
                "id": row.get::<Uuid, _>("pixel_id"),
                "name": row.get::<String, _>("name"),
                "slug": row.get::<String, _>("slug"),
                "userId": row.get::<Option<Uuid>, _>("user_id"),
                "teamId": row.get::<Option<Uuid>, _>("team_id"),
                "createdAt": row.get::<Option<DateTime<Utc>>, _>("created_at"),
                "updatedAt": row.get::<Option<DateTime<Utc>>, _>("updated_at"),
                "deletedAt": row.get::<Option<DateTime<Utc>>, _>("deleted_at")
            })
        })
        .collect();
    json_response(paged_json(data, count, &page))
}

async fn handle_me_teams(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (user_id, _) = match authenticate(pool, &headers, "/api/me/teams").await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    match list_user_teams(pool, user_id, &params).await {
        Ok(value) => json_response(value),
        Err(response) => response,
    }
}

async fn handle_user_teams(
    State(state): State<AppState>,
    Path(target_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (user_id, admin) = match authenticate(pool, &headers, "/api/users").await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    if user_id != target_id && !admin {
        return error_response(StatusCode::UNAUTHORIZED, "Unauthorized");
    }
    match list_user_teams(pool, target_id, &params).await {
        Ok(value) => json_response(value),
        Err(response) => response,
    }
}

pub(super) fn register_routes(router: Router<AppState>) -> Router<AppState> {
    router
        .route(
            "/api/teams",
            get(handle_list_teams).post(handle_create_team),
        )
        .route("/api/teams/join", post(handle_join_team))
        .route(
            "/api/teams/{team_id}",
            get(handle_get_team)
                .post(handle_update_team)
                .delete(handle_delete_team),
        )
        .route(
            "/api/teams/{team_id}/users",
            get(handle_list_members).post(handle_add_member),
        )
        .route(
            "/api/teams/{team_id}/users/{user_id}",
            get(handle_get_member)
                .post(handle_update_member)
                .delete(handle_delete_member),
        )
        .route("/api/teams/{team_id}/websites", get(handle_team_websites))
        .route("/api/teams/{team_id}/boards", get(handle_team_boards))
        .route("/api/teams/{team_id}/links", get(handle_team_links))
        .route("/api/teams/{team_id}/pixels", get(handle_team_pixels))
        .route("/api/me/teams", get(handle_me_teams))
        .route("/api/users/{user_id}/teams", get(handle_user_teams))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_match_team_permissions() {
        assert!(role_allows(Some(OWNER), TeamPermission::Delete));
        assert!(role_allows(Some(MANAGER), TeamPermission::Update));
        assert!(role_allows(Some(VIEW_ONLY), TeamPermission::View));
        assert!(!role_allows(Some(MEMBER), TeamPermission::Update));
        assert!(!role_allows(Some(MANAGER), TeamPermission::Delete));
    }

    #[test]
    fn access_codes_are_only_listed_for_editors() {
        assert!(exposes_access_code(Some(OWNER)));
        assert!(exposes_access_code(Some(MANAGER)));
        assert!(!exposes_access_code(Some(MEMBER)));
        assert!(!exposes_access_code(Some(VIEW_ONLY)));
    }

    #[test]
    fn last_owner_cannot_be_removed_or_demoted() {
        assert!(removing_last_owner(OWNER, 1));
        assert!(!removing_last_owner(OWNER, 2));
        assert!(!removing_last_owner(MANAGER, 1));
    }
}
