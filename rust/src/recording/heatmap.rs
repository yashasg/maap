use crate::{
    access::{website_pool, Access},
    error_response, json_response, AppState,
};
use axum::{
    body::to_bytes,
    extract::{Path, Query, Request, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::{get, post},
    Router,
};
use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::{PgPool, Postgres, QueryBuilder, Row};
use std::collections::HashMap;
use url::Url;
use uuid::Uuid;

const PAGE_LIMIT: i64 = 100;
const POINT_LIMIT: i64 = 5_000;
const SCROLL_BUCKET_SIZE: i32 = 10;
const MAX_LEGACY_BODY_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy)]
struct DateRange {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Click,
    Scroll,
}

impl Mode {
    fn parse(value: Option<&str>) -> Result<Self, Response> {
        match value.unwrap_or("click") {
            "click" => Ok(Self::Click),
            "scroll" => Ok(Self::Scroll),
            _ => Err(bad_request("Invalid heatmap mode")),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Click => "click",
            Self::Scroll => "scroll",
        }
    }

    fn event_type(self) -> i32 {
        match self {
            Self::Click => 1,
            Self::Scroll => 2,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum FilterTarget {
    Heatmap,
    Event,
    Session,
}

struct Filter {
    name: &'static str,
    column: &'static str,
    operator: &'static str,
    values: Vec<String>,
}

struct Filters {
    visits: Vec<Filter>,
    paths: Vec<Filter>,
    match_any: bool,
    needs_session: bool,
    exclude_bounce: bool,
}

struct HeatmapRequest {
    range: DateRange,
    mode: Mode,
    url_path: Option<String>,
    filters: Filters,
    audience: crate::audience::Audience,
}

struct Page {
    url_path: String,
    count: i32,
    sessions: i32,
}

#[derive(Clone)]
struct Point {
    x: i32,
    y: i32,
    page_x: i32,
    page_y: i32,
    page_w: i32,
    page_h: i32,
    viewport_w: i32,
    viewport_h: i32,
    count: i32,
}

#[derive(Clone)]
struct ScrollBucket {
    depth: i32,
    sessions: i32,
    page_w: i32,
    page_h: i32,
    viewport_w: i32,
    viewport_h: i32,
}

#[derive(Deserialize)]
struct LegacyBody {
    #[serde(rename = "websiteId")]
    website_id: Uuid,
    #[serde(rename = "type")]
    kind: String,
    filters: Value,
    parameters: LegacyParameters,
}

#[derive(Deserialize)]
struct LegacyParameters {
    #[serde(rename = "startDate")]
    start_date: Value,
    #[serde(rename = "endDate")]
    end_date: Value,
    timezone: Option<String>,
    #[serde(rename = "urlPath")]
    url_path: Option<String>,
    mode: Option<String>,
}

pub(super) fn register_routes(router: Router<AppState>) -> Router<AppState> {
    router
        .route("/api/websites/{website_id}/heatmaps", get(website_heatmaps))
        .route("/api/reports/heatmap", post(legacy_heatmap))
}

async fn website_heatmaps(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(mut params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match website_pool(&state, &headers, website_id, Access::Read).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let audience = match crate::audience::Audience::resolve(&pool, website_id, &mut params).await {
        Ok(audience) => audience,
        Err(response) => return response,
    };
    let request = match parse_get_request(&params, audience) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let request = match finish_request(&pool, website_id, &params, request).await {
        Ok(request) => request,
        Err(response) => return response,
    };
    heatmap(&pool, website_id, request).await
}

async fn legacy_heatmap(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request,
) -> Response {
    let bytes = match to_bytes(request.into_body(), MAX_LEGACY_BODY_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => return bad_request("Invalid request body"),
    };
    let body = match serde_json::from_slice::<LegacyBody>(&bytes) {
        Ok(body) if body.kind == "heatmap" => body,
        _ => return bad_request("Invalid request body"),
    };
    if body
        .url_path()
        .is_some_and(|value| value.chars().count() > 500)
    {
        return bad_request("Invalid heatmap path");
    }
    let mut params = match crate::insights::report_filter_params(Some(&body.filters)) {
        Ok(params) => params,
        Err(response) => return response,
    };
    params.remove("timezone");
    if let Some(timezone) = &body.parameters.timezone {
        params.insert("timezone".into(), timezone.clone());
    }
    if let Some(value) = &body.parameters.url_path {
        params.insert("urlPath".to_string(), value.clone());
    }
    if let Some(value) = &body.parameters.mode {
        params.insert("mode".to_string(), value.clone());
    }
    let range = match legacy_range(&body.parameters) {
        Ok(range) => range,
        Err(response) => return response,
    };
    let pool = match website_pool(&state, &headers, body.website_id, Access::Read).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let audience =
        match crate::audience::Audience::resolve(&pool, body.website_id, &mut params).await {
            Ok(audience) => audience,
            Err(response) => return response,
        };
    let request = match parse_common(&params, range, audience) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let request = match finish_request(&pool, body.website_id, &params, request).await {
        Ok(request) => request,
        Err(response) => return response,
    };
    heatmap(&pool, body.website_id, request).await
}

impl LegacyBody {
    fn url_path(&self) -> Option<&str> {
        self.parameters.url_path.as_deref()
    }
}

fn parse_get_request(
    params: &HashMap<String, String>,
    audience: crate::audience::Audience,
) -> Result<HeatmapRequest, Response> {
    let start = parse_millis(params.get("startAt"), "startAt")?;
    let end = parse_millis(params.get("endAt"), "endAt")?;
    if end < start {
        return Err(bad_request(
            "endAt must be greater than or equal to startAt",
        ));
    }
    parse_common(params, DateRange { start, end }, audience)
}

fn parse_common(
    params: &HashMap<String, String>,
    range: DateRange,
    audience: crate::audience::Audience,
) -> Result<HeatmapRequest, Response> {
    validate_params(params)?;
    let mode = Mode::parse(params.get("mode").map(String::as_str))?;
    let url_path = params.get("urlPath").cloned();
    if url_path
        .as_ref()
        .is_some_and(|value| value.chars().count() > 500)
    {
        return Err(bad_request("Invalid heatmap path"));
    }
    Ok(HeatmapRequest {
        audience,
        range,
        mode,
        url_path,
        filters: parse_filters(params)?,
    })
}

async fn finish_request(
    pool: &PgPool,
    website_id: Uuid,
    params: &HashMap<String, String>,
    mut request: HeatmapRequest,
) -> Result<HeatmapRequest, Response> {
    if let Some(timezone) = params.get("timezone") {
        let valid = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM pg_timezone_names WHERE name = $1)",
        )
        .bind(timezone)
        .fetch_one(pool)
        .await
        .map_err(|_| internal_error("Unable to validate timezone"))?;
        if !valid {
            return Err(bad_request("Invalid timezone"));
        }
    }
    if let Some(reset_at) = sqlx::query_scalar::<_, Option<DateTime<Utc>>>(
        "SELECT reset_at FROM website WHERE website_id = $1 AND deleted_at IS NULL",
    )
    .bind(website_id)
    .fetch_optional(pool)
    .await
    .map_err(|_| internal_error("Unable to load website date range"))?
    .flatten()
    {
        request.range.start = request.range.start.max(reset_at);
    }
    Ok(request)
}

fn validate_params(params: &HashMap<String, String>) -> Result<(), Response> {
    for key in params.keys() {
        let base = key.trim_end_matches(|character: char| character.is_ascii_digit());
        if filter_column(base).is_some()
            || matches!(
                key.as_str(),
                "startAt"
                    | "endAt"
                    | "timezone"
                    | "unit"
                    | "urlPath"
                    | "mode"
                    | "testMode"
                    | "match"
                    | "excludeBounce"
            )
        {
            continue;
        }
        return Err(bad_request(format!("Unsupported query parameter: {key}")));
    }
    if params
        .get("match")
        .is_some_and(|value| !matches!(value.as_str(), "all" | "any"))
    {
        return Err(bad_request("Invalid match"));
    }
    if params.get("unit").is_some_and(|value| {
        !matches!(
            value.as_str(),
            "minute" | "hour" | "day" | "week" | "month" | "year"
        )
    }) {
        return Err(bad_request("Invalid unit"));
    }
    Ok(())
}

fn parse_millis(value: Option<&String>, name: &str) -> Result<DateTime<Utc>, Response> {
    let millis = value
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| (0..=8_640_000_000_000_000).contains(value))
        .ok_or_else(|| bad_request(format!("Invalid {name}")))?;
    Utc.timestamp_millis_opt(millis)
        .single()
        .ok_or_else(|| bad_request(format!("Invalid {name}")))
}

fn legacy_range(parameters: &LegacyParameters) -> Result<DateRange, Response> {
    let start =
        parse_json_date(&parameters.start_date).ok_or_else(|| bad_request("Invalid startDate"))?;
    let end =
        parse_json_date(&parameters.end_date).ok_or_else(|| bad_request("Invalid endDate"))?;
    if end < start {
        return Err(bad_request(
            "endDate must be greater than or equal to startDate",
        ));
    }
    Ok(DateRange { start, end })
}

fn parse_json_date(value: &Value) -> Option<DateTime<Utc>> {
    match value {
        Value::String(value) => DateTime::parse_from_rfc3339(value)
            .ok()
            .map(|value| value.with_timezone(&Utc))
            .or_else(|| {
                value
                    .parse::<i64>()
                    .ok()
                    .and_then(|value| Utc.timestamp_millis_opt(value).single())
            }),
        Value::Number(value) => value
            .as_i64()
            .and_then(|value| Utc.timestamp_millis_opt(value).single()),
        _ => None,
    }
}

fn filter_column(name: &str) -> Option<(&'static str, FilterTarget)> {
    Some(match name {
        "path" => ("h.url_path", FilterTarget::Heatmap),
        "referrer" => ("e.referrer_domain", FilterTarget::Event),
        "title" => ("e.page_title", FilterTarget::Event),
        "query" => ("e.url_query", FilterTarget::Event),
        "os" => ("s.os", FilterTarget::Session),
        "browser" => ("s.browser", FilterTarget::Session),
        "device" => ("s.device", FilterTarget::Session),
        "country" => ("s.country", FilterTarget::Session),
        "region" => ("s.region", FilterTarget::Session),
        "city" => ("s.city", FilterTarget::Session),
        "tag" => ("e.tag", FilterTarget::Event),
        "hostname" => ("e.hostname", FilterTarget::Event),
        "distinctId" => ("s.distinct_id", FilterTarget::Session),
        "language" => ("s.language", FilterTarget::Session),
        "event" => ("e.event_name", FilterTarget::Event),
        "utmSource" => ("e.utm_source", FilterTarget::Event),
        "utmMedium" => ("e.utm_medium", FilterTarget::Event),
        "utmCampaign" => ("e.utm_campaign", FilterTarget::Event),
        "utmContent" => ("e.utm_content", FilterTarget::Event),
        "utmTerm" => ("e.utm_term", FilterTarget::Event),
        "eventType" => ("e.event_type", FilterTarget::Event),
        _ => return None,
    })
}

fn parse_filters(params: &HashMap<String, String>) -> Result<Filters, Response> {
    let mut visits = Vec::new();
    let mut paths = Vec::new();
    let mut has_tag = false;
    let mut needs_session = false;
    for (key, value) in params {
        let base = key.trim_end_matches(|character: char| character.is_ascii_digit());
        let Some((column, target)) = filter_column(base) else {
            continue;
        };
        let (operator, raw) = value
            .split_once('.')
            .filter(|(operator, _)| {
                matches!(
                    *operator,
                    "eq" | "neq" | "s" | "ns" | "c" | "dnc" | "re" | "nre"
                )
            })
            .unwrap_or(("eq", value.as_str()));
        let operator = match operator {
            "eq" => "eq",
            "neq" => "neq",
            "s" => "s",
            "ns" => "ns",
            "c" => "c",
            "dnc" => "dnc",
            "re" => "re",
            "nre" => "nre",
            _ => unreachable!(),
        };
        let values = if matches!(operator, "eq" | "neq") {
            raw.split(',').map(str::to_string).collect::<Vec<_>>()
        } else {
            vec![raw.to_string()]
        };
        if base == "eventType"
            && (operator != "eq"
                || values.len() != 1
                || values[0]
                    .parse::<i32>()
                    .ok()
                    .filter(|value| *value > 0)
                    .is_none())
        {
            return Err(bad_request("Invalid event type filter"));
        }
        has_tag |= base == "tag";
        needs_session |= target == FilterTarget::Session;
        let filter = Filter {
            name: match base {
                "eventType" => "eventType",
                "referrer" => "referrer",
                "tag" => "tag",
                _ => "",
            },
            column,
            operator,
            values,
        };
        if target == FilterTarget::Heatmap {
            paths.push(filter);
        } else {
            visits.push(filter);
        }
    }
    if !has_tag {
        visits.push(Filter {
            name: "tag",
            column: "e.tag",
            operator: if params.get("testMode").map(String::as_str) == Some("true") {
                "eq"
            } else {
                "neq"
            },
            values: vec!["test".to_string()],
        });
    }
    Ok(Filters {
        visits,
        paths,
        match_any: params.get("match").map(String::as_str) == Some("any"),
        needs_session,
        exclude_bounce: params
            .get("excludeBounce")
            .is_some_and(|value| !value.is_empty()),
    })
}

async fn heatmap(pool: &PgPool, website_id: Uuid, request: HeatmapRequest) -> Response {
    let pages = match load_pages(pool, website_id, &request).await {
        Ok(pages) => pages,
        Err(response) => return response,
    };
    let page_json = pages
        .into_iter()
        .map(|page| {
            json!({
                "urlPath": page.url_path,
                "count": page.count,
                "sessions": page.sessions
            })
        })
        .collect::<Vec<_>>();
    let Some(url_path) = request.url_path.as_deref() else {
        return json_response(json!({
            "mode": request.mode.name(),
            "pages": page_json,
            "points": [],
            "snapshot": null,
            "scroll": empty_scroll()
        }));
    };
    if request.mode == Mode::Scroll {
        let buckets = match load_scroll(pool, website_id, url_path, &request).await {
            Ok(buckets) => buckets,
            Err(response) => return response,
        };
        let viewport = pick_scroll_viewport(&buckets);
        let snapshot = snapshot(pool, website_id, url_path, viewport).await;
        let total_sessions = buckets
            .iter()
            .map(|bucket| bucket.sessions as i64)
            .sum::<i64>();
        let bucket_json = buckets
            .into_iter()
            .map(|bucket| {
                json!({
                    "depth": bucket.depth,
                    "sessions": bucket.sessions,
                    "pageW": bucket.page_w,
                    "pageH": bucket.page_h,
                    "viewportW": bucket.viewport_w,
                    "viewportH": bucket.viewport_h
                })
            })
            .collect::<Vec<_>>();
        return json_response(json!({
            "mode": "scroll",
            "pages": page_json,
            "points": [],
            "snapshot": snapshot,
            "scroll": {
                "buckets": bucket_json,
                "totalSessions": total_sessions,
                "pageW": viewport.map(|value| value.2),
                "pageH": viewport.map(|value| value.3),
                "viewportW": viewport.map(|value| value.0),
                "viewportH": viewport.map(|value| value.1)
            }
        }));
    }
    let points = match load_points(pool, website_id, url_path, &request).await {
        Ok(points) => points,
        Err(response) => return response,
    };
    let viewport = pick_point_viewport(&points);
    let snapshot = snapshot(pool, website_id, url_path, viewport).await;
    let point_json = points
        .into_iter()
        .map(|point| {
            json!({
                "x": point.x,
                "y": point.y,
                "pageX": point.page_x,
                "pageY": point.page_y,
                "pageW": point.page_w,
                "pageH": point.page_h,
                "viewportW": point.viewport_w,
                "viewportH": point.viewport_h,
                "count": point.count
            })
        })
        .collect::<Vec<_>>();
    json_response(json!({
        "mode": "click",
        "pages": page_json,
        "points": point_json,
        "snapshot": snapshot,
        "scroll": empty_scroll()
    }))
}

async fn load_pages(
    pool: &PgPool,
    website_id: Uuid,
    request: &HeatmapRequest,
) -> Result<Vec<Page>, Response> {
    let mut query = heatmap_query_prefix(website_id, request);
    query
        .push("SELECT h.url_path, count(*)::int AS count, count(DISTINCT h.visit_id)::int AS sessions FROM heatmap_event h JOIN filtered_visits f ON f.website_id = h.website_id AND f.session_id = h.session_id AND f.visit_id = h.visit_id WHERE h.website_id = ")
        .push_bind(website_id)
        .push(" AND h.event_type = ")
        .push_bind(request.mode.event_type())
        .push(" AND h.created_at BETWEEN ")
        .push_bind(request.range.start)
        .push(" AND ")
        .push_bind(request.range.end);
    push_path_filters(&mut query, &request.filters);
    if request.mode == Mode::Click {
        query.push(" AND h.x IS NOT NULL AND h.y IS NOT NULL AND h.page_x IS NOT NULL AND h.page_y IS NOT NULL AND h.page_w IS NOT NULL AND h.page_h IS NOT NULL AND h.viewport_w IS NOT NULL AND h.viewport_h IS NOT NULL");
    } else {
        query.push(" AND h.scroll_pct IS NOT NULL AND h.page_w IS NOT NULL AND h.page_h IS NOT NULL AND h.viewport_w IS NOT NULL AND h.viewport_h IS NOT NULL");
    }
    query
        .push(" GROUP BY h.url_path ORDER BY sessions DESC, count DESC LIMIT ")
        .push_bind(PAGE_LIMIT);
    query
        .build()
        .fetch_all(pool)
        .await
        .map(|rows| {
            rows.into_iter()
                .map(|row| Page {
                    url_path: row.get("url_path"),
                    count: row.get("count"),
                    sessions: row.get("sessions"),
                })
                .collect()
        })
        .map_err(|_| internal_error("Unable to load heatmap pages"))
}

async fn load_points(
    pool: &PgPool,
    website_id: Uuid,
    url_path: &str,
    request: &HeatmapRequest,
) -> Result<Vec<Point>, Response> {
    let mut query = heatmap_query_prefix(website_id, request);
    query
        .push("SELECT h.x, h.y, h.page_x, h.page_y, h.page_w, h.page_h, h.viewport_w, h.viewport_h, count(*)::int AS count FROM heatmap_event h JOIN filtered_visits f ON f.website_id = h.website_id AND f.session_id = h.session_id AND f.visit_id = h.visit_id WHERE h.website_id = ")
        .push_bind(website_id)
        .push(" AND h.event_type = 1 AND h.url_path = ")
        .push_bind(url_path.to_string())
        .push(" AND h.created_at BETWEEN ")
        .push_bind(request.range.start)
        .push(" AND ")
        .push_bind(request.range.end);
    push_path_filters(&mut query, &request.filters);
    query.push(" AND h.x IS NOT NULL AND h.y IS NOT NULL AND h.page_x IS NOT NULL AND h.page_y IS NOT NULL AND h.page_w IS NOT NULL AND h.page_h IS NOT NULL AND h.viewport_w IS NOT NULL AND h.viewport_h IS NOT NULL GROUP BY h.x, h.y, h.page_x, h.page_y, h.page_w, h.page_h, h.viewport_w, h.viewport_h ORDER BY count DESC LIMIT ").push_bind(POINT_LIMIT);
    query
        .build()
        .fetch_all(pool)
        .await
        .map(|rows| {
            rows.into_iter()
                .map(|row| Point {
                    x: row.get("x"),
                    y: row.get("y"),
                    page_x: row.get("page_x"),
                    page_y: row.get("page_y"),
                    page_w: row.get("page_w"),
                    page_h: row.get("page_h"),
                    viewport_w: row.get("viewport_w"),
                    viewport_h: row.get("viewport_h"),
                    count: row.get("count"),
                })
                .collect()
        })
        .map_err(|_| internal_error("Unable to load heatmap points"))
}

async fn load_scroll(
    pool: &PgPool,
    website_id: Uuid,
    url_path: &str,
    request: &HeatmapRequest,
) -> Result<Vec<ScrollBucket>, Response> {
    let mut query = heatmap_query_prefix(website_id, request);
    query
        .push("SELECT (floor(max_pct / ")
        .push_bind(SCROLL_BUCKET_SIZE)
        .push(") * ")
        .push_bind(SCROLL_BUCKET_SIZE)
        .push(")::int AS depth, count(*)::int AS sessions, page_w::int, page_h::int, viewport_w::int, viewport_h::int FROM (SELECT h.visit_id, max(h.scroll_pct) AS max_pct, (mode() WITHIN GROUP (ORDER BY h.page_w))::int AS page_w, (mode() WITHIN GROUP (ORDER BY h.page_h))::int AS page_h, (mode() WITHIN GROUP (ORDER BY h.viewport_w))::int AS viewport_w, (mode() WITHIN GROUP (ORDER BY h.viewport_h))::int AS viewport_h FROM heatmap_event h JOIN filtered_visits f ON f.website_id = h.website_id AND f.session_id = h.session_id AND f.visit_id = h.visit_id WHERE h.website_id = ")
        .push_bind(website_id)
        .push(" AND h.event_type = 2 AND h.url_path = ")
        .push_bind(url_path.to_string())
        .push(" AND h.created_at BETWEEN ")
        .push_bind(request.range.start)
        .push(" AND ")
        .push_bind(request.range.end);
    push_path_filters(&mut query, &request.filters);
    query.push(" AND h.scroll_pct IS NOT NULL AND h.page_w IS NOT NULL AND h.page_h IS NOT NULL AND h.viewport_w IS NOT NULL AND h.viewport_h IS NOT NULL GROUP BY h.visit_id) per_session GROUP BY depth, page_w, page_h, viewport_w, viewport_h ORDER BY depth");
    query
        .build()
        .fetch_all(pool)
        .await
        .map(|rows| {
            rows.into_iter()
                .map(|row| ScrollBucket {
                    depth: row.get("depth"),
                    sessions: row.get("sessions"),
                    page_w: row.get("page_w"),
                    page_h: row.get("page_h"),
                    viewport_w: row.get("viewport_w"),
                    viewport_h: row.get("viewport_h"),
                })
                .collect()
        })
        .map_err(|_| internal_error("Unable to load heatmap scroll data"))
}

fn heatmap_query_prefix(
    website_id: Uuid,
    request: &HeatmapRequest,
) -> QueryBuilder<'static, Postgres> {
    let mut query = QueryBuilder::new(
        "WITH filtered_visits AS (SELECT DISTINCT e.website_id, e.session_id, e.visit_id FROM website_event e ",
    );
    if request.filters.needs_session {
        query
            .push("JOIN session s ON s.session_id = e.session_id AND s.website_id = e.website_id ");
    }
    query
        .push("WHERE e.website_id = ")
        .push_bind(website_id)
        .push(" AND e.created_at BETWEEN ")
        .push_bind(request.range.start)
        .push(" AND ")
        .push_bind(request.range.end);
    push_visit_filters(&mut query, &request.filters);
    request
        .audience
        .push(&mut query, "e", request.range.start, request.range.end);
    if request.filters.exclude_bounce {
        query
            .push(" AND EXISTS (SELECT 1 FROM website_event bx WHERE bx.website_id = e.website_id AND bx.session_id = e.session_id AND bx.visit_id = e.visit_id AND bx.created_at BETWEEN ")
            .push_bind(request.range.start)
            .push(" AND ")
            .push_bind(request.range.end)
            .push(" AND bx.event_type != 5 GROUP BY bx.session_id, bx.visit_id HAVING count(*) FILTER (WHERE bx.event_type NOT IN (2, 5)) > 1 OR (count(*) FILTER (WHERE bx.event_type NOT IN (2, 5)) = 1 AND count(*) FILTER (WHERE bx.event_type = 2) > 0))");
    }
    query.push(") ");
    query
}

fn push_visit_filters(query: &mut QueryBuilder<'_, Postgres>, filters: &Filters) {
    push_filter_set(query, &filters.visits, filters.match_any);
    if filters
        .visits
        .iter()
        .any(|filter| filter.name == "referrer")
    {
        query.push(" AND (e.referrer_domain != regexp_replace(e.hostname, '^www.', '') OR e.referrer_domain IS NULL)");
    }
}

fn push_path_filters(query: &mut QueryBuilder<'_, Postgres>, filters: &Filters) {
    push_filter_set(query, &filters.paths, filters.match_any);
}

fn push_filter_set(query: &mut QueryBuilder<'_, Postgres>, filters: &[Filter], match_any: bool) {
    let (always, matched): (Vec<_>, Vec<_>) = filters
        .iter()
        .partition(|filter| filter.name == "eventType" || !match_any);
    if !matched.is_empty() {
        query.push(" AND (");
        for (index, filter) in matched.into_iter().enumerate() {
            if index > 0 {
                query.push(" OR ");
            }
            push_filter(query, filter);
        }
        query.push(")");
    }
    for filter in always {
        query.push(" AND ");
        push_filter(query, filter);
    }
}

fn push_filter(query: &mut QueryBuilder<'_, Postgres>, filter: &Filter) {
    if filter.name == "eventType" {
        query.push(filter.column).push(" = ").push_bind(
            filter.values[0]
                .parse::<i32>()
                .expect("validated event type"),
        );
        return;
    }
    match filter.operator {
        "eq" => {
            query
                .push(filter.column)
                .push(" = ANY(")
                .push_bind(filter.values.clone())
                .push(")");
        }
        "neq" => {
            query
                .push("COALESCE(")
                .push(filter.column)
                .push(", '') != ALL(")
                .push_bind(filter.values.clone())
                .push(")");
        }
        "s" => {
            query
                .push(filter.column)
                .push(" ILIKE ")
                .push_bind(format!("{}%", filter.values[0]));
        }
        "ns" => {
            query
                .push("COALESCE(")
                .push(filter.column)
                .push(", '') NOT ILIKE ")
                .push_bind(format!("{}%", filter.values[0]));
        }
        "c" => {
            query
                .push(filter.column)
                .push(" ILIKE ")
                .push_bind(format!("%{}%", filter.values[0]));
        }
        "dnc" => {
            query
                .push("COALESCE(")
                .push(filter.column)
                .push(", '') NOT ILIKE ")
                .push_bind(format!("%{}%", filter.values[0]));
        }
        "re" => {
            query
                .push(filter.column)
                .push(" ~* ")
                .push_bind(filter.values[0].clone());
        }
        "nre" => {
            query
                .push("COALESCE(")
                .push(filter.column)
                .push(", '') !~* ")
                .push_bind(filter.values[0].clone());
        }
        _ => unreachable!(),
    }
}

fn empty_scroll() -> Value {
    json!({
        "buckets": [],
        "totalSessions": 0,
        "pageW": null,
        "pageH": null,
        "viewportW": null,
        "viewportH": null
    })
}

fn pick_point_viewport(points: &[Point]) -> Option<(i32, i32, i32, i32)> {
    struct Bucket {
        width: i32,
        height: i32,
        count: i64,
        max_page_w: i32,
        max_page_y: i32,
        page_heights: HashMap<i32, i64>,
    }
    let mut buckets = Vec::<Bucket>::new();
    let mut indexes = HashMap::<(i32, i32), usize>::new();
    for point in points {
        let index = *indexes
            .entry((point.viewport_w, point.viewport_h))
            .or_insert_with(|| {
                buckets.push(Bucket {
                    width: point.viewport_w,
                    height: point.viewport_h,
                    count: 0,
                    max_page_w: point.page_w,
                    max_page_y: point.page_y,
                    page_heights: HashMap::new(),
                });
                buckets.len() - 1
            });
        let bucket = &mut buckets[index];
        bucket.count += point.count as i64;
        bucket.max_page_w = bucket.max_page_w.max(point.page_w);
        bucket.max_page_y = bucket.max_page_y.max(point.page_y);
        *bucket.page_heights.entry(point.page_h).or_default() += point.count as i64;
    }
    let best = buckets.iter().max_by_key(|bucket| bucket.count)?;
    let common_height = best
        .page_heights
        .iter()
        .max_by(|(left_height, left_count), (right_height, right_count)| {
            left_count
                .cmp(right_count)
                .then_with(|| left_height.cmp(right_height))
        })
        .map(|(height, _)| *height)
        .unwrap_or(0);
    Some((
        best.width,
        best.height,
        best.max_page_w,
        common_height.max(best.max_page_y),
    ))
}

fn pick_scroll_viewport(buckets: &[ScrollBucket]) -> Option<(i32, i32, i32, i32)> {
    struct Viewport {
        width: i32,
        height: i32,
        sessions: i64,
        max_page_w: i32,
        max_page_h: i32,
    }
    let mut viewports = Vec::<Viewport>::new();
    let mut indexes = HashMap::<(i32, i32), usize>::new();
    for bucket in buckets {
        let index = *indexes
            .entry((bucket.viewport_w, bucket.viewport_h))
            .or_insert_with(|| {
                viewports.push(Viewport {
                    width: bucket.viewport_w,
                    height: bucket.viewport_h,
                    sessions: 0,
                    max_page_w: bucket.page_w,
                    max_page_h: bucket.page_h,
                });
                viewports.len() - 1
            });
        let viewport = &mut viewports[index];
        viewport.sessions += bucket.sessions as i64;
        viewport.max_page_w = viewport.max_page_w.max(bucket.page_w);
        viewport.max_page_h = viewport.max_page_h.max(bucket.page_h);
    }
    viewports
        .iter()
        .max_by_key(|viewport| viewport.sessions)
        .map(|viewport| {
            (
                viewport.width,
                viewport.height,
                viewport.max_page_w,
                viewport.max_page_h,
            )
        })
}

async fn snapshot(
    pool: &PgPool,
    website_id: Uuid,
    url_path: &str,
    viewport: Option<(i32, i32, i32, i32)>,
) -> Value {
    let Some((viewport_w, viewport_h, page_w, page_h)) = viewport else {
        return Value::Null;
    };
    if viewport_w == 0 || page_w == 0 || page_h == 0 {
        return Value::Null;
    }
    let domain = sqlx::query_scalar::<_, Option<String>>(
        "SELECT domain FROM website WHERE website_id = $1 AND deleted_at IS NULL",
    )
    .bind(website_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .flatten();
    let Some(url) = domain.and_then(|domain| build_page_url(&domain, url_path)) else {
        return Value::Null;
    };
    let viewport_h = if viewport_h != 0 {
        viewport_h
    } else {
        page_h.clamp(640, 1080)
    };
    json!({
        "kind": "iframe",
        "id": format!("iframe:{website_id}:{url_path}:{viewport_w}x{viewport_h}"),
        "url": url,
        "pageW": page_w,
        "pageH": page_h,
        "viewportW": viewport_w,
        "viewportH": viewport_h
    })
}

fn build_page_url(domain: &str, path: &str) -> Option<String> {
    let host = domain.split(',').next()?.trim();
    if host.is_empty() {
        return None;
    }
    let origin = if host.starts_with("http://") || host.starts_with("https://") {
        Url::parse(host).ok()?
    } else {
        let protocol = if host.starts_with("localhost")
            || host.starts_with("127.0.0.1")
            || host.starts_with("[::1]")
        {
            "http"
        } else {
            "https"
        };
        Url::parse(&format!("{protocol}://{host}")).ok()?
    };
    let normalized = format!("/{}", path.replace('\\', "/").trim_start_matches('/'));
    let resolved = origin.join(&normalized).ok()?;
    if resolved.origin() != origin.origin() {
        return None;
    }
    Some(resolved.into())
}

fn bad_request(message: impl Into<String>) -> Response {
    error_response(StatusCode::BAD_REQUEST, message)
}

fn internal_error(message: impl Into<String>) -> Response {
    error_response(StatusCode::INTERNAL_SERVER_ERROR, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_filter_excludes_test_mode() {
        let filters = parse_filters(&HashMap::new()).unwrap();
        let tag = filters
            .visits
            .iter()
            .find(|filter| filter.name == "tag")
            .unwrap();
        assert_eq!(tag.operator, "neq");
        assert_eq!(tag.values, ["test"]);
    }

    #[test]
    fn page_url_matches_node_origin_rules() {
        assert_eq!(
            build_page_url("example.com,other.example", "/pricing"),
            Some("https://example.com/pricing".to_string())
        );
        assert_eq!(
            build_page_url("localhost:3000", "/"),
            Some("http://localhost:3000/".to_string())
        );
        assert_eq!(
            build_page_url("example.com", "//attacker.example/phish"),
            Some("https://example.com/attacker.example/phish".to_string())
        );
        assert_eq!(
            build_page_url("example.com", "/\\attacker.example/phish"),
            Some("https://example.com/attacker.example/phish".to_string())
        );
    }

    #[test]
    fn heatmap_path_filters_are_separate_from_visit_filters() {
        let filters = parse_filters(&HashMap::from([
            ("path".to_string(), "c.pricing".to_string()),
            ("browser".to_string(), "chrome".to_string()),
        ]))
        .unwrap();
        assert_eq!(filters.paths.len(), 1);
        assert!(filters.needs_session);
    }
}
