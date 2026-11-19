use super::{check_bearer_auth, error_response, json_response, AppState};
use crate::access::verified_share;
use axum::{
    extract::{rejection::JsonRejection, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use rand::{distributions::Alphanumeric, Rng};
use serde_json::{json, Map, Value};
use sqlx::{postgres::PgRow, PgPool, Row};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

const BOARD_SHARE_TYPE: i32 = 4;
const BOARD_SELECT: &str = r#"
    SELECT board_id, type, name, description, parameters::text AS parameters,
           user_id, team_id, created_at, updated_at
    FROM board
"#;

#[derive(Clone, Debug)]
struct UserAuth {
    id: Uuid,
    role: String,
}

impl UserAuth {
    fn is_admin(&self) -> bool {
        self.role == "admin"
    }
}

#[derive(Clone, Debug)]
struct Board {
    id: Uuid,
    kind: String,
    name: String,
    description: String,
    parameters: Value,
    user_id: Option<Uuid>,
    team_id: Option<Uuid>,
    created_at: Option<DateTime<Utc>>,
    updated_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum EntityKind {
    Website,
    Pixel,
    Link,
}

#[derive(Clone, Copy, Debug)]
enum BoardAccess {
    Read,
    Write,
    Delete,
}

#[derive(Debug)]
struct ListParams {
    page: i64,
    page_size: i64,
    search: Option<String>,
    order_by: Option<&'static str>,
    order_name: Option<String>,
    descending: bool,
}

#[derive(Debug)]
struct CreateBoardInput {
    kind: String,
    name: String,
    description: String,
    parameters: Value,
    team_id: Option<Uuid>,
}

#[derive(Debug)]
struct UpdateBoardInput {
    kind: Option<String>,
    name: Option<String>,
    description: Option<String>,
    parameters: Option<Value>,
}

#[derive(Clone, Debug)]
struct ReportMeta {
    kind: String,
    website_id: Uuid,
}

pub(super) fn register_routes(router: Router<AppState>) -> Router<AppState> {
    router
        .route("/api/boards", get(list_boards).post(create_board))
        .route(
            "/api/boards/{board_id}",
            get(get_board).post(update_board).delete(delete_board),
        )
        .route(
            "/api/boards/{board_id}/clone",
            axum::routing::post(clone_board),
        )
        .route(
            "/api/boards/{board_id}/shares",
            get(list_board_shares).post(create_board_share),
        )
}

fn database(state: &AppState) -> Result<&PgPool, Response> {
    state
        .db
        .as_ref()
        .ok_or_else(|| error_response(StatusCode::SERVICE_UNAVAILABLE, "Database unavailable"))
}

async fn authenticate(pool: &PgPool, headers: &HeaderMap) -> Result<UserAuth, Response> {
    check_bearer_auth(pool, headers, "/api/boards")
        .await
        .map(|(id, role)| UserAuth { id, role })
        .ok_or_else(|| error_response(StatusCode::UNAUTHORIZED, "Unauthorized"))
}

fn share_allows_board(share_type: i32, entity_id: Uuid, board_id: Uuid) -> bool {
    share_type == BOARD_SHARE_TYPE && entity_id == board_id
}

async fn read_credentials(
    pool: &PgPool,
    headers: &HeaderMap,
    board_id: Uuid,
) -> Result<(Option<UserAuth>, bool), Response> {
    let user = check_bearer_auth(pool, headers, "/api/boards")
        .await
        .map(|(id, role)| UserAuth { id, role });
    let shared = verified_share(pool, headers)
        .await?
        .is_some_and(|share| share_allows_board(share.share_type, share.entity_id, board_id));
    if user.is_none() && !shared {
        return Err(unauthorized());
    }
    Ok((user, shared))
}

fn bad_request(message: impl Into<String>) -> Response {
    error_response(StatusCode::BAD_REQUEST, message)
}

fn unauthorized() -> Response {
    error_response(StatusCode::UNAUTHORIZED, "Unauthorized")
}

fn parse_uuid(value: &str) -> Result<Uuid, Response> {
    Uuid::parse_str(value).map_err(|_| bad_request("Invalid resource ID"))
}

fn body_object(body: Result<Json<Value>, JsonRejection>) -> Result<Map<String, Value>, Response> {
    let Json(value) = body.map_err(|_| bad_request("Invalid request"))?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| bad_request("Invalid request"))
}

fn required_string(object: &Map<String, Value>, key: &str, max: usize) -> Result<String, Response> {
    let value = object
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| bad_request("Invalid request"))?;
    if value.chars().count() > max {
        return Err(bad_request("Invalid request"));
    }
    Ok(value.to_string())
}

fn optional_string(
    object: &Map<String, Value>,
    key: &str,
    max: usize,
) -> Result<Option<String>, Response> {
    let Some(value) = object.get(key) else {
        return Ok(None);
    };
    let value = value
        .as_str()
        .ok_or_else(|| bad_request("Invalid request"))?;
    if value.chars().count() > max {
        return Err(bad_request("Invalid request"));
    }
    Ok(Some(value.to_string()))
}

fn optional_uuid(object: &Map<String, Value>, key: &str) -> Result<Option<Uuid>, Response> {
    let Some(value) = object.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_str()
        .and_then(|value| Uuid::parse_str(value).ok())
        .map(Some)
        .ok_or_else(|| bad_request("Invalid request"))
}

fn optional_object(object: &Map<String, Value>, key: &str) -> Result<Option<Value>, Response> {
    let Some(value) = object.get(key) else {
        return Ok(None);
    };
    if !value.is_object() {
        return Err(bad_request("Invalid request"));
    }
    Ok(Some(value.clone()))
}

fn normalize_board_type(value: &str, dashboard: bool) -> Option<&'static str> {
    match value {
        "open" => Some("mixed"),
        "mixed" => Some("mixed"),
        "website" => Some("website"),
        "pixel" => Some("pixel"),
        "link" => Some("link"),
        "dashboard" if dashboard => Some("dashboard"),
        _ => None,
    }
}

fn validate_parameter_ids(parameters: &Value) -> Result<(), Response> {
    let Some(parameters) = parameters.as_object() else {
        return Err(bad_request("Invalid request"));
    };
    for key in ["websiteId", "pixelId", "linkId"] {
        if let Some(value) = parameters.get(key) {
            if value
                .as_str()
                .and_then(|value| Uuid::parse_str(value).ok())
                .is_none()
            {
                return Err(bad_request("Invalid request"));
            }
        }
    }
    Ok(())
}

fn parse_create_input(
    body: Result<Json<Value>, JsonRejection>,
) -> Result<CreateBoardInput, Response> {
    let object = body_object(body)?;
    let kind = required_string(&object, "type", 50)?;
    let kind = normalize_board_type(&kind, false)
        .ok_or_else(|| bad_request("Invalid request"))?
        .to_string();
    let name = required_string(&object, "name", 100)?;
    let description = optional_string(&object, "description", 500)?.unwrap_or_default();
    let team_id = optional_uuid(&object, "teamId")?;
    let _ = optional_uuid(&object, "userId")?;
    let parameters = optional_object(&object, "parameters")?.unwrap_or_else(|| json!({}));
    validate_parameter_ids(&parameters)?;
    Ok(CreateBoardInput {
        kind,
        name,
        description,
        parameters,
        team_id,
    })
}

fn parse_update_input(
    body: Result<Json<Value>, JsonRejection>,
) -> Result<UpdateBoardInput, Response> {
    let object = body_object(body)?;
    let kind = optional_string(&object, "type", 50)?
        .map(|value| {
            normalize_board_type(&value, true)
                .map(str::to_string)
                .ok_or_else(|| bad_request("Invalid request"))
        })
        .transpose()?;
    Ok(UpdateBoardInput {
        kind,
        name: optional_string(&object, "name", 200)?,
        description: optional_string(&object, "description", 500)?,
        parameters: optional_object(&object, "parameters")?,
    })
}

fn parse_clone_input(
    body: Result<Json<Value>, JsonRejection>,
) -> Result<UpdateBoardInput, Response> {
    let object = body_object(body)?;
    Ok(UpdateBoardInput {
        kind: None,
        name: optional_string(&object, "name", 200)?,
        description: optional_string(&object, "description", 500)?,
        parameters: optional_object(&object, "parameters")?,
    })
}

fn board_from_row(row: &PgRow) -> Board {
    Board {
        id: row.get("board_id"),
        kind: row.get("type"),
        name: row.get("name"),
        description: row.get("description"),
        parameters: row
            .get::<String, _>("parameters")
            .parse()
            .unwrap_or_else(|_| json!({})),
        user_id: row.get("user_id"),
        team_id: row.get("team_id"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn board_json(board: &Board) -> Value {
    json!({
        "id": board.id,
        "type": board.kind,
        "name": board.name,
        "description": board.description,
        "parameters": board.parameters,
        "userId": board.user_id,
        "teamId": board.team_id,
        "createdAt": board.created_at,
        "updatedAt": board.updated_at,
    })
}

fn share_json(row: &PgRow) -> Value {
    json!({
        "id": row.get::<Uuid, _>("share_id"),
        "entityId": row.get::<Uuid, _>("entity_id"),
        "name": row.get::<String, _>("name"),
        "shareType": row.get::<i32, _>("share_type"),
        "slug": row.get::<String, _>("slug"),
        "parameters": row
            .get::<String, _>("parameters")
            .parse::<Value>()
            .unwrap_or_else(|_| json!({})),
        "createdAt": row.get::<Option<DateTime<Utc>>, _>("created_at"),
        "updatedAt": row.get::<Option<DateTime<Utc>>, _>("updated_at"),
    })
}

async fn load_board(pool: &PgPool, board_id: Uuid) -> Result<Option<Board>, sqlx::Error> {
    sqlx::query(&format!("{BOARD_SELECT} WHERE board_id = $1"))
        .bind(board_id)
        .fetch_optional(pool)
        .await
        .map(|row| row.as_ref().map(board_from_row))
}

fn team_role_allows(role: &str, access: BoardAccess) -> bool {
    match access {
        BoardAccess::Read => matches!(
            role,
            "team-owner" | "team-manager" | "team-member" | "team-view-only"
        ),
        BoardAccess::Write | BoardAccess::Delete => {
            matches!(role, "team-owner" | "team-manager" | "team-member")
        }
    }
}

async fn team_role(
    pool: &PgPool,
    team_id: Uuid,
    user_id: Uuid,
) -> Result<Option<String>, sqlx::Error> {
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
}

async fn board_access_allowed(
    pool: &PgPool,
    user: &UserAuth,
    board: &Board,
    access: BoardAccess,
) -> Result<bool, sqlx::Error> {
    if user.is_admin() {
        return Ok(true);
    }
    if board.user_id.is_some() {
        return Ok(board.user_id == Some(user.id));
    }
    let Some(team_id) = board.team_id else {
        return Ok(false);
    };
    Ok(team_role(pool, team_id, user.id)
        .await?
        .is_some_and(|role| team_role_allows(&role, access)))
}

async fn can_create_board(
    pool: &PgPool,
    user: &UserAuth,
    team_id: Option<Uuid>,
) -> Result<bool, sqlx::Error> {
    if user.is_admin() {
        return Ok(true);
    }
    if user.role != "user" {
        return Ok(false);
    }
    let Some(team_id) = team_id else {
        return Ok(true);
    };
    Ok(team_role(pool, team_id, user.id)
        .await?
        .is_some_and(|role| team_role_allows(&role, BoardAccess::Write)))
}

fn inferred_board_type(kind: &str, parameters: &Map<String, Value>) -> &'static str {
    if let Some(kind) = normalize_board_type(kind, true) {
        return kind;
    }
    if parameters.get("pixelId").is_some() {
        "pixel"
    } else if parameters.get("linkId").is_some() {
        "link"
    } else if parameters.get("websiteId").is_some() {
        "website"
    } else {
        "mixed"
    }
}

fn value_uuid(value: Option<&Value>) -> Result<Option<Uuid>, ()> {
    match value {
        None => Ok(None),
        Some(value) => value
            .as_str()
            .and_then(|value| Uuid::parse_str(value).ok())
            .map(Some)
            .ok_or(()),
    }
}

fn board_entity(
    kind: &str,
    parameters: &Map<String, Value>,
) -> Result<Option<(EntityKind, Uuid)>, ()> {
    match inferred_board_type(kind, parameters) {
        "website" => {
            value_uuid(parameters.get("websiteId")).map(|id| id.map(|id| (EntityKind::Website, id)))
        }
        "pixel" => {
            value_uuid(parameters.get("pixelId")).map(|id| id.map(|id| (EntityKind::Pixel, id)))
        }
        "link" => {
            value_uuid(parameters.get("linkId")).map(|id| id.map(|id| (EntityKind::Link, id)))
        }
        _ => Ok(None),
    }
}

fn component_entity(component: &Map<String, Value>) -> Result<Option<(EntityKind, Uuid)>, ()> {
    if let (Some(entity_type), Some(entity_id)) = (
        component.get("entityType").and_then(Value::as_str),
        component.get("entityId"),
    ) {
        let kind = match entity_type {
            "website" => EntityKind::Website,
            "pixel" => EntityKind::Pixel,
            "link" => EntityKind::Link,
            _ => return Ok(None),
        };
        return value_uuid(Some(entity_id)).map(|id| id.map(|id| (kind, id)));
    }
    value_uuid(component.get("websiteId")).map(|id| id.map(|id| (EntityKind::Website, id)))
}

fn collect_entity_ids(kind: &str, parameters: &Value) -> Result<Vec<(EntityKind, Uuid)>, ()> {
    let parameters = parameters.as_object().ok_or(())?;
    let mut ids = HashSet::new();
    if let Some(entity) = board_entity(kind, parameters)? {
        ids.insert(entity);
    }
    for (key, entity_kind) in [
        ("websiteId", EntityKind::Website),
        ("pixelId", EntityKind::Pixel),
        ("linkId", EntityKind::Link),
    ] {
        if let Some(id) = value_uuid(parameters.get(key))? {
            ids.insert((entity_kind, id));
        }
    }
    if let Some(rows) = parameters.get("rows") {
        let rows = rows.as_array().ok_or(())?;
        for row in rows {
            let Some(row) = row.as_object() else {
                return Err(());
            };
            let Some(columns) = row.get("columns") else {
                continue;
            };
            let columns = columns.as_array().ok_or(())?;
            for column in columns {
                let Some(component) = column
                    .as_object()
                    .and_then(|column| column.get("component"))
                    .and_then(Value::as_object)
                else {
                    continue;
                };
                if let Some(entity) = component_entity(component)? {
                    ids.insert(entity);
                }
            }
        }
    }
    Ok(ids.into_iter().collect())
}

async fn entity_access_allowed(
    pool: &PgPool,
    user: &UserAuth,
    kind: EntityKind,
    entity_id: Uuid,
) -> Result<bool, sqlx::Error> {
    if user.is_admin() {
        return Ok(true);
    }
    let (table, id_column) = match kind {
        EntityKind::Website => ("website", "website_id"),
        EntityKind::Pixel => ("pixel", "pixel_id"),
        EntityKind::Link => ("link", "link_id"),
    };
    let query = format!(
        r#"
        SELECT e.user_id, tu.role AS team_role
        FROM {table} e
        LEFT JOIN team t ON t.team_id = e.team_id AND t.deleted_at IS NULL
        LEFT JOIN team_user tu ON tu.team_id = t.team_id AND tu.user_id = $2
        WHERE e.{id_column} = $1 AND e.deleted_at IS NULL
        "#
    );
    let row = sqlx::query(&query)
        .bind(entity_id)
        .bind(user.id)
        .fetch_optional(pool)
        .await?;
    Ok(row.is_some_and(|row| {
        row.get::<Option<Uuid>, _>("user_id") == Some(user.id)
            || row.get::<Option<String>, _>("team_role").is_some()
    }))
}

async fn board_entities_allowed(
    pool: &PgPool,
    user: &UserAuth,
    kind: &str,
    parameters: &Value,
) -> Result<bool, sqlx::Error> {
    let ids = match collect_entity_ids(kind, parameters) {
        Ok(ids) => ids,
        Err(_) => return Ok(false),
    };
    for (entity_kind, entity_id) in ids {
        if !entity_access_allowed(pool, user, entity_kind, entity_id).await? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn expected_report_type(component: &Map<String, Value>) -> Option<&'static str> {
    match component.get("type").and_then(Value::as_str) {
        Some("Goal") => Some("goal"),
        Some("Funnel") => Some("funnel"),
        _ => None,
    }
}

fn report_id(component: &Map<String, Value>) -> Result<Option<Uuid>, ()> {
    let Some(report_id) = component
        .get("props")
        .and_then(Value::as_object)
        .and_then(|props| props.get("reportId"))
    else {
        return Ok(None);
    };
    if report_id.is_null() || report_id.as_str() == Some("") {
        return Ok(None);
    }
    report_id
        .as_str()
        .and_then(|value| Uuid::parse_str(value).ok())
        .map(Some)
        .ok_or(())
}

async fn report_meta(
    pool: &PgPool,
    report_id: Uuid,
    cache: &mut HashMap<Uuid, Option<ReportMeta>>,
) -> Result<Option<ReportMeta>, sqlx::Error> {
    if let Some(report) = cache.get(&report_id) {
        return Ok(report.clone());
    }
    let report = sqlx::query("SELECT type, website_id FROM report WHERE report_id = $1")
        .bind(report_id)
        .fetch_optional(pool)
        .await?
        .map(|row| ReportMeta {
            kind: row.get("type"),
            website_id: row.get("website_id"),
        });
    cache.insert(report_id, report.clone());
    Ok(report)
}

async fn report_reference_valid(
    pool: &PgPool,
    board_entity: Option<(EntityKind, Uuid)>,
    component: &Map<String, Value>,
    cache: &mut HashMap<Uuid, Option<ReportMeta>>,
) -> Result<bool, sqlx::Error> {
    let Some(expected) = expected_report_type(component) else {
        return Ok(true);
    };
    let report_id = match report_id(component) {
        Ok(Some(report_id)) => report_id,
        Ok(None) => return Ok(true),
        Err(_) => return Ok(false),
    };
    let entity = match board_entity {
        Some(entity) => Some(entity),
        None => component_entity(component).ok().flatten(),
    };
    let Some((EntityKind::Website, website_id)) = entity else {
        return Ok(false);
    };
    Ok(report_meta(pool, report_id, cache)
        .await?
        .is_some_and(|report| report.kind == expected && report.website_id == website_id))
}

async fn board_reports_valid(
    pool: &PgPool,
    kind: &str,
    parameters: &Value,
) -> Result<bool, sqlx::Error> {
    let Some(parameters) = parameters.as_object() else {
        return Ok(false);
    };
    let board_entity = match board_entity(kind, parameters) {
        Ok(entity) => entity,
        Err(_) => return Ok(false),
    };
    let Some(rows) = parameters.get("rows") else {
        return Ok(true);
    };
    let Some(rows) = rows.as_array() else {
        return Ok(false);
    };
    let mut cache = HashMap::new();
    for row in rows {
        let Some(row) = row.as_object() else {
            return Ok(false);
        };
        let Some(columns) = row.get("columns") else {
            continue;
        };
        let Some(columns) = columns.as_array() else {
            return Ok(false);
        };
        for column in columns {
            let Some(component) = column
                .as_object()
                .and_then(|column| column.get("component"))
                .and_then(Value::as_object)
            else {
                continue;
            };
            if !report_reference_valid(pool, board_entity, component, &mut cache).await? {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

pub(super) async fn dashboard_reports_valid(
    pool: &PgPool,
    parameters: &Value,
) -> Result<bool, sqlx::Error> {
    board_reports_valid(pool, "dashboard", parameters).await
}

fn remove_report_id(component: &mut Map<String, Value>) {
    let remove_props = component
        .get_mut("props")
        .and_then(Value::as_object_mut)
        .map(|props| {
            props.remove("reportId");
            props.is_empty()
        })
        .unwrap_or(false);
    if remove_props {
        component.remove("props");
    }
}

async fn strip_invalid_reports(
    pool: &PgPool,
    kind: &str,
    parameters: &mut Value,
) -> Result<(), sqlx::Error> {
    let Some(parameters_object) = parameters.as_object() else {
        return Ok(());
    };
    let board_entity = board_entity(kind, parameters_object).ok().flatten();
    let mut cache = HashMap::new();
    let Some(rows) = parameters
        .as_object_mut()
        .and_then(|parameters| parameters.get_mut("rows"))
        .and_then(Value::as_array_mut)
    else {
        return Ok(());
    };
    for row in rows {
        let Some(columns) = row
            .as_object_mut()
            .and_then(|row| row.get_mut("columns"))
            .and_then(Value::as_array_mut)
        else {
            continue;
        };
        for column in columns {
            let Some(component) = column
                .as_object_mut()
                .and_then(|column| column.get_mut("component"))
                .and_then(Value::as_object_mut)
            else {
                continue;
            };
            if !report_reference_valid(pool, board_entity, component, &mut cache).await? {
                remove_report_id(component);
            }
        }
    }
    Ok(())
}

fn parse_positive(
    params: &HashMap<String, String>,
    key: &str,
    default: i64,
) -> Result<i64, Response> {
    match params.get(key) {
        None => Ok(default),
        Some(value) => value
            .parse::<i64>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| bad_request("Invalid query")),
    }
}

fn parse_list_params(params: &HashMap<String, String>) -> Result<ListParams, Response> {
    let page = parse_positive(params, "page", 1)?;
    let page_size = parse_positive(params, "pageSize", 20)?;
    let _ = parse_positive(params, "maxResults", 1)?;
    let descending = match params.get("sortDescending").map(String::as_str) {
        None | Some("false") => false,
        Some("true") => true,
        Some(_) => return Err(bad_request("Invalid query")),
    };
    let (order_by, order_name) = match params.get("orderBy").map(String::as_str) {
        Some("name") => (Some("name"), Some("name".to_string())),
        Some("description") => (Some("description"), Some("description".to_string())),
        Some("type") => (Some("type"), Some("type".to_string())),
        Some("createdAt") => (Some("created_at"), Some("createdAt".to_string())),
        _ => (None, None),
    };
    Ok(ListParams {
        page,
        page_size,
        search: params.get("search").cloned(),
        order_by,
        order_name,
        descending: order_by.is_some() && descending,
    })
}

fn search_pattern(search: Option<&str>) -> Option<String> {
    search.filter(|value| !value.is_empty()).map(|value| {
        let escaped = value
            .replace('\\', r"\\")
            .replace('%', r"\%")
            .replace('_', r"\_");
        format!("%{escaped}%")
    })
}

async fn list_boards(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let user = match authenticate(pool, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let params = match parse_list_params(&params) {
        Ok(params) => params,
        Err(response) => return response,
    };
    let search = search_pattern(params.search.as_deref());
    let order = params.order_by.unwrap_or("created_at");
    let direction = if params.descending { "DESC" } else { "ASC" };
    let offset = (params.page - 1).saturating_mul(params.page_size);
    let query = format!(
        r#"
        {BOARD_SELECT}
        WHERE user_id = $1
          AND type <> 'dashboard'
          AND ($2::text IS NULL OR name ILIKE $2 ESCAPE '\' OR description ILIKE $2 ESCAPE '\')
        ORDER BY {order} {direction}
        LIMIT $3 OFFSET $4
        "#
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
                format!("Unable to list boards: {error}"),
            )
        }
    };
    let count = match sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)::bigint
        FROM board
        WHERE user_id = $1
          AND type <> 'dashboard'
          AND ($2::text IS NULL OR name ILIKE $2 ESCAPE '\' OR description ILIKE $2 ESCAPE '\')
        "#,
    )
    .bind(user.id)
    .bind(&search)
    .fetch_one(pool)
    .await
    {
        Ok(count) => count,
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to count boards: {error}"),
            )
        }
    };
    let mut result = Map::from_iter([
        (
            "data".to_string(),
            Value::Array(
                rows.iter()
                    .map(board_from_row)
                    .map(|board| board_json(&board))
                    .collect(),
            ),
        ),
        ("count".to_string(), json!(count)),
        ("page".to_string(), json!(params.page)),
        ("pageSize".to_string(), json!(params.page_size)),
    ]);
    if let Some(order_by) = params.order_name {
        result.insert("orderBy".to_string(), json!(order_by));
    }
    if let Some(search) = params.search {
        result.insert("search".to_string(), json!(search));
    }
    json_response(Value::Object(result))
}

async fn create_board(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<Value>, JsonRejection>,
) -> Response {
    let input = match parse_create_input(body) {
        Ok(input) => input,
        Err(response) => return response,
    };
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let user = match authenticate(pool, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match can_create_board(pool, &user, input.team_id).await {
        Ok(true) => {}
        Ok(false) => return unauthorized(),
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to verify board create access: {error}"),
            )
        }
    }
    match board_entities_allowed(pool, &user, &input.kind, &input.parameters).await {
        Ok(true) => {}
        Ok(false) => return bad_request("Board contains inaccessible entities."),
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to verify board entities: {error}"),
            )
        }
    }
    match board_reports_valid(pool, &input.kind, &input.parameters).await {
        Ok(true) => {}
        Ok(false) => return bad_request("Board contains invalid saved reports."),
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to validate board reports: {error}"),
            )
        }
    }
    let board_id = Uuid::new_v4();
    let user_id = input.team_id.is_none().then_some(user.id);
    let row = match sqlx::query(
        r#"
        INSERT INTO board
            (board_id, type, name, description, parameters, user_id, team_id,
             created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5::jsonb, $6, $7, NOW(), NOW())
        RETURNING board_id, type, name, description, parameters::text AS parameters,
                  user_id, team_id, created_at, updated_at
        "#,
    )
    .bind(board_id)
    .bind(input.kind)
    .bind(input.name)
    .bind(input.description)
    .bind(input.parameters.to_string())
    .bind(user_id)
    .bind(input.team_id)
    .fetch_one(pool)
    .await
    {
        Ok(row) => row,
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to create board: {error}"),
            )
        }
    };
    json_response(board_json(&board_from_row(&row)))
}

async fn get_board(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(board_id): Path<String>,
) -> Response {
    let board_id = match parse_uuid(&board_id) {
        Ok(board_id) => board_id,
        Err(response) => return response,
    };
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (user, shared) = match read_credentials(pool, &headers, board_id).await {
        Ok(credentials) => credentials,
        Err(response) => return response,
    };
    let board = match load_board(pool, board_id).await {
        Ok(board) => board,
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to load board: {error}"),
            )
        }
    };
    let Some(board) = board else {
        return if user.as_ref().is_some_and(UserAuth::is_admin) {
            json_response(Value::Null)
        } else {
            unauthorized()
        };
    };
    if shared {
        return json_response(board_json(&board));
    }
    let Some(user) = user else {
        return unauthorized();
    };
    match board_access_allowed(pool, &user, &board, BoardAccess::Read).await {
        Ok(true) => json_response(board_json(&board)),
        Ok(false) => unauthorized(),
        Err(error) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to verify board access: {error}"),
        ),
    }
}

async fn update_board(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(board_id): Path<String>,
    body: Result<Json<Value>, JsonRejection>,
) -> Response {
    let board_id = match parse_uuid(&board_id) {
        Ok(board_id) => board_id,
        Err(response) => return response,
    };
    let input = match parse_update_input(body) {
        Ok(input) => input,
        Err(response) => return response,
    };
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let user = match authenticate(pool, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let current = match load_board(pool, board_id).await {
        Ok(Some(board)) => board,
        Ok(None) => return unauthorized(),
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to load board: {error}"),
            )
        }
    };
    match board_access_allowed(pool, &user, &current, BoardAccess::Write).await {
        Ok(true) => {}
        Ok(false) => return unauthorized(),
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to verify board update access: {error}"),
            )
        }
    }
    if input.kind.is_some() || input.parameters.is_some() {
        let kind = input.kind.as_deref().unwrap_or(&current.kind);
        let parameters = input.parameters.as_ref().unwrap_or(&current.parameters);
        match board_entities_allowed(pool, &user, kind, parameters).await {
            Ok(true) => {}
            Ok(false) => return bad_request("Board contains inaccessible entities."),
            Err(error) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Unable to verify board entities: {error}"),
                )
            }
        }
        match board_reports_valid(pool, kind, parameters).await {
            Ok(true) => {}
            Ok(false) => return bad_request("Board contains invalid saved reports."),
            Err(error) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Unable to validate board reports: {error}"),
                )
            }
        }
    }
    let parameters = input.parameters.as_ref().map(Value::to_string);
    let row = match sqlx::query(
        r#"
        UPDATE board SET
            type = CASE WHEN $2 THEN $3 ELSE type END,
            name = CASE WHEN $4 THEN $5 ELSE name END,
            description = CASE WHEN $6 THEN $7 ELSE description END,
            parameters = CASE WHEN $8 THEN $9::jsonb ELSE parameters END,
            updated_at = NOW()
        WHERE board_id = $1
        RETURNING board_id, type, name, description, parameters::text AS parameters,
                  user_id, team_id, created_at, updated_at
        "#,
    )
    .bind(board_id)
    .bind(input.kind.is_some())
    .bind(input.kind)
    .bind(input.name.is_some())
    .bind(input.name)
    .bind(input.description.is_some())
    .bind(input.description)
    .bind(parameters.is_some())
    .bind(parameters)
    .fetch_one(pool)
    .await
    {
        Ok(row) => row,
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to update board: {error}"),
            )
        }
    };
    json_response(board_json(&board_from_row(&row)))
}

async fn delete_board(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(board_id): Path<String>,
) -> Response {
    let board_id = match parse_uuid(&board_id) {
        Ok(board_id) => board_id,
        Err(response) => return response,
    };
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let user = match authenticate(pool, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let board = match load_board(pool, board_id).await {
        Ok(Some(board)) => board,
        Ok(None) => return unauthorized(),
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to load board: {error}"),
            )
        }
    };
    match board_access_allowed(pool, &user, &board, BoardAccess::Delete).await {
        Ok(true) => {}
        Ok(false) => return unauthorized(),
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to verify board delete access: {error}"),
            )
        }
    }
    match sqlx::query("DELETE FROM board WHERE board_id = $1")
        .bind(board_id)
        .execute(pool)
        .await
    {
        Ok(result) if result.rows_affected() == 1 => json_response(json!({ "ok": true })),
        Ok(_) => unauthorized(),
        Err(error) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to delete board: {error}"),
        ),
    }
}

async fn clone_board(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(board_id): Path<String>,
    body: Result<Json<Value>, JsonRejection>,
) -> Response {
    let board_id = match parse_uuid(&board_id) {
        Ok(board_id) => board_id,
        Err(response) => return response,
    };
    let input = match parse_clone_input(body) {
        Ok(input) => input,
        Err(response) => return response,
    };
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let user = match authenticate(pool, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let source = match load_board(pool, board_id).await {
        Ok(Some(board)) => board,
        Ok(None) => return unauthorized(),
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to load board: {error}"),
            )
        }
    };
    match board_access_allowed(pool, &user, &source, BoardAccess::Write).await {
        Ok(true) => {}
        Ok(false) => return unauthorized(),
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to verify board clone access: {error}"),
            )
        }
    }
    let Some(kind) = normalize_board_type(&source.kind, false) else {
        return bad_request("Board cannot be cloned.");
    };
    match can_create_board(pool, &user, source.team_id).await {
        Ok(true) => {}
        Ok(false) => return unauthorized(),
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to verify board create access: {error}"),
            )
        }
    }
    let mut parameters = input
        .parameters
        .unwrap_or_else(|| source.parameters.clone());
    match board_entities_allowed(pool, &user, kind, &parameters).await {
        Ok(true) => {}
        Ok(false) => return bad_request("Board contains inaccessible entities."),
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to verify board entities: {error}"),
            )
        }
    }
    if let Err(error) = strip_invalid_reports(pool, kind, &mut parameters).await {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to sanitize board reports: {error}"),
        );
    }
    match board_reports_valid(pool, kind, &parameters).await {
        Ok(true) => {}
        Ok(false) => return bad_request("Board contains invalid saved reports."),
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to validate board reports: {error}"),
            )
        }
    }
    let clone_id = Uuid::new_v4();
    let user_id = if source.team_id.is_some() {
        None
    } else {
        source.user_id.or(Some(user.id))
    };
    let row = match sqlx::query(
        r#"
        INSERT INTO board
            (board_id, type, name, description, parameters, user_id, team_id,
             created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5::jsonb, $6, $7, NOW(), NOW())
        RETURNING board_id, type, name, description, parameters::text AS parameters,
                  user_id, team_id, created_at, updated_at
        "#,
    )
    .bind(clone_id)
    .bind(kind)
    .bind(input.name.unwrap_or(source.name))
    .bind(input.description.unwrap_or(source.description))
    .bind(parameters.to_string())
    .bind(user_id)
    .bind(source.team_id)
    .fetch_one(pool)
    .await
    {
        Ok(row) => row,
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to clone board: {error}"),
            )
        }
    };
    json_response(board_json(&board_from_row(&row)))
}

fn validate_share_query(params: &HashMap<String, String>) -> Result<(i64, i64), Response> {
    let page = parse_positive(params, "page", 1)?;
    let page_size = parse_positive(params, "pageSize", 20)?;
    let _ = parse_positive(params, "maxResults", 1)?;
    if params.get("eventType").is_some_and(|value| {
        value
            .parse::<i64>()
            .ok()
            .filter(|value| *value > 0)
            .is_none()
    }) {
        return Err(bad_request("Invalid query"));
    }
    if params
        .get("match")
        .is_some_and(|value| !matches!(value.as_str(), "all" | "any"))
    {
        return Err(bad_request("Invalid query"));
    }
    for key in ["segment", "cohort"] {
        if params
            .get(key)
            .is_some_and(|value| Uuid::parse_str(value).is_err())
        {
            return Err(bad_request("Invalid query"));
        }
    }
    Ok((page, page_size))
}

async fn list_board_shares(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
    Path(board_id): Path<String>,
) -> Response {
    let board_id = match parse_uuid(&board_id) {
        Ok(board_id) => board_id,
        Err(response) => return response,
    };
    let (page, page_size) = match validate_share_query(&params) {
        Ok(page) => page,
        Err(response) => return response,
    };
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (user, shared) = match read_credentials(pool, &headers, board_id).await {
        Ok(credentials) => credentials,
        Err(response) => return response,
    };
    if !shared && !user.as_ref().is_some_and(UserAuth::is_admin) {
        let Some(user) = user.as_ref() else {
            return unauthorized();
        };
        let board = match load_board(pool, board_id).await {
            Ok(Some(board)) => board,
            Ok(None) => return unauthorized(),
            Err(error) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Unable to load board: {error}"),
                )
            }
        };
        match board_access_allowed(pool, user, &board, BoardAccess::Read).await {
            Ok(true) => {}
            Ok(false) => return unauthorized(),
            Err(error) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Unable to verify board access: {error}"),
                )
            }
        }
    }
    let offset = (page - 1).saturating_mul(page_size);
    let rows = match sqlx::query(
        r#"
        SELECT share_id, entity_id, name, share_type, slug,
               parameters::text AS parameters, created_at, updated_at
        FROM share
        WHERE entity_id = $1
        ORDER BY created_at DESC
        LIMIT $2 OFFSET $3
        "#,
    )
    .bind(board_id)
    .bind(page_size)
    .bind(offset)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to list board shares: {error}"),
            )
        }
    };
    let count = match sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)::bigint FROM share WHERE entity_id = $1",
    )
    .bind(board_id)
    .fetch_one(pool)
    .await
    {
        Ok(count) => count,
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to count board shares: {error}"),
            )
        }
    };
    json_response(json!({
        "data": rows.iter().map(share_json).collect::<Vec<_>>(),
        "count": count,
        "page": page,
        "pageSize": page_size,
    }))
}

async fn create_board_share(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(board_id): Path<String>,
    body: Result<Json<Value>, JsonRejection>,
) -> Response {
    let board_id = match parse_uuid(&board_id) {
        Ok(board_id) => board_id,
        Err(response) => return response,
    };
    let object = match body_object(body) {
        Ok(object) => object,
        Err(response) => return response,
    };
    let name = match required_string(&object, "name", 200) {
        Ok(name) => name,
        Err(response) => return response,
    };
    let parameters = match optional_object(&object, "parameters") {
        Ok(parameters) => parameters.unwrap_or_else(|| json!({})),
        Err(response) => return response,
    };
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let user = match authenticate(pool, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    if !user.is_admin() {
        let board = match load_board(pool, board_id).await {
            Ok(Some(board)) => board,
            Ok(None) => return unauthorized(),
            Err(error) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Unable to load board: {error}"),
                )
            }
        };
        match board_access_allowed(pool, &user, &board, BoardAccess::Write).await {
            Ok(true) => {}
            Ok(false) => return unauthorized(),
            Err(error) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Unable to verify board share access: {error}"),
                )
            }
        }
    }
    let share_id = Uuid::new_v4();
    let slug: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(16)
        .map(char::from)
        .collect();
    let row = match sqlx::query(
        r#"
        INSERT INTO share
            (share_id, entity_id, name, share_type, slug, parameters, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, $6::jsonb, NOW(), NOW())
        RETURNING share_id, entity_id, name, share_type, slug,
                  parameters::text AS parameters, created_at, updated_at
        "#,
    )
    .bind(share_id)
    .bind(board_id)
    .bind(name)
    .bind(BOARD_SHARE_TYPE)
    .bind(slug)
    .bind(parameters.to_string())
    .fetch_one(pool)
    .await
    {
        Ok(row) => row,
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to create board share: {error}"),
            )
        }
    };
    json_response(share_json(&row))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_only_supported_board_types() {
        assert_eq!(normalize_board_type("open", false), Some("mixed"));
        assert_eq!(normalize_board_type("website", false), Some("website"));
        assert_eq!(normalize_board_type("dashboard", false), None);
        assert_eq!(normalize_board_type("dashboard", true), Some("dashboard"));
        assert_eq!(normalize_board_type("other", true), None);
    }

    #[test]
    fn team_roles_match_node_board_permissions() {
        assert!(team_role_allows("team-view-only", BoardAccess::Read));
        assert!(!team_role_allows("team-view-only", BoardAccess::Write));
        assert!(team_role_allows("team-member", BoardAccess::Write));
        assert!(team_role_allows("team-manager", BoardAccess::Delete));
        assert!(!team_role_allows("unknown", BoardAccess::Read));
    }

    #[test]
    fn board_shares_are_scoped_to_the_exact_board() {
        let board_id = Uuid::new_v4();
        assert!(share_allows_board(BOARD_SHARE_TYPE, board_id, board_id));
        assert!(!share_allows_board(
            BOARD_SHARE_TYPE,
            Uuid::new_v4(),
            board_id
        ));
        assert!(!share_allows_board(1, board_id, board_id));
    }

    #[test]
    fn collects_top_level_and_component_entities() {
        let website_id = Uuid::new_v4();
        let pixel_id = Uuid::new_v4();
        let link_id = Uuid::new_v4();
        let parameters = json!({
            "websiteId": website_id,
            "rows": [{
                "columns": [
                    { "component": { "entityType": "pixel", "entityId": pixel_id } },
                    { "component": { "entityType": "link", "entityId": link_id } },
                    { "component": { "websiteId": website_id } }
                ]
            }]
        });
        let ids = collect_entity_ids("website", &parameters)
            .unwrap()
            .into_iter()
            .collect::<HashSet<_>>();
        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&(EntityKind::Website, website_id)));
        assert!(ids.contains(&(EntityKind::Pixel, pixel_id)));
        assert!(ids.contains(&(EntityKind::Link, link_id)));
    }

    #[test]
    fn invalid_sort_boolean_is_rejected() {
        let params = HashMap::from([("sortDescending".to_string(), "yes".to_string())]);
        assert!(parse_list_params(&params).is_err());
    }

    #[test]
    fn stripping_report_id_preserves_other_component_props() {
        let mut component = json!({
            "type": "Goal",
            "title": "Saved goal",
            "props": {
                "reportId": Uuid::new_v4(),
                "customValue": "keep"
            }
        })
        .as_object()
        .unwrap()
        .clone();
        remove_report_id(&mut component);
        assert_eq!(
            component.get("props"),
            Some(&json!({ "customValue": "keep" }))
        );
        assert_eq!(component.get("title"), Some(&json!("Saved goal")));
    }
}
