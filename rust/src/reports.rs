use crate::insights::{report_filter_params, scalar_filter_predicate, ScalarFilterPredicate};
use crate::{
    access::{website_pool, website_section_pool, Access, ShareDataVisibility, ShareSection},
    audience::Audience,
    check_bearer_auth, error_response, json_response, status_response, AppState,
};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sqlx::{PgPool, Postgres, QueryBuilder, Row};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq)]
enum ReportKind {
    Goal,
    Funnel,
}

impl ReportKind {
    fn name(self) -> &'static str {
        match self {
            Self::Goal => "goal",
            Self::Funnel => "funnel",
        }
    }

    fn section(self) -> ShareSection {
        match self {
            Self::Goal => ShareSection::Goals,
            Self::Funnel => ShareSection::Funnels,
        }
    }
}

#[derive(Debug)]
struct Report {
    id: Uuid,
    user_id: Uuid,
    website_id: Uuid,
    kind: String,
    name: String,
    description: String,
    parameters: Value,
    created_at: Option<DateTime<Utc>>,
    updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug)]
struct Definition {
    name: String,
    description: Option<String>,
    parameters: Value,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct GoalParameters {
    #[serde(rename = "type")]
    action_type: String,
    value: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct FunnelParameters {
    window: f64,
    steps: Vec<FunnelStep>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct FunnelStep {
    #[serde(rename = "type")]
    action_type: String,
    value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    filters: Option<Vec<FunnelStepFilter>>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct FunnelStepFilter {
    property: String,
    operator: String,
    value: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct DateRange {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
}

pub(super) fn register_routes(router: Router<AppState>) -> Router<AppState> {
    router
        .route("/api/reports", get(compat_list).post(compat_create))
        .route("/api/reports/goal", post(compat_goal))
        .route("/api/reports/funnel", post(compat_funnel))
        .route(
            "/api/reports/{report_id}",
            get(compat_get).post(compat_update).delete(compat_delete),
        )
        .route(
            "/api/websites/{website_id}/reports",
            get(compat_website_reports),
        )
        .route(
            "/api/websites/{website_id}/goals",
            get(list_goals).post(create_goal),
        )
        .route("/api/websites/{website_id}/goals/stats", get(goal_stats))
        .route(
            "/api/websites/{website_id}/goals/{report_id}",
            get(get_goal).post(update_goal).delete(delete_goal),
        )
        .route(
            "/api/websites/{website_id}/goals/{report_id}/stats",
            get(saved_goal_stats),
        )
        .route(
            "/api/websites/{website_id}/funnels",
            get(list_funnels).post(create_funnel),
        )
        .route(
            "/api/websites/{website_id}/funnels/stats",
            get(funnel_stats),
        )
        .route(
            "/api/websites/{website_id}/funnels/{report_id}",
            get(get_funnel).post(update_funnel).delete(delete_funnel),
        )
        .route(
            "/api/websites/{website_id}/funnels/{report_id}/stats",
            get(saved_funnel_stats),
        )
}

fn report_json(report: &Report) -> Value {
    json!({
        "id": report.id,
        "userId": report.user_id,
        "websiteId": report.website_id,
        "type": report.kind,
        "name": report.name,
        "description": report.description,
        "parameters": report.parameters,
        "createdAt": report.created_at,
        "updatedAt": report.updated_at,
    })
}

fn report_from_row(row: &sqlx::postgres::PgRow) -> Option<Report> {
    let parameters = row.get::<String, _>("parameters").parse().ok()?;
    Some(Report {
        id: row.get("report_id"),
        user_id: row.get("user_id"),
        website_id: row.get("website_id"),
        kind: row.get("type"),
        name: row.get("name"),
        description: row.get("description"),
        parameters,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

async fn load_report(
    pool: &PgPool,
    website_id: Uuid,
    report_id: Uuid,
    kind: ReportKind,
) -> Result<Option<Report>, ()> {
    let row = sqlx::query(
        r#"
        SELECT report_id, user_id, website_id, type, name, description,
               parameters::text AS parameters, created_at, updated_at
        FROM report
        WHERE report_id = $1 AND website_id = $2 AND type = $3
        "#,
    )
    .bind(report_id)
    .bind(website_id)
    .bind(kind.name())
    .fetch_optional(pool)
    .await
    .map_err(|_| ())?;
    match row {
        None => Ok(None),
        Some(row) => report_from_row(&row).map(Some).ok_or(()),
    }
}

fn database_pool(state: &AppState) -> Result<PgPool, Response> {
    state
        .db
        .clone()
        .ok_or_else(|| error_response(StatusCode::SERVICE_UNAVAILABLE, "Database unavailable"))
}

async fn report_section_pool(
    state: &AppState,
    headers: &HeaderMap,
    website_id: Uuid,
    kind: ReportKind,
) -> Result<PgPool, Response> {
    website_section_pool(
        state,
        headers,
        website_id,
        &[kind.section()],
        ShareDataVisibility::Aggregate,
    )
    .await
}

async fn report_pool(
    state: &AppState,
    headers: &HeaderMap,
    website_id: Uuid,
    report_id: Uuid,
    kind: ReportKind,
    access: Access,
) -> Result<(PgPool, Report), Response> {
    let raw_pool = database_pool(state)?;
    let report = match load_report(&raw_pool, website_id, report_id, kind).await {
        Ok(Some(report)) => report,
        Ok(None) => return Err(status_response(StatusCode::NOT_FOUND)),
        Err(_) => {
            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to load report",
            ))
        }
    };

    if let Some((user_id, role)) = check_bearer_auth(&raw_pool, headers, "/api/websites").await {
        if role == "admin" || user_id == report.user_id {
            return Ok((raw_pool, report));
        }
    }

    let pool = if access == Access::Read {
        website_section_pool(
            state,
            headers,
            website_id,
            &[kind.section()],
            ShareDataVisibility::Aggregate,
        )
        .await
    } else {
        website_pool(state, headers, website_id, access).await
    };
    pool.map(|pool| (pool, report))
}

fn valid_report_type(value: &str) -> bool {
    matches!(
        value,
        "attribution"
            | "breakdown"
            | "funnel"
            | "goal"
            | "heatmap"
            | "journey"
            | "performance"
            | "retention"
            | "revenue"
            | "utm"
    )
}

fn report_section(value: &str) -> Option<ShareSection> {
    match value {
        "attribution" => Some(ShareSection::Attribution),
        "breakdown" => Some(ShareSection::Breakdown),
        "funnel" => Some(ShareSection::Funnels),
        "goal" => Some(ShareSection::Goals),
        "journey" => Some(ShareSection::Journeys),
        "performance" => Some(ShareSection::Performance),
        "retention" => Some(ShareSection::Retention),
        "revenue" => Some(ShareSection::Revenue),
        "utm" => Some(ShareSection::Utm),
        _ => None,
    }
}

async fn load_report_by_id(pool: &PgPool, report_id: Uuid) -> Result<Option<Report>, ()> {
    let row = sqlx::query(
        r#"
        SELECT report_id, user_id, website_id, type, name, description,
               parameters::text AS parameters, created_at, updated_at
        FROM report
        WHERE report_id = $1
        "#,
    )
    .bind(report_id)
    .fetch_optional(pool)
    .await
    .map_err(|_| ())?;
    match row {
        None => Ok(None),
        Some(row) => report_from_row(&row).map(Some).ok_or(()),
    }
}

async fn compat_report_pool(
    state: &AppState,
    headers: &HeaderMap,
    report_id: Uuid,
    access: Access,
) -> Result<(PgPool, Report), Response> {
    let raw_pool = database_pool(state)?;
    let report = match load_report_by_id(&raw_pool, report_id).await {
        Ok(Some(report)) => report,
        Ok(None) => return Err(status_response(StatusCode::NOT_FOUND)),
        Err(_) => {
            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to load report",
            ))
        }
    };
    if let Some((user_id, role)) = check_bearer_auth(&raw_pool, headers, "/api/reports").await {
        if role == "admin" || user_id == report.user_id {
            return Ok((raw_pool, report));
        }
    }
    let pool = if access == Access::Read {
        match report_section(&report.kind) {
            Some(section) => {
                website_section_pool(
                    state,
                    headers,
                    report.website_id,
                    &[section],
                    ShareDataVisibility::Aggregate,
                )
                .await
            }
            None => website_pool(state, headers, report.website_id, Access::Read).await,
        }
    } else {
        website_pool(state, headers, report.website_id, access).await
    };
    pool.map(|pool| (pool, report))
}

fn parse_compat_definition(
    body: &Value,
) -> Result<(Uuid, String, String, Option<String>, Value), &'static str> {
    let object = body.as_object().ok_or("Invalid request body")?;
    let website_id = object
        .get("websiteId")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or("websiteId must be a UUID")?;
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .filter(|value| valid_report_type(value))
        .ok_or("Invalid report type")?;
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .ok_or("name is required")?;
    if string_len(name) > 200 {
        return Err("name must contain at most 200 characters");
    }
    let description = match object.get("description") {
        Some(value) => {
            let value = value.as_str().ok_or("description must be a string")?;
            if string_len(value) > 500 {
                return Err("description must contain at most 500 characters");
            }
            Some(value.to_string())
        }
        None => None,
    };
    let parameters = object
        .get("parameters")
        .and_then(Value::as_object)
        .ok_or("parameters must be an object")?;
    Ok((
        website_id,
        kind.to_string(),
        name.to_string(),
        description,
        Value::Object(parameters.clone()),
    ))
}

fn string_len(value: &str) -> usize {
    value.chars().count()
}

fn parse_definition(body: &Value, kind: ReportKind) -> Result<Definition, &'static str> {
    let object = body.as_object().ok_or("Invalid request body")?;
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .ok_or("name is required")?;
    if name.is_empty() || string_len(name) > 200 {
        return Err("name must contain between 1 and 200 characters");
    }
    let description = match object.get("description") {
        Some(value) => {
            let value = value.as_str().ok_or("description must be a string")?;
            if string_len(value) > 500 {
                return Err("description must contain at most 500 characters");
            }
            Some(value.to_string())
        }
        None => None,
    };
    let parameters = object.get("parameters").ok_or("parameters are required")?;
    let parameters = match kind {
        ReportKind::Goal => serde_json::to_value(parse_goal_parameters(parameters)?)
            .map_err(|_| "Invalid goal parameters")?,
        ReportKind::Funnel => serde_json::to_value(parse_funnel_parameters(parameters)?)
            .map_err(|_| "Invalid funnel parameters")?,
    };
    Ok(Definition {
        name: name.to_string(),
        description,
        parameters,
    })
}

fn parse_goal_parameters(value: &Value) -> Result<GoalParameters, &'static str> {
    let object = value.as_object().ok_or("parameters must be an object")?;
    let action_type = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or("parameters.type is required")?;
    if !matches!(action_type, "path" | "event") {
        return Err("parameters.type must be path or event");
    }
    let value = object
        .get("value")
        .and_then(Value::as_str)
        .ok_or("parameters.value is required")?;
    Ok(GoalParameters {
        action_type: action_type.to_string(),
        value: value.to_string(),
    })
}

fn coerced_positive_number(value: &Value) -> Option<f64> {
    let value = value
        .as_f64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))?;
    (value.is_finite() && value > 0.0).then_some(value)
}

fn parse_funnel_parameters(value: &Value) -> Result<FunnelParameters, &'static str> {
    let object = value.as_object().ok_or("parameters must be an object")?;
    let window = object
        .get("window")
        .and_then(coerced_positive_number)
        .ok_or("parameters.window must be a positive number")?;
    let steps = object
        .get("steps")
        .and_then(Value::as_array)
        .ok_or("parameters.steps are required")?;
    if !(2..=8).contains(&steps.len()) {
        return Err("parameters.steps must contain between 2 and 8 steps");
    }

    let mut parsed_steps = Vec::with_capacity(steps.len());
    for step in steps {
        let step = step
            .as_object()
            .ok_or("Each funnel step must be an object")?;
        let action_type = step
            .get("type")
            .and_then(Value::as_str)
            .ok_or("Each funnel step requires a type")?;
        if !matches!(action_type, "path" | "event") {
            return Err("Funnel step type must be path or event");
        }
        let step_value = step
            .get("value")
            .and_then(Value::as_str)
            .ok_or("Each funnel step requires a value")?;
        let filters = match step.get("filters") {
            None => None,
            Some(value) => {
                let values = value
                    .as_array()
                    .ok_or("Funnel step filters must be an array")?;
                let mut filters = Vec::with_capacity(values.len());
                for filter in values {
                    let filter = filter
                        .as_object()
                        .ok_or("Each funnel step filter must be an object")?;
                    let property = filter
                        .get("property")
                        .and_then(Value::as_str)
                        .filter(|value| !value.is_empty())
                        .ok_or("Each funnel step filter requires a property")?;
                    let operator = filter
                        .get("operator")
                        .and_then(Value::as_str)
                        .ok_or("Each funnel step filter requires an operator")?;
                    if !matches!(operator, "eq" | "neq" | "c" | "dnc") {
                        return Err("Invalid funnel step filter operator");
                    }
                    let value = filter
                        .get("value")
                        .and_then(Value::as_str)
                        .ok_or("Each funnel step filter requires a value")?;
                    filters.push(FunnelStepFilter {
                        property: property.to_string(),
                        operator: operator.to_string(),
                        value: value.to_string(),
                    });
                }
                Some(filters)
            }
        };
        parsed_steps.push(FunnelStep {
            action_type: action_type.to_string(),
            value: step_value.to_string(),
            filters,
        });
    }
    Ok(FunnelParameters {
        window,
        steps: parsed_steps,
    })
}

fn bad_request(message: impl Into<String>) -> Response {
    error_response(StatusCode::BAD_REQUEST, message)
}

fn positive_param(
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
            .ok_or_else(|| bad_request(format!("{key} must be a positive integer"))),
    }
}

async fn list_definitions(
    state: AppState,
    headers: HeaderMap,
    website_id: Uuid,
    params: HashMap<String, String>,
    kind: ReportKind,
) -> Response {
    let page = match positive_param(&params, "page", 1) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let page_size = match positive_param(&params, "pageSize", 20) {
        Ok(value) => value,
        Err(response) => return response,
    };
    if let Err(response) = positive_param(&params, "maxResults", 1) {
        return response;
    }
    let pool = match report_section_pool(&state, &headers, website_id, kind).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let search = params.get("search").cloned();
    let search_pattern = search.as_ref().map(|value| format!("%{value}%"));
    let count = match sqlx::query(
        r#"
        SELECT count(*)::bigint AS count
        FROM report r
        JOIN website w ON w.website_id = r.website_id
        JOIN "user" u ON u.user_id = r.user_id
        WHERE r.website_id = $1
          AND r.type = $2
          AND w.deleted_at IS NULL
          AND (
            $3::text IS NULL
            OR r.name ILIKE $3
            OR r.description ILIKE $3
            OR r.type ILIKE $3
            OR u.username ILIKE $3
            OR w.name ILIKE $3
            OR w.domain ILIKE $3
          )
        "#,
    )
    .bind(website_id)
    .bind(kind.name())
    .bind(search_pattern.as_deref())
    .fetch_one(&pool)
    .await
    {
        Ok(row) => row.get::<i64, _>("count"),
        Err(_) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Unable to count reports")
        }
    };
    let offset = match (page - 1).checked_mul(page_size) {
        Some(offset) => offset,
        None => return bad_request("Invalid pagination"),
    };
    let rows = match sqlx::query(
        r#"
        SELECT r.report_id, r.user_id, r.website_id, r.type, r.name, r.description,
               r.parameters::text AS parameters, r.created_at, r.updated_at
        FROM report r
        JOIN website w ON w.website_id = r.website_id
        JOIN "user" u ON u.user_id = r.user_id
        WHERE r.website_id = $1
          AND r.type = $2
          AND w.deleted_at IS NULL
          AND (
            $3::text IS NULL
            OR r.name ILIKE $3
            OR r.description ILIKE $3
            OR r.type ILIKE $3
            OR u.username ILIKE $3
            OR w.name ILIKE $3
            OR w.domain ILIKE $3
          )
        ORDER BY r.name ASC, r.report_id ASC
        LIMIT $4 OFFSET $5
        "#,
    )
    .bind(website_id)
    .bind(kind.name())
    .bind(search_pattern.as_deref())
    .bind(page_size)
    .bind(offset)
    .fetch_all(&pool)
    .await
    {
        Ok(rows) => rows,
        Err(_) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Unable to load reports")
        }
    };
    let Some(data) = rows
        .iter()
        .map(report_from_row)
        .map(|report| report.map(|report| report_json(&report)))
        .collect::<Option<Vec<_>>>()
    else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unable to decode report parameters",
        );
    };
    let mut result = Map::from_iter([
        ("data".to_string(), Value::Array(data)),
        ("count".to_string(), Value::from(count)),
        ("page".to_string(), Value::from(page)),
        ("pageSize".to_string(), Value::from(page_size)),
    ]);
    if let Some(search) = search {
        result.insert("search".to_string(), Value::String(search));
    }
    json_response(Value::Object(result))
}

async fn list_goals(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    list_definitions(state, headers, website_id, params, ReportKind::Goal).await
}

async fn list_funnels(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    list_definitions(state, headers, website_id, params, ReportKind::Funnel).await
}

async fn compat_list(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let website_id = match params
        .get("websiteId")
        .and_then(|value| Uuid::parse_str(value).ok())
    {
        Some(website_id) => website_id,
        None => return bad_request("websiteId must be a UUID"),
    };
    let kind = match params.get("type") {
        Some(kind) if valid_report_type(kind) => Some(kind.as_str()),
        Some(_) => return bad_request("Invalid report type"),
        None => None,
    };
    let page = match positive_param(&params, "page", 1) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let page_size = match positive_param(&params, "pageSize", 20) {
        Ok(value) => value,
        Err(response) => return response,
    };
    if let Err(response) = positive_param(&params, "maxResults", 1) {
        return response;
    }
    let pool = match kind.and_then(report_section) {
        Some(section) => {
            website_section_pool(
                &state,
                &headers,
                website_id,
                &[section],
                ShareDataVisibility::Aggregate,
            )
            .await
        }
        None => website_pool(&state, &headers, website_id, Access::Read).await,
    };
    let pool = match pool {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    compat_report_page(&pool, website_id, kind, page, page_size, true).await
}

async fn compat_website_reports(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let kind = match params.get("type") {
        Some(kind) if valid_report_type(kind) => Some(kind.as_str()),
        Some(_) => return bad_request("Invalid report type"),
        None => None,
    };
    let page = match positive_param(&params, "page", 1) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let page_size = match positive_param(&params, "pageSize", 20) {
        Ok(value) => value,
        Err(response) => return response,
    };
    if let Err(response) = positive_param(&params, "maxResults", 1) {
        return response;
    }
    let pool = match website_pool(&state, &headers, website_id, Access::Read).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    compat_report_page(&pool, website_id, kind, page, page_size, false).await
}

async fn compat_report_page(
    pool: &PgPool,
    website_id: Uuid,
    kind: Option<&str>,
    page: i64,
    page_size: i64,
    active_website_only: bool,
) -> Response {
    let count = match sqlx::query(
        r#"
        SELECT count(*)::bigint AS count
        FROM report r
        JOIN website w ON w.website_id = r.website_id
        WHERE r.website_id = $1
          AND ($2::text IS NULL OR r.type = $2)
          AND (NOT $3 OR w.deleted_at IS NULL)
        "#,
    )
    .bind(website_id)
    .bind(kind)
    .bind(active_website_only)
    .fetch_one(pool)
    .await
    {
        Ok(row) => row.get::<i64, _>("count"),
        Err(_) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Unable to count reports")
        }
    };
    let offset = match (page - 1).checked_mul(page_size) {
        Some(offset) => offset,
        None => return bad_request("Invalid pagination"),
    };
    let rows = match sqlx::query(
        r#"
        SELECT r.report_id, r.user_id, r.website_id, r.type, r.name, r.description,
               r.parameters::text AS parameters, r.created_at, r.updated_at
        FROM report r
        JOIN website w ON w.website_id = r.website_id
        WHERE r.website_id = $1
          AND ($2::text IS NULL OR r.type = $2)
          AND (NOT $3 OR w.deleted_at IS NULL)
        ORDER BY r.name ASC, r.report_id ASC
        LIMIT $4 OFFSET $5
        "#,
    )
    .bind(website_id)
    .bind(kind)
    .bind(active_website_only)
    .bind(page_size)
    .bind(offset)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(_) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Unable to load reports")
        }
    };
    let Some(data) = rows
        .iter()
        .map(report_from_row)
        .map(|report| report.map(|report| report_json(&report)))
        .collect::<Option<Vec<_>>>()
    else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unable to decode report parameters",
        );
    };
    json_response(json!({
        "data": data,
        "count": count,
        "page": page,
        "pageSize": page_size,
    }))
}

async fn compat_create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let (website_id, kind, name, description, parameters) = match parse_compat_definition(&body) {
        Ok(definition) => definition,
        Err(message) => return bad_request(message),
    };
    let pool = match website_pool(&state, &headers, website_id, Access::Write).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let Some((user_id, _)) = check_bearer_auth(&pool, &headers, "/api/reports").await else {
        return status_response(StatusCode::UNAUTHORIZED);
    };
    let row = sqlx::query(
        r#"
        INSERT INTO report
            (report_id, user_id, website_id, type, name, description, parameters,
             created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7::jsonb, NOW(), NOW())
        RETURNING report_id, user_id, website_id, type, name, description,
                  parameters::text AS parameters, created_at, updated_at
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(website_id)
    .bind(kind)
    .bind(name)
    .bind(description.unwrap_or_default())
    .bind(parameters.to_string())
    .fetch_one(&pool)
    .await;
    match row {
        Ok(row) => match report_from_row(&row) {
            Some(report) => json_response(report_json(&report)),
            None => error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to decode report parameters",
            ),
        },
        Err(_) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "Unable to create report"),
    }
}

async fn compat_get(
    State(state): State<AppState>,
    Path(report_id): Path<Uuid>,
    headers: HeaderMap,
) -> Response {
    match compat_report_pool(&state, &headers, report_id, Access::Read).await {
        Ok((_, report)) => json_response(report_json(&report)),
        Err(response) => response,
    }
}

async fn compat_update(
    State(state): State<AppState>,
    Path(report_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let (_, kind, name, description, parameters) = match parse_compat_definition(&body) {
        Ok(definition) => definition,
        Err(message) => return bad_request(message),
    };
    let (pool, _) = match compat_report_pool(&state, &headers, report_id, Access::Write).await {
        Ok(result) => result,
        Err(response) => return response,
    };
    let row = sqlx::query(
        r#"
        UPDATE report
        SET type = $1,
            name = $2,
            description = COALESCE($3, description),
            parameters = $4::jsonb,
            updated_at = NOW()
        WHERE report_id = $5
        RETURNING report_id, user_id, website_id, type, name, description,
                  parameters::text AS parameters, created_at, updated_at
        "#,
    )
    .bind(kind)
    .bind(name)
    .bind(description)
    .bind(parameters.to_string())
    .bind(report_id)
    .fetch_optional(&pool)
    .await;
    match row {
        Ok(Some(row)) => match report_from_row(&row) {
            Some(report) => json_response(report_json(&report)),
            None => error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to decode report parameters",
            ),
        },
        Ok(None) => status_response(StatusCode::NOT_FOUND),
        Err(_) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "Unable to update report"),
    }
}

async fn compat_delete(
    State(state): State<AppState>,
    Path(report_id): Path<Uuid>,
    headers: HeaderMap,
) -> Response {
    let (pool, _) = match compat_report_pool(&state, &headers, report_id, Access::Delete).await {
        Ok(result) => result,
        Err(response) => return response,
    };
    match sqlx::query("DELETE FROM report WHERE report_id = $1")
        .bind(report_id)
        .execute(&pool)
        .await
    {
        Ok(result) if result.rows_affected() == 1 => json_response(json!({ "ok": true })),
        Ok(_) => status_response(StatusCode::NOT_FOUND),
        Err(_) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "Unable to delete report"),
    }
}

async fn create_definition(
    state: AppState,
    headers: HeaderMap,
    website_id: Uuid,
    body: Value,
    kind: ReportKind,
) -> Response {
    let definition = match parse_definition(&body, kind) {
        Ok(definition) => definition,
        Err(message) => return bad_request(message),
    };
    let pool = match website_pool(&state, &headers, website_id, Access::Write).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let Some((user_id, _)) = check_bearer_auth(&pool, &headers, "/api/websites").await else {
        return status_response(StatusCode::UNAUTHORIZED);
    };
    let parameters = definition.parameters.to_string();
    let rows = sqlx::query(
        r#"
        INSERT INTO report
            (report_id, user_id, website_id, type, name, description, parameters,
             created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7::jsonb, NOW(), NOW())
        RETURNING report_id, user_id, website_id, type, name, description,
                  parameters::text AS parameters, created_at, updated_at
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(website_id)
    .bind(kind.name())
    .bind(definition.name)
    .bind(definition.description.unwrap_or_default())
    .bind(parameters)
    .fetch_one(&pool)
    .await;
    match rows {
        Ok(row) => match report_from_row(&row) {
            Some(report) => json_response(report_json(&report)),
            None => error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to decode report parameters",
            ),
        },
        Err(_) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "Unable to create report"),
    }
}

async fn create_goal(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    create_definition(state, headers, website_id, body, ReportKind::Goal).await
}

async fn create_funnel(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    create_definition(state, headers, website_id, body, ReportKind::Funnel).await
}

async fn get_definition(
    state: AppState,
    headers: HeaderMap,
    website_id: Uuid,
    report_id: Uuid,
    kind: ReportKind,
) -> Response {
    match report_pool(&state, &headers, website_id, report_id, kind, Access::Read).await {
        Ok((_, report)) => json_response(report_json(&report)),
        Err(response) => response,
    }
}

async fn get_goal(
    State(state): State<AppState>,
    Path((website_id, report_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Response {
    get_definition(state, headers, website_id, report_id, ReportKind::Goal).await
}

async fn get_funnel(
    State(state): State<AppState>,
    Path((website_id, report_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Response {
    get_definition(state, headers, website_id, report_id, ReportKind::Funnel).await
}

async fn update_definition(
    state: AppState,
    headers: HeaderMap,
    website_id: Uuid,
    report_id: Uuid,
    body: Value,
    kind: ReportKind,
) -> Response {
    let definition = match parse_definition(&body, kind) {
        Ok(definition) => definition,
        Err(message) => return bad_request(message),
    };
    let (pool, _) =
        match report_pool(&state, &headers, website_id, report_id, kind, Access::Write).await {
            Ok(result) => result,
            Err(response) => return response,
        };
    let row = sqlx::query(
        r#"
        UPDATE report
        SET name = $1,
            description = COALESCE($2, description),
            parameters = $3::jsonb,
            updated_at = NOW()
        WHERE report_id = $4 AND website_id = $5 AND type = $6
        RETURNING report_id, user_id, website_id, type, name, description,
                  parameters::text AS parameters, created_at, updated_at
        "#,
    )
    .bind(definition.name)
    .bind(definition.description)
    .bind(definition.parameters.to_string())
    .bind(report_id)
    .bind(website_id)
    .bind(kind.name())
    .fetch_optional(&pool)
    .await;
    match row {
        Ok(Some(row)) => match report_from_row(&row) {
            Some(report) => json_response(report_json(&report)),
            None => error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to decode report parameters",
            ),
        },
        Ok(None) => status_response(StatusCode::NOT_FOUND),
        Err(_) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "Unable to update report"),
    }
}

async fn update_goal(
    State(state): State<AppState>,
    Path((website_id, report_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    update_definition(
        state,
        headers,
        website_id,
        report_id,
        body,
        ReportKind::Goal,
    )
    .await
}

async fn update_funnel(
    State(state): State<AppState>,
    Path((website_id, report_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    update_definition(
        state,
        headers,
        website_id,
        report_id,
        body,
        ReportKind::Funnel,
    )
    .await
}

async fn delete_definition(
    state: AppState,
    headers: HeaderMap,
    website_id: Uuid,
    report_id: Uuid,
    kind: ReportKind,
) -> Response {
    let (pool, _) = match report_pool(
        &state,
        &headers,
        website_id,
        report_id,
        kind,
        Access::Delete,
    )
    .await
    {
        Ok(result) => result,
        Err(response) => return response,
    };
    match sqlx::query("DELETE FROM report WHERE report_id = $1 AND website_id = $2 AND type = $3")
        .bind(report_id)
        .bind(website_id)
        .bind(kind.name())
        .execute(&pool)
        .await
    {
        Ok(result) if result.rows_affected() == 1 => json_response(json!({ "ok": true })),
        Ok(_) => status_response(StatusCode::NOT_FOUND),
        Err(_) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "Unable to delete report"),
    }
}

async fn delete_goal(
    State(state): State<AppState>,
    Path((website_id, report_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Response {
    delete_definition(state, headers, website_id, report_id, ReportKind::Goal).await
}

async fn delete_funnel(
    State(state): State<AppState>,
    Path((website_id, report_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Response {
    delete_definition(state, headers, website_id, report_id, ReportKind::Funnel).await
}

fn parse_date_range(params: &HashMap<String, String>) -> Result<DateRange, Response> {
    fn parse(params: &HashMap<String, String>, key: &str) -> Result<DateTime<Utc>, Response> {
        let value = params
            .get(key)
            .ok_or_else(|| bad_request(format!("{key} is required")))?;
        let millis = value
            .parse::<i64>()
            .ok()
            .filter(|value| *value >= 0)
            .ok_or_else(|| bad_request(format!("{key} must be a non-negative integer")))?;
        Utc.timestamp_millis_opt(millis)
            .single()
            .ok_or_else(|| bad_request(format!("{key} is outside the supported date range")))
    }

    let start = parse(params, "startAt")?;
    let end = parse(params, "endAt")?;
    if end < start {
        return Err(bad_request(
            "endAt must be greater than or equal to startAt",
        ));
    }
    if let Some(unit) = params.get("unit") {
        if !matches!(unit.as_str(), "minute" | "hour" | "day" | "month" | "year") {
            return Err(bad_request("Invalid unit"));
        }
    }
    Ok(DateRange { start, end })
}

async fn clamp_website_date(
    pool: &PgPool,
    website_id: Uuid,
    range: DateRange,
) -> Result<DateRange, Response> {
    let reset_at = sqlx::query("SELECT reset_at FROM website WHERE website_id = $1")
        .bind(website_id)
        .fetch_optional(pool)
        .await
        .map_err(|_| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to load website date range",
            )
        })?
        .and_then(|row| row.get::<Option<DateTime<Utc>>, _>("reset_at"));
    Ok(DateRange {
        start: reset_at.map_or(range.start, |reset| range.start.max(reset)),
        end: range.end,
    })
}

fn wildcard(value: &str) -> (bool, String) {
    if !value.contains('*') {
        return (false, value.to_string());
    }
    let mut pattern = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' | '%' | '_' => {
                pattern.push('\\');
                pattern.push(character);
            }
            '*' => pattern.push('%'),
            _ => pattern.push(character),
        }
    }
    (true, pattern)
}

fn push_step_match(qb: &mut QueryBuilder<'_, Postgres>, alias: &str, step: &FunnelStep) {
    let column = if step.action_type == "path" {
        "url_path"
    } else {
        "event_name"
    };
    let (is_like, value) = wildcard(&step.value);
    qb.push(" AND ")
        .push(alias)
        .push(".")
        .push(column)
        .push(if is_like { " LIKE " } else { " = " })
        .push_bind(value);
    if is_like {
        qb.push(" ESCAPE '\\'");
    }
}

fn push_step_filters(
    qb: &mut QueryBuilder<'_, Postgres>,
    alias: &str,
    step: &FunnelStep,
    website_id: Uuid,
    range: DateRange,
) {
    if step.action_type != "event" {
        return;
    }
    for filter in step.filters.as_deref().unwrap_or_default() {
        qb.push(" AND EXISTS (SELECT 1 FROM event_data ed WHERE ed.website_event_id = ")
            .push(alias)
            .push(".event_id AND ed.website_id = ")
            .push_bind(website_id)
            .push(" AND ed.created_at BETWEEN ")
            .push_bind(range.start)
            .push(" AND ")
            .push_bind(range.end)
            .push(" AND ed.data_key = ")
            .push_bind(filter.property.clone())
            .push(" AND CASE WHEN ed.data_type = 2 THEN replace(ed.string_value, '.0000', '') ELSE ed.string_value END ");
        let value = match filter.operator.as_str() {
            "neq" => {
                qb.push("!= ");
                filter.value.clone()
            }
            "c" => {
                qb.push("ILIKE ");
                format!("%{}%", filter.value)
            }
            "dnc" => {
                qb.push("NOT ILIKE ");
                format!("%{}%", filter.value)
            }
            _ => {
                qb.push("= ");
                filter.value.clone()
            }
        };
        qb.push_bind(value).push(")");
    }
}

fn goal_query(
    website_id: Uuid,
    range: DateRange,
    filters: &ScalarFilterPredicate,
    audience: &Audience,
    parameters: &GoalParameters,
) -> QueryBuilder<'static, Postgres> {
    let column = if parameters.action_type == "path" {
        "url_path"
    } else {
        "event_name"
    };
    let event_type = if parameters.action_type == "path" {
        1
    } else {
        2
    };
    let mut qb = QueryBuilder::<Postgres>::new(
        "WITH matching AS (SELECT DISTINCT e.session_id FROM website_event e ",
    );
    if filters.needs_session() {
        qb.push("JOIN session s ON s.session_id = e.session_id AND s.website_id = e.website_id ");
    }
    qb.push("WHERE e.website_id = ")
        .push_bind(website_id)
        .push(" AND e.created_at BETWEEN ")
        .push_bind(range.start)
        .push(" AND ")
        .push_bind(range.end);
    filters.push(&mut qb, true);
    audience.push(&mut qb, "e", range.start, range.end);
    filters.push_bounce(&mut qb, range.start, range.end);
    let (is_like, value) = wildcard(&parameters.value);
    qb.push(" AND e.event_type = ")
        .push_bind(event_type)
        .push(" AND e.")
        .push(column)
        .push(if is_like { " LIKE " } else { " = " })
        .push_bind(value);
    if is_like {
        qb.push(" ESCAPE '\\'");
    }
    qb.push("), total AS (SELECT DISTINCT e.session_id FROM website_event e ");
    if filters.needs_session() {
        qb.push("JOIN session s ON s.session_id = e.session_id AND s.website_id = e.website_id ");
    }
    qb.push("WHERE e.website_id = ")
        .push_bind(website_id)
        .push(" AND e.created_at BETWEEN ")
        .push_bind(range.start)
        .push(" AND ")
        .push_bind(range.end);
    filters.push(&mut qb, false);
    audience.push(&mut qb, "e", range.start, range.end);
    filters.push_bounce(&mut qb, range.start, range.end);
    qb.push(") SELECT (SELECT COUNT(*) FROM matching)::bigint AS num, (SELECT COUNT(*) FROM total)::bigint AS total");
    qb
}

async fn run_goal(
    pool: &PgPool,
    website_id: Uuid,
    range: DateRange,
    filters: &ScalarFilterPredicate,
    audience: &Audience,
    parameters: GoalParameters,
) -> Result<Value, sqlx::Error> {
    let mut qb = goal_query(website_id, range, filters, audience, &parameters);
    let row = qb.build().fetch_one(pool).await?;
    Ok(json!({
        "num": row.get::<i64, _>("num"),
        "total": row.get::<i64, _>("total"),
    }))
}

fn funnel_query(
    website_id: Uuid,
    range: DateRange,
    filters: &ScalarFilterPredicate,
    audience: &Audience,
    parameters: &FunnelParameters,
) -> QueryBuilder<'static, Postgres> {
    let mut qb = QueryBuilder::<Postgres>::new("WITH ");
    for (index, step) in parameters.steps.iter().enumerate() {
        let level = index + 1;
        if index > 0 {
            qb.push(", ");
        }
        qb.push(format!("level{level} AS (SELECT DISTINCT "));
        if index == 0 {
            qb.push("e.session_id, e.created_at FROM website_event e ");
            if filters.needs_session() {
                qb.push("JOIN session s ON s.session_id = e.session_id AND s.website_id = e.website_id ");
            }
            qb.push("WHERE e.website_id = ")
                .push_bind(website_id)
                .push(" AND e.created_at BETWEEN ")
                .push_bind(range.start)
                .push(" AND ")
                .push_bind(range.end);
            filters.push(&mut qb, true);
            audience.push(&mut qb, "e", range.start, range.end);
            filters.push_bounce(&mut qb, range.start, range.end);
            push_step_match(&mut qb, "e", step);
            push_step_filters(&mut qb, "e", step, website_id, range);
        } else {
            qb.push(format!(
                "we.session_id, we.created_at FROM level{index} l JOIN website_event we ON l.session_id = we.session_id WHERE we.website_id = "
            ))
            .push_bind(website_id)
            .push(" AND we.created_at BETWEEN l.created_at AND l.created_at + (")
            .push_bind(parameters.window)
            .push(" * interval '1 minute') AND we.created_at >= ")
            .push_bind(range.start)
            .push(" AND we.created_at <= ")
            .push_bind(range.end);
            push_step_match(&mut qb, "we", step);
            push_step_filters(&mut qb, "we", step, website_id, range);
        }
        qb.push(")");
    }
    qb.push(" ");
    for index in 0..parameters.steps.len() {
        if index > 0 {
            qb.push(" UNION ");
        }
        let level = index + 1;
        qb.push(format!(
            "SELECT {level}::bigint AS level, COUNT(DISTINCT session_id)::bigint AS count FROM level{level}"
        ));
    }
    qb.push(" ORDER BY level");

    qb
}

async fn run_funnel(
    pool: &PgPool,
    website_id: Uuid,
    range: DateRange,
    filters: &ScalarFilterPredicate,
    audience: &Audience,
    parameters: FunnelParameters,
) -> Result<Value, sqlx::Error> {
    let mut qb = funnel_query(website_id, range, filters, audience, &parameters);
    let rows = qb.build().fetch_all(pool).await?;
    let counts = rows
        .iter()
        .map(|row| row.get::<i64, _>("count"))
        .collect::<Vec<_>>();
    Ok(format_funnel_results(&parameters.steps, &counts))
}

fn format_funnel_results(steps: &[FunnelStep], counts: &[i64]) -> Value {
    let first = counts.first().copied().unwrap_or_default();
    Value::Array(
        steps
            .iter()
            .enumerate()
            .map(|(index, step)| {
                let visitors = counts.get(index).copied().unwrap_or_default();
                let previous = index
                    .checked_sub(1)
                    .and_then(|previous| counts.get(previous).copied())
                    .unwrap_or_default();
                let dropped = if previous > 0 { previous - visitors } else { 0 };
                let dropoff = (previous > 0)
                    .then(|| Value::from(1.0 - visitors as f64 / previous as f64))
                    .unwrap_or(Value::Null);
                let remaining = (first > 0)
                    .then(|| Value::from(visitors as f64 / first as f64))
                    .unwrap_or(Value::Null);
                let mut value = serde_json::to_value(step)
                    .expect("serializing validated funnel steps cannot fail");
                if let Some(object) = value.as_object_mut() {
                    object.insert("visitors".to_string(), Value::from(visitors));
                    object.insert("previous".to_string(), Value::from(previous));
                    object.insert("dropped".to_string(), Value::from(dropped));
                    object.insert("dropoff".to_string(), dropoff);
                    object.insert("remaining".to_string(), remaining);
                }
                value
            })
            .collect(),
    )
}

fn stats_context(params: &HashMap<String, String>) -> Result<DateRange, Response> {
    parse_date_range(params)
}

async fn report_filter_predicate(
    pool: &PgPool,
    website_id: Uuid,
    params: &mut HashMap<String, String>,
    event_type: Option<i32>,
) -> Result<(ScalarFilterPredicate, Audience), Response> {
    let audience = Audience::resolve(pool, website_id, params).await?;
    params.retain(|key, _| !key.starts_with("pf_") && key != "epf" && key != "spf");
    if params.contains_key("filters") {
        return Err(bad_request("Unsupported query parameter: filters"));
    }
    if let Some(event_type) = event_type {
        params.insert("eventType".to_string(), event_type.to_string());
    }
    scalar_filter_predicate(params).map(|filters| (filters, audience))
}

fn compat_filters(value: Option<&Value>) -> Result<HashMap<String, String>, Response> {
    report_filter_params(value)
}

fn compat_date_range(parameters: &Map<String, Value>) -> Result<DateRange, Response> {
    fn parse(value: Option<&Value>, key: &str) -> Result<DateTime<Utc>, Response> {
        let value = value.ok_or_else(|| bad_request(format!("{key} is required")))?;
        if let Some(millis) = value.as_i64() {
            return Utc
                .timestamp_millis_opt(millis)
                .single()
                .ok_or_else(|| bad_request(format!("{key} is outside the supported date range")));
        }
        let value = value
            .as_str()
            .ok_or_else(|| bad_request(format!("{key} must be a date")))?;
        DateTime::parse_from_rfc3339(value)
            .map(|date| date.with_timezone(&Utc))
            .map_err(|_| bad_request(format!("{key} must be a date")))
    }
    let start = parse(parameters.get("startDate"), "startDate")?;
    let end = parse(parameters.get("endDate"), "endDate")?;
    Ok(DateRange { start, end })
}

fn compat_run_parts(
    body: &Value,
    expected_type: &str,
) -> Result<(Uuid, Map<String, Value>, HashMap<String, String>), Response> {
    let object = body
        .as_object()
        .ok_or_else(|| bad_request("Invalid request body"))?;
    if object.get("type").and_then(Value::as_str) != Some(expected_type) {
        return Err(bad_request("Report type does not match the route"));
    }
    let website_id = object
        .get("websiteId")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(|| bad_request("websiteId must be a UUID"))?;
    let parameters = object
        .get("parameters")
        .and_then(Value::as_object)
        .cloned()
        .ok_or_else(|| bad_request("parameters must be an object"))?;
    let filters = compat_filters(object.get("filters"))?;
    Ok((website_id, parameters, filters))
}

async fn compat_goal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let (website_id, parameters, mut filters) = match compat_run_parts(&body, "goal") {
        Ok(parts) => parts,
        Err(response) => return response,
    };
    let range = match compat_date_range(&parameters) {
        Ok(range) => range,
        Err(response) => return response,
    };
    let parameters = match parse_goal_parameters(&Value::Object(parameters)) {
        Ok(parameters) => parameters,
        Err(message) => return bad_request(message),
    };
    let pool = match report_section_pool(&state, &headers, website_id, ReportKind::Goal).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let range = match clamp_website_date(&pool, website_id, range).await {
        Ok(range) => range,
        Err(response) => return response,
    };
    let event_type = if parameters.action_type == "path" {
        1
    } else {
        2
    };
    let (filters, audience) =
        match report_filter_predicate(&pool, website_id, &mut filters, Some(event_type)).await {
            Ok(filters) => filters,
            Err(response) => return response,
        };
    match run_goal(&pool, website_id, range, &filters, &audience, parameters).await {
        Ok(result) => json_response(result),
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unable to calculate goal",
        ),
    }
}

async fn compat_funnel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let (website_id, parameters, mut filters) = match compat_run_parts(&body, "funnel") {
        Ok(parts) => parts,
        Err(response) => return response,
    };
    let range = match compat_date_range(&parameters) {
        Ok(range) => range,
        Err(response) => return response,
    };
    let parameters = match parse_funnel_parameters(&Value::Object(parameters)) {
        Ok(parameters) => parameters,
        Err(message) => return bad_request(message),
    };
    let pool = match report_section_pool(&state, &headers, website_id, ReportKind::Funnel).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let range = match clamp_website_date(&pool, website_id, range).await {
        Ok(range) => range,
        Err(response) => return response,
    };
    let (filters, audience) =
        match report_filter_predicate(&pool, website_id, &mut filters, None).await {
            Ok(filters) => filters,
            Err(response) => return response,
        };
    match run_funnel(&pool, website_id, range, &filters, &audience, parameters).await {
        Ok(result) => json_response(result),
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unable to calculate funnel",
        ),
    }
}

async fn goal_stats(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(mut params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let range = match stats_context(&params) {
        Ok(range) => range,
        Err(response) => return response,
    };
    let parameters = match parse_goal_parameters(&Value::Object(
        ["type", "value"]
            .into_iter()
            .filter_map(|key| {
                params
                    .get(key)
                    .map(|value| (key.to_string(), Value::String(value.clone())))
            })
            .collect(),
    )) {
        Ok(parameters) => parameters,
        Err(message) => return bad_request(message),
    };
    let pool = match report_section_pool(&state, &headers, website_id, ReportKind::Goal).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let range = match clamp_website_date(&pool, website_id, range).await {
        Ok(range) => range,
        Err(response) => return response,
    };
    let event_type = if parameters.action_type == "path" {
        1
    } else {
        2
    };
    let (filters, audience) =
        match report_filter_predicate(&pool, website_id, &mut params, Some(event_type)).await {
            Ok(filters) => filters,
            Err(response) => return response,
        };
    match run_goal(&pool, website_id, range, &filters, &audience, parameters).await {
        Ok(result) => json_response(result),
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unable to calculate goal",
        ),
    }
}

async fn saved_goal_stats(
    State(state): State<AppState>,
    Path((website_id, report_id)): Path<(Uuid, Uuid)>,
    Query(mut params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let range = match stats_context(&params) {
        Ok(range) => range,
        Err(response) => return response,
    };
    let pool = match report_section_pool(&state, &headers, website_id, ReportKind::Goal).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let report = match load_report(&pool, website_id, report_id, ReportKind::Goal).await {
        Ok(Some(report)) => report,
        Ok(None) => return status_response(StatusCode::NOT_FOUND),
        Err(_) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Unable to load report")
        }
    };
    let parameters = match parse_goal_parameters(&report.parameters) {
        Ok(parameters) => parameters,
        Err(message) => return bad_request(message),
    };
    let range = match clamp_website_date(&pool, website_id, range).await {
        Ok(range) => range,
        Err(response) => return response,
    };
    let event_type = if parameters.action_type == "path" {
        1
    } else {
        2
    };
    let (filters, audience) =
        match report_filter_predicate(&pool, website_id, &mut params, Some(event_type)).await {
            Ok(filters) => filters,
            Err(response) => return response,
        };
    match run_goal(&pool, website_id, range, &filters, &audience, parameters).await {
        Ok(result) => json_response(result),
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unable to calculate goal",
        ),
    }
}

fn funnel_query_parameters(params: &HashMap<String, String>) -> Result<FunnelParameters, Response> {
    let window = params
        .get("window")
        .map(|value| Value::String(value.clone()))
        .ok_or_else(|| bad_request("parameters.window is required"))?;
    let steps = params
        .get("steps")
        .ok_or_else(|| bad_request("parameters.steps are required"))?;
    let steps: Value =
        serde_json::from_str(steps).map_err(|_| bad_request("Invalid steps JSON"))?;
    parse_funnel_parameters(&json!({ "window": window, "steps": steps })).map_err(bad_request)
}

async fn funnel_stats(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(mut params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let range = match stats_context(&params) {
        Ok(range) => range,
        Err(response) => return response,
    };
    let parameters = match funnel_query_parameters(&params) {
        Ok(parameters) => parameters,
        Err(response) => return response,
    };
    let pool = match report_section_pool(&state, &headers, website_id, ReportKind::Funnel).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let range = match clamp_website_date(&pool, website_id, range).await {
        Ok(range) => range,
        Err(response) => return response,
    };
    let (filters, audience) =
        match report_filter_predicate(&pool, website_id, &mut params, None).await {
            Ok(filters) => filters,
            Err(response) => return response,
        };
    match run_funnel(&pool, website_id, range, &filters, &audience, parameters).await {
        Ok(result) => json_response(result),
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unable to calculate funnel",
        ),
    }
}

async fn saved_funnel_stats(
    State(state): State<AppState>,
    Path((website_id, report_id)): Path<(Uuid, Uuid)>,
    Query(mut params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let range = match stats_context(&params) {
        Ok(range) => range,
        Err(response) => return response,
    };
    let pool = match report_section_pool(&state, &headers, website_id, ReportKind::Funnel).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let report = match load_report(&pool, website_id, report_id, ReportKind::Funnel).await {
        Ok(Some(report)) => report,
        Ok(None) => return status_response(StatusCode::NOT_FOUND),
        Err(_) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Unable to load report")
        }
    };
    let parameters = match parse_funnel_parameters(&report.parameters) {
        Ok(parameters) => parameters,
        Err(message) => return bad_request(message),
    };
    let range = match clamp_website_date(&pool, website_id, range).await {
        Ok(range) => range,
        Err(response) => return response,
    };
    let (filters, audience) =
        match report_filter_predicate(&pool, website_id, &mut params, None).await {
            Ok(filters) => filters,
            Err(response) => return response,
        };
    match run_funnel(&pool, website_id, range, &filters, &audience, parameters).await {
        Ok(result) => json_response(result),
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unable to calculate funnel",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_and_normalizes_definitions() {
        let goal = parse_definition(
            &json!({
                "name": "Signup",
                "parameters": { "type": "event", "value": "signup", "ignored": true }
            }),
            ReportKind::Goal,
        )
        .unwrap();
        assert_eq!(
            goal.parameters,
            json!({ "type": "event", "value": "signup" })
        );

        let funnel = parse_definition(
            &json!({
                "name": "Checkout",
                "description": "Two steps",
                "parameters": {
                    "window": "30",
                    "steps": [
                        { "type": "path", "value": "/" },
                        {
                            "type": "event",
                            "value": "purchase",
                            "filters": [{ "property": "plan", "operator": "c", "value": "pro" }]
                        }
                    ]
                }
            }),
            ReportKind::Funnel,
        )
        .unwrap();
        assert_eq!(funnel.parameters["window"], 30.0);
        assert!(parse_definition(
            &json!({
                "name": "",
                "parameters": { "type": "path", "value": "/" }
            }),
            ReportKind::Goal
        )
        .is_err());
    }

    #[test]
    fn wildcard_escapes_sql_like_characters() {
        assert_eq!(wildcard("/plain_path"), (false, "/plain_path".into()));
        assert_eq!(wildcard("/docs/*/v2_*"), (true, "/docs/%/v2\\_%".into()));
        assert_eq!(wildcard(r"/a\%*"), (true, r"/a\\\%%".into()));
    }

    #[test]
    fn validates_stats_dates_and_ignores_legacy_pivot_filters() {
        let params = HashMap::from([
            ("startAt".to_string(), "1000".to_string()),
            ("endAt".to_string(), "2000".to_string()),
        ]);
        assert_eq!(
            parse_date_range(&params).unwrap(),
            DateRange {
                start: Utc.timestamp_millis_opt(1000).single().unwrap(),
                end: Utc.timestamp_millis_opt(2000).single().unwrap(),
            }
        );
        let mut filtered = params;
        filtered.insert("browser".to_string(), "Chrome".to_string());
        filtered.insert("pf_country".to_string(), "eq.US".to_string());
        filtered.insert("epf".to_string(), "ignored".to_string());
        filtered.insert("spf".to_string(), "ignored".to_string());
        filtered.retain(|key, _| !key.starts_with("pf_") && key != "epf" && key != "spf");
        assert!(scalar_filter_predicate(&filtered).is_ok());
    }

    #[test]
    fn scalar_filters_apply_at_the_report_scope() {
        let range = DateRange {
            start: Utc.timestamp_millis_opt(1000).single().unwrap(),
            end: Utc.timestamp_millis_opt(2000).single().unwrap(),
        };
        let goal_filters = scalar_filter_predicate(&HashMap::from([
            ("browser".to_string(), "Chrome".to_string()),
            ("eventType".to_string(), "1".to_string()),
        ]))
        .unwrap();
        let goal = goal_query(
            Uuid::nil(),
            range,
            &goal_filters,
            &Audience::default(),
            &GoalParameters {
                action_type: "path".into(),
                value: "/pricing".into(),
            },
        );
        let (matching, total) = goal.sql().split_once("), total AS").unwrap();
        assert!(matching.contains("JOIN session s"));
        assert!(matching.contains("s.browser"));
        assert!(matching.contains("e.event_type"));
        assert!(total.contains("s.browser"));
        assert!(!total.contains("e.event_type"));

        let funnel_filters = scalar_filter_predicate(&HashMap::from([(
            "browser".to_string(),
            "Chrome".to_string(),
        )]))
        .unwrap();
        let funnel = funnel_query(
            Uuid::nil(),
            range,
            &funnel_filters,
            &Audience::default(),
            &FunnelParameters {
                window: 30.0,
                steps: vec![
                    FunnelStep {
                        action_type: "path".into(),
                        value: "/".into(),
                        filters: None,
                    },
                    FunnelStep {
                        action_type: "path".into(),
                        value: "/pricing".into(),
                        filters: None,
                    },
                ],
            },
        );
        assert_eq!(funnel.sql().matches("s.browser").count(), 1);
        assert_eq!(funnel.sql().matches("JOIN session s").count(), 1);
    }

    #[test]
    fn computes_funnel_dropoff_without_inventing_zero_denominators() {
        let steps = vec![
            FunnelStep {
                action_type: "path".into(),
                value: "/".into(),
                filters: None,
            },
            FunnelStep {
                action_type: "event".into(),
                value: "signup".into(),
                filters: None,
            },
        ];
        let result = format_funnel_results(&steps, &[100, 40]);
        assert_eq!(result[0]["remaining"], 1.0);
        assert_eq!(result[0]["dropoff"], Value::Null);
        assert_eq!(result[1]["dropped"], 60);
        assert_eq!(result[1]["dropoff"], 0.6);

        let empty = format_funnel_results(&steps, &[0, 0]);
        assert_eq!(empty[0]["remaining"], Value::Null);
        assert_eq!(empty[1]["dropoff"], Value::Null);
    }
}
