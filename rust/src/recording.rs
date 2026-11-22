use crate::{
    access::{website_pool, Access},
    collection::{extract_client_ip, is_allowed_collection_origin, is_blocked_ip, is_bot},
    error_response, json_response, secret_hash, with_rust_header, AppState,
};
use axum::{
    body::to_bytes,
    extract::{Path, Query, Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use chrono::{DateTime, Duration, TimeZone, Utc};
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sqlx::{PgPool, Postgres, QueryBuilder, Row};
use std::{
    collections::{BTreeSet, HashMap},
    io::{Read, Write},
};
use url::Url;
use uuid::Uuid;

mod heatmap;

const MAX_RECORD_REQUEST_BYTES: usize = 1_000_000;
const MAX_SAVED_REQUEST_BYTES: usize = 16_384;
const MAX_REPLAY_BYTES: usize = 16 * 1024 * 1024;
const MAX_EVENTS: usize = 200;

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum RecordEnvelope {
    #[serde(rename = "record")]
    Record { payload: ReplayPayload },
    #[serde(rename = "heatmap")]
    Heatmap { payload: HeatmapPayload },
}

#[derive(Debug, Deserialize)]
struct ReplayPayload {
    website: Uuid,
    events: Vec<Value>,
    timestamp: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct HeatmapPayload {
    website: Uuid,
    events: Vec<HeatmapInput>,
    timestamp: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum HeatmapInput {
    #[serde(rename = "click")]
    Click {
        url: String,
        x: Option<f64>,
        y: Option<f64>,
        #[serde(rename = "pageX")]
        page_x: Option<f64>,
        #[serde(rename = "pageY")]
        page_y: Option<f64>,
        #[serde(rename = "pageW")]
        page_w: Option<f64>,
        #[serde(rename = "pageH")]
        page_h: Option<f64>,
        #[serde(rename = "viewportW")]
        viewport_w: Option<f64>,
        #[serde(rename = "viewportH")]
        viewport_h: Option<f64>,
        timestamp: Option<i64>,
    },
    #[serde(rename = "scroll")]
    Scroll {
        url: String,
        #[serde(rename = "scrollPct")]
        scroll_pct: Option<f64>,
        #[serde(rename = "pageW")]
        page_w: Option<f64>,
        #[serde(rename = "pageH")]
        page_h: Option<f64>,
        #[serde(rename = "viewportW")]
        viewport_w: Option<f64>,
        #[serde(rename = "viewportH")]
        viewport_h: Option<f64>,
        timestamp: Option<i64>,
    },
}

#[derive(Debug)]
struct CacheClaims {
    session_id: Uuid,
    visit_id: Uuid,
}

#[derive(Debug)]
struct WebsiteRecorder {
    enabled: bool,
    domain: Option<String>,
    config: Value,
}

#[derive(Debug)]
struct HeatmapRow {
    event_type: i32,
    url_path: String,
    x: Option<i32>,
    y: Option<i32>,
    page_x: Option<i32>,
    page_y: Option<i32>,
    page_w: Option<i32>,
    page_h: Option<i32>,
    viewport_w: Option<i32>,
    viewport_h: Option<i32>,
    scroll_pct: Option<i32>,
    created_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug)]
struct DateRange {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
}

#[derive(Debug)]
struct ReplayPage {
    page: i64,
    page_size: i64,
    range: DateRange,
    search: Option<String>,
    min_duration_ms: Option<i64>,
}

#[derive(Debug)]
struct ReplayChunk {
    session_id: Uuid,
    chunk_index: i32,
    events: Vec<Value>,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
}

pub(super) fn register_routes(router: Router<AppState>) -> Router<AppState> {
    let router = router
        .route("/api/record", post(record).options(cors_preflight))
        .route(
            "/api/websites/{website_id}/recorder",
            get(recorder_config).options(cors_preflight),
        )
        .route("/api/websites/{website_id}/replays", get(list_replays))
        .route(
            "/api/websites/{website_id}/replays/saved",
            get(list_saved_replays),
        )
        .route(
            "/api/websites/{website_id}/replays/saved/{replay_id}",
            get(saved_replay_status).post(set_saved_replay),
        )
        .route(
            "/api/websites/{website_id}/replays/{replay_id}",
            get(replay_detail),
        )
        .route(
            "/api/websites/{website_id}/sessions/{session_id}/replays",
            get(list_session_replays),
        );
    heatmap::register_routes(router)
}

async fn cors_preflight() -> Response {
    with_cors(with_rust_header(StatusCode::NO_CONTENT.into_response()))
}

fn with_cors(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Content-Type, x-umami-cache"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    headers.insert(
        header::ACCESS_CONTROL_MAX_AGE,
        HeaderValue::from_static("86400"),
    );
    response
}

fn cors_error(status: StatusCode, message: impl Into<String>) -> Response {
    with_cors(error_response(status, message))
}

fn payload_too_large(size: Option<usize>) -> Response {
    let mut error = Map::from_iter([
        ("message".to_string(), json!("Payload too large")),
        ("code".to_string(), json!("payload-too-large")),
        ("status".to_string(), json!(413)),
        ("reason".to_string(), json!("payload_too_large")),
        ("maxBytes".to_string(), json!(MAX_RECORD_REQUEST_BYTES)),
    ]);
    if let Some(size) = size {
        error.insert("size".to_string(), json!(size));
    }
    with_cors(with_rust_header(
        (
            StatusCode::PAYLOAD_TOO_LARGE,
            axum::Json(json!({ "error": error })),
        )
            .into_response(),
    ))
}

fn validate_chunk_timestamp(timestamp: Option<i64>) -> Result<(), Response> {
    if timestamp.is_some_and(|value| !(0..=i32::MAX as i64).contains(&value)) {
        return Err(cors_error(StatusCode::BAD_REQUEST, "Invalid timestamp."));
    }
    Ok(())
}

fn parse_cache(headers: &HeaderMap, website_id: Uuid) -> Option<CacheClaims> {
    parse_cache_with_secret(headers, website_id, secret_hash().as_bytes())
}

fn parse_cache_with_secret(
    headers: &HeaderMap,
    website_id: Uuid,
    secret: &[u8],
) -> Option<CacheClaims> {
    let token = headers.get("x-umami-cache")?.to_str().ok()?;
    let mut validation = Validation::new(Algorithm::HS256);
    validation.required_spec_claims.clear();
    let claims = decode::<Value>(token, &DecodingKey::from_secret(secret), &validation)
        .ok()?
        .claims;
    let claims = claims.as_object()?;
    if claims.get("type")?.as_str()? != "cache"
        || Uuid::parse_str(claims.get("websiteId")?.as_str()?).ok()? != website_id
        || claims.get("iat")?.as_i64().is_none()
    {
        return None;
    }
    Some(CacheClaims {
        session_id: Uuid::parse_str(claims.get("sessionId")?.as_str()?).ok()?,
        visit_id: Uuid::parse_str(claims.get("visitId")?.as_str()?).ok()?,
    })
}

async fn load_recorder(
    pool: &PgPool,
    website_id: Uuid,
) -> Result<Option<WebsiteRecorder>, Response> {
    let row = sqlx::query(
        r#"
        SELECT recorder_enabled, domain, replay_config::text AS replay_config
        FROM website
        WHERE website_id = $1 AND deleted_at IS NULL
        "#,
    )
    .bind(website_id)
    .fetch_optional(pool)
    .await
    .map_err(|_| error_response(StatusCode::INTERNAL_SERVER_ERROR, "Unable to load website"))?;
    Ok(row.map(|row| WebsiteRecorder {
        enabled: row.get("recorder_enabled"),
        domain: row.get("domain"),
        config: row
            .get::<Option<String>, _>("replay_config")
            .and_then(|value| serde_json::from_str(&value).ok())
            .unwrap_or(Value::Null),
    }))
}

fn config_bool(config: &Value, key: &str) -> bool {
    config
        .as_object()
        .and_then(|value| value.get(key))
        .and_then(Value::as_bool)
        == Some(true)
}

async fn record(State(state): State<AppState>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let content_length = parts
        .headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok());
    if content_length.is_some_and(|size| size > MAX_RECORD_REQUEST_BYTES) {
        return payload_too_large(content_length);
    }
    let bytes = match to_bytes(body, MAX_RECORD_REQUEST_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => return payload_too_large(content_length),
    };
    let envelope = match serde_json::from_slice::<RecordEnvelope>(&bytes) {
        Ok(envelope) => envelope,
        Err(_) => return cors_error(StatusCode::BAD_REQUEST, "Invalid request body"),
    };
    let (website_id, event_count, timestamp) = match &envelope {
        RecordEnvelope::Record { payload } => {
            (payload.website, payload.events.len(), payload.timestamp)
        }
        RecordEnvelope::Heatmap { payload } => {
            (payload.website, payload.events.len(), payload.timestamp)
        }
    };
    if event_count == 0 {
        return cors_error(StatusCode::BAD_REQUEST, "Events are required.");
    }
    if event_count > MAX_EVENTS {
        return cors_error(StatusCode::BAD_REQUEST, "Too many events.");
    }
    if let Err(response) = validate_chunk_timestamp(timestamp) {
        return response;
    }
    let Some(cache_header) = parts.headers.get("x-umami-cache") else {
        return cors_error(StatusCode::BAD_REQUEST, "Missing session token.");
    };
    if cache_header.to_str().is_err() {
        return cors_error(StatusCode::BAD_REQUEST, "Invalid session token.");
    }
    let Some(cache) = parse_cache(&parts.headers, website_id) else {
        return cors_error(StatusCode::BAD_REQUEST, "Invalid session token.");
    };
    let Some(pool) = state.db else {
        return cors_error(StatusCode::SERVICE_UNAVAILABLE, "Database unavailable");
    };
    let website = match load_recorder(&pool, website_id).await {
        Ok(Some(website)) => website,
        Ok(None) => return cors_error(StatusCode::BAD_REQUEST, "Website not found."),
        Err(response) => return with_cors(response),
    };
    if !is_allowed_collection_origin(&parts.headers, website.domain.as_deref(), None, None) {
        return cors_error(
            StatusCode::FORBIDDEN,
            "Origin does not match the registered website domain.",
        );
    }
    if !website.enabled {
        return with_cors(json_response(json!({
            "ok": false,
            "reason": "recorder_disabled"
        })));
    }
    let user_agent = parts
        .headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if is_bot(user_agent) {
        return with_cors(json_response(json!({ "beep": "boop" })));
    }
    if is_blocked_ip(&extract_client_ip(&parts.headers)) {
        return cors_error(StatusCode::FORBIDDEN, "Forbidden");
    }
    match envelope {
        RecordEnvelope::Record { payload } => {
            if !config_bool(&website.config, "replayEnabled") {
                return with_cors(json_response(json!({
                    "ok": false,
                    "reason": "replay_disabled"
                })));
            }
            match save_replay(&pool, cache, payload).await {
                Ok(()) => with_cors(json_response(json!({ "ok": true }))),
                Err(response) => with_cors(response),
            }
        }
        RecordEnvelope::Heatmap { payload } => {
            if !config_bool(&website.config, "heatmapEnabled") {
                return with_cors(json_response(json!({
                    "ok": false,
                    "reason": "heatmap_disabled"
                })));
            }
            match save_heatmap(&pool, website_id, cache, payload).await {
                Ok(()) => with_cors(json_response(json!({ "ok": true }))),
                Err(response) => with_cors(response),
            }
        }
    }
}

async fn save_replay(
    pool: &PgPool,
    cache: CacheClaims,
    payload: ReplayPayload,
) -> Result<(), Response> {
    let now = Utc::now();
    let fallback_ms = payload
        .timestamp
        .filter(|value| *value != 0)
        .unwrap_or_else(|| now.timestamp())
        * 1000;
    let mut timestamps = Vec::new();
    for event in &payload.events {
        if let Some(timestamp) = event_timestamp(event).filter(|value| *value > 0.0) {
            if timestamp > i64::MAX as f64
                || Utc
                    .timestamp_millis_opt(timestamp as i64)
                    .single()
                    .is_none()
            {
                return Err(error_response(
                    StatusCode::BAD_REQUEST,
                    "Invalid event timestamp.",
                ));
            }
            timestamps.push(timestamp as i64);
        }
    }
    let min_timestamp = timestamps.iter().copied().min().unwrap_or(fallback_ms);
    let max_timestamp = timestamps.iter().copied().max().unwrap_or(fallback_ms);
    let Some(started_at) = Utc.timestamp_millis_opt(min_timestamp).single() else {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "Invalid event timestamp.",
        ));
    };
    let Some(ended_at) = Utc.timestamp_millis_opt(max_timestamp).single() else {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "Invalid event timestamp.",
        ));
    };
    let chunk_index = payload
        .timestamp
        .filter(|value| *value != 0)
        .unwrap_or_else(|| now.timestamp()) as i32;
    let event_count = replay_event_count(&payload.events);
    let serialized = serde_json::to_vec(&payload.events).map_err(|_| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unable to encode replay events",
        )
    })?;
    let compressed = gzip_encode(&serialized).map_err(|_| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unable to compress replay events",
        )
    })?;
    let result = sqlx::query(
        r#"
        INSERT INTO session_replay (
            replay_id, website_id, session_id, visit_id, chunk_index,
            events, event_count, started_at, ended_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(payload.website)
    .bind(cache.session_id)
    .bind(cache.visit_id)
    .bind(chunk_index)
    .bind(compressed)
    .bind(event_count as i32)
    .bind(started_at)
    .bind(ended_at)
    .execute(pool)
    .await
    .map_err(|_| error_response(StatusCode::INTERNAL_SERVER_ERROR, "Unable to save replay"))?;
    if result.rows_affected() != 1 {
        return Err(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Replay was not saved",
        ));
    }
    Ok(())
}

async fn save_heatmap(
    pool: &PgPool,
    website_id: Uuid,
    cache: CacheClaims,
    payload: HeatmapPayload,
) -> Result<(), Response> {
    let fallback_ms = payload
        .timestamp
        .filter(|value| *value != 0)
        .unwrap_or_else(|| Utc::now().timestamp())
        * 1000;
    let rows = payload
        .events
        .into_iter()
        .map(|event| heatmap_row(event, fallback_ms))
        .collect::<Result<Vec<_>, _>>()?;
    let mut builder = QueryBuilder::<Postgres>::new(
        r#"
        INSERT INTO heatmap_event (
            heatmap_event_id, website_id, session_id, visit_id, url_path,
            event_type, x, y, page_x, page_y, page_w, viewport_w,
            viewport_h, page_h, scroll_pct, created_at
        )
        "#,
    );
    builder.push_values(rows.iter(), |mut values, row| {
        values
            .push_bind(Uuid::new_v4())
            .push_bind(website_id)
            .push_bind(cache.session_id)
            .push_bind(cache.visit_id)
            .push_bind(&row.url_path)
            .push_bind(row.event_type)
            .push_bind(row.x)
            .push_bind(row.y)
            .push_bind(row.page_x)
            .push_bind(row.page_y)
            .push_bind(row.page_w)
            .push_bind(row.viewport_w)
            .push_bind(row.viewport_h)
            .push_bind(row.page_h)
            .push_bind(row.scroll_pct)
            .push_bind(row.created_at);
    });
    let result = builder.build().execute(pool).await.map_err(|_| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unable to save heatmap events",
        )
    })?;
    if result.rows_affected() != rows.len() as u64 {
        return Err(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Heatmap events were not saved",
        ));
    }
    Ok(())
}

fn heatmap_row(event: HeatmapInput, fallback_ms: i64) -> Result<HeatmapRow, Response> {
    let (
        event_type,
        url,
        x,
        y,
        page_x,
        page_y,
        page_w,
        page_h,
        viewport_w,
        viewport_h,
        scroll_pct,
        timestamp,
    ) = match event {
        HeatmapInput::Click {
            url,
            x,
            y,
            page_x,
            page_y,
            page_w,
            page_h,
            viewport_w,
            viewport_h,
            timestamp,
        } => (
            1,
            url,
            finite_i32(x)?,
            finite_i32(y)?,
            finite_i32(page_x)?,
            finite_i32(page_y)?,
            finite_i32(page_w)?,
            finite_i32(page_h)?,
            finite_i32(viewport_w)?,
            finite_i32(viewport_h)?,
            None,
            timestamp,
        ),
        HeatmapInput::Scroll {
            url,
            scroll_pct,
            page_w,
            page_h,
            viewport_w,
            viewport_h,
            timestamp,
        } => (
            2,
            url,
            None,
            None,
            None,
            None,
            finite_i32(page_w)?,
            finite_i32(page_h)?,
            finite_i32(viewport_w)?,
            finite_i32(viewport_h)?,
            finite_i32(scroll_pct)?.map(|value| value.clamp(0, 100)),
            timestamp,
        ),
    };
    let url_path = url_path(&url);
    if url_path.chars().count() > 500 {
        return Err(error_response(StatusCode::BAD_REQUEST, "URL is too long."));
    }
    let millis = timestamp.unwrap_or(fallback_ms);
    let Some(created_at) = Utc.timestamp_millis_opt(millis).single() else {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "Invalid event timestamp.",
        ));
    };
    Ok(HeatmapRow {
        event_type,
        url_path,
        x,
        y,
        page_x,
        page_y,
        page_w,
        page_h,
        viewport_w,
        viewport_h,
        scroll_pct,
        created_at,
    })
}

fn finite_i32(value: Option<f64>) -> Result<Option<i32>, Response> {
    let Some(value) = value else {
        return Ok(None);
    };
    if !value.is_finite() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "Invalid coordinate.",
        ));
    }
    let rounded = (value + 0.5).floor();
    if rounded < i32::MIN as f64 || rounded > i32::MAX as f64 {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "Invalid coordinate.",
        ));
    }
    Ok(Some(rounded as i32))
}

fn url_path(value: &str) -> String {
    if let Ok(url) = Url::parse(value) {
        let path = url.path();
        return if path.is_empty() { "/" } else { path }.to_string();
    }
    if value.starts_with('/') {
        return value
            .split(['?', '#'])
            .next()
            .filter(|path| !path.is_empty())
            .unwrap_or("/")
            .to_string();
    }
    "/".to_string()
}

fn event_timestamp(event: &Value) -> Option<f64> {
    let value = event.as_object()?.get("timestamp")?;
    value
        .as_f64()
        .or_else(|| value.as_str()?.parse::<f64>().ok())
        .filter(|value| value.is_finite())
}

fn replay_event_count(events: &[Value]) -> usize {
    events
        .iter()
        .filter(|event| {
            fragment(event)
                .map(|fragment| fragment.index == 0)
                .unwrap_or(true)
        })
        .count()
}

async fn recorder_config(State(state): State<AppState>, Path(website_id): Path<Uuid>) -> Response {
    let Some(pool) = state.db else {
        return cors_error(StatusCode::SERVICE_UNAVAILABLE, "Database unavailable");
    };
    let website = match load_recorder(&pool, website_id).await {
        Ok(website) => website,
        Err(response) => return with_cors(response),
    };
    let mut response = match website {
        None => json_response(json!({ "enabled": false })),
        Some(website) if !website.enabled => json_response(json!({ "enabled": false })),
        Some(website) => {
            let config = website.config.as_object();
            json_response(json!({
                "enabled": true,
                "replayEnabled": config_bool(&website.config, "replayEnabled"),
                "heatmapEnabled": config_bool(&website.config, "heatmapEnabled"),
                "sampleRate": config.and_then(|value| value.get("sampleRate")).and_then(Value::as_f64).unwrap_or(0.15),
                "heatmapSampleRate": config.and_then(|value| value.get("heatmapSampleRate")).and_then(Value::as_f64).unwrap_or(0.15),
                "maskLevel": config.and_then(|value| value.get("maskLevel")).and_then(Value::as_str).filter(|value| matches!(*value, "strict" | "moderate")).unwrap_or("moderate"),
                "maxDuration": config.and_then(|value| value.get("maxDuration")).and_then(Value::as_f64).map(|value| (value + 0.5).floor() as i64).unwrap_or(300000),
                "blockSelector": config.and_then(|value| value.get("blockSelector")).and_then(Value::as_str).unwrap_or("")
            }))
        }
    };
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=60, stale-while-revalidate=300"),
    );
    with_cors(response)
}

fn positive_param(
    params: &HashMap<String, String>,
    key: &str,
    default: i64,
    max: i64,
) -> Result<i64, Response> {
    match params.get(key) {
        None => Ok(default),
        Some(value) => value
            .parse::<i64>()
            .ok()
            .filter(|value| *value > 0 && *value <= max)
            .ok_or_else(|| {
                error_response(
                    StatusCode::BAD_REQUEST,
                    format!("{key} must be a positive integer no greater than {max}"),
                )
            }),
    }
}

fn parse_date_range(params: &HashMap<String, String>) -> Result<DateRange, Response> {
    let now = Utc::now();
    if let Some(period) = params.get("period") {
        if params.contains_key("startAt")
            || params.contains_key("endAt")
            || params.contains_key("startDate")
            || params.contains_key("endDate")
        {
            return Err(error_response(
                StatusCode::BAD_REQUEST,
                "period cannot be combined with an explicit date range",
            ));
        }
        let duration = match period.as_str() {
            "today" | "0day" => Duration::days(1),
            "24h" | "24hour" => Duration::hours(24),
            "7d" | "7day" => Duration::days(7),
            "30d" | "30day" => Duration::days(30),
            "0week" => Duration::weeks(1),
            "0month" => Duration::days(31),
            "90day" => Duration::days(90),
            "0year" | "12month" => Duration::days(366),
            "6month" => Duration::days(183),
            _ => return Err(error_response(StatusCode::BAD_REQUEST, "Invalid period")),
        };
        return Ok(DateRange {
            start: now - duration,
            end: now,
        });
    }
    let range = if params.contains_key("startAt") || params.contains_key("endAt") {
        let start = parse_millis(params, "startAt")?;
        let end = parse_millis(params, "endAt")?;
        DateRange { start, end }
    } else if params.contains_key("startDate") || params.contains_key("endDate") {
        let start = parse_iso_date(params, "startDate")?;
        let end = parse_iso_date(params, "endDate")?;
        DateRange { start, end }
    } else {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "Either startAt+endAt, startDate+endDate, or period must be provided",
        ));
    };
    if range.end < range.start {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "endAt must be greater than or equal to startAt",
        ));
    }
    Ok(range)
}

fn parse_millis(params: &HashMap<String, String>, key: &str) -> Result<DateTime<Utc>, Response> {
    let value = params
        .get(key)
        .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, format!("{key} is required")))?
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value >= i64::MIN as f64 && *value <= i64::MAX as f64)
        .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, format!("Invalid {key}")))?;
    Utc.timestamp_millis_opt(value as i64)
        .single()
        .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, format!("Invalid {key}")))
}

fn parse_iso_date(params: &HashMap<String, String>, key: &str) -> Result<DateTime<Utc>, Response> {
    params
        .get(key)
        .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, format!("{key} is required")))?
        .parse::<DateTime<Utc>>()
        .map_err(|_| error_response(StatusCode::BAD_REQUEST, format!("Invalid {key}")))
}

fn replay_page(params: &HashMap<String, String>) -> Result<ReplayPage, Response> {
    let page = positive_param(params, "page", 1, i32::MAX as i64)?;
    let page_size = positive_param(params, "pageSize", 20, 500)?;
    if let Some(value) = params.get("maxResults") {
        value
            .parse::<i64>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| {
                error_response(
                    StatusCode::BAD_REQUEST,
                    "maxResults must be a positive integer",
                )
            })?;
    }
    let min_duration_ms = match params.get("minDuration") {
        None => None,
        Some(value) => Some(
            value
                .parse::<i64>()
                .ok()
                .filter(|value| *value >= 0)
                .and_then(|value| value.checked_mul(1000))
                .ok_or_else(|| {
                    error_response(
                        StatusCode::BAD_REQUEST,
                        "minDuration must be a non-negative integer",
                    )
                })?,
        ),
    };
    let search = params
        .get("search")
        .filter(|value| !value.is_empty())
        .map(|value| format!("%{value}%"));
    Ok(ReplayPage {
        page,
        page_size,
        range: parse_date_range(params)?,
        search,
        min_duration_ms,
    })
}

async fn list_replays(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let page = match replay_page(&params) {
        Ok(page) => page,
        Err(response) => return response,
    };
    let pool = match website_pool(&state, &headers, website_id, Access::Read).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    replay_list(&pool, website_id, None, page, params).await
}

async fn list_session_replays(
    State(state): State<AppState>,
    Path((website_id, session_id)): Path<(Uuid, Uuid)>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let page = match replay_page(&params) {
        Ok(page) => page,
        Err(response) => return response,
    };
    let pool = match website_pool(&state, &headers, website_id, Access::Read).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    replay_list(&pool, website_id, Some(session_id), page, params).await
}

async fn replay_list(
    pool: &PgPool,
    website_id: Uuid,
    session_id: Option<Uuid>,
    page: ReplayPage,
    mut params: HashMap<String, String>,
) -> Response {
    let include_distinct_ids = session_id.is_none();
    let audience = match crate::audience::Audience::resolve(pool, website_id, &mut params).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let scalar = match crate::insights::scalar_filter_predicate(&params) {
        Ok(f) => f,
        Err(r) => return r,
    };
    let scope = |q: &mut QueryBuilder<'_, Postgres>| {
        q.push(" FROM session_replay sr JOIN session s ON s.website_id=sr.website_id AND s.session_id=sr.session_id WHERE sr.website_id=")
            .push_bind(website_id).push(" AND sr.created_at BETWEEN ").push_bind(page.range.start)
            .push(" AND ").push_bind(page.range.end);
        if let Some(id) = session_id {
            q.push(" AND sr.session_id=").push_bind(id);
        }
        if let Some(search) = &page.search {
            q.push(" AND EXISTS (SELECT 1 FROM unnest(ARRAY[s.distinct_id,s.city,s.browser,s.os,s.device]) v WHERE v ILIKE ")
                .push_bind(search.clone()).push(")");
        }
        q.push(" AND EXISTS (SELECT 1 FROM website_event e WHERE e.website_id=sr.website_id AND e.session_id=sr.session_id AND e.visit_id=sr.visit_id AND e.created_at BETWEEN ")
            .push_bind(page.range.start).push(" AND ").push_bind(page.range.end);
        // The outer session s is the same website/session as this event.
        scalar.push(q, true);
        audience.push(q, "e", page.range.start, page.range.end);
        q.push(") GROUP BY sr.visit_id,sr.session_id,sr.website_id,s.browser,s.os,s.device,s.country,s.city");
        if let Some(min_duration) = page.min_duration_ms {
            q.push(" HAVING extract(epoch FROM max(sr.ended_at)-min(sr.started_at))*1000 >= ")
                .push_bind(min_duration);
        }
    };
    let mut count_query = QueryBuilder::new("SELECT count(*)::bigint FROM (SELECT sr.visit_id");
    scope(&mut count_query);
    let count: i64 = match count_query
        .push(") replay")
        .build_query_scalar()
        .fetch_one(pool)
        .await
    {
        Ok(n) => n,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    let offset = match (page.page - 1).checked_mul(page.page_size) {
        Some(offset) => offset,
        None => return error_response(StatusCode::BAD_REQUEST, "Invalid pagination"),
    };
    let mut query = QueryBuilder::new(
        r#"
        SELECT
            sr.visit_id AS id,
            sr.session_id,
            sr.website_id,
            s.browser,
            s.os,
            s.device,
            s.country,
            s.city,
            sum(sr.event_count)::bigint AS event_count,
            count(sr.replay_id)::bigint AS chunk_count,
            min(sr.started_at) AS started_at,
            max(sr.ended_at) AS ended_at,
            (extract(epoch from max(sr.ended_at) - min(sr.started_at)) * 1000)::bigint AS duration,
            max(sr.created_at) AS created_at
        "#,
    );
    scope(&mut query);
    let rows = match query
        .push(" ORDER BY max(sr.created_at) DESC LIMIT ")
        .push_bind(page.page_size)
        .push(" OFFSET ")
        .push_bind(offset)
        .build()
        .fetch_all(pool)
        .await
    {
        Ok(rows) => rows,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    let visit_ids = rows
        .iter()
        .map(|row| row.get::<Uuid, _>("id"))
        .collect::<Vec<_>>();
    let distinct_ids = if include_distinct_ids {
        match replay_distinct_ids(pool, website_id, &visit_ids).await {
            Ok(ids) => ids,
            Err(response) => return response,
        }
    } else {
        HashMap::new()
    };
    let data = rows
        .into_iter()
        .map(|row| {
            let id = row.get::<Uuid, _>("id");
            let mut replay = json!({
                "id": id,
                "websiteId": row.get::<Uuid, _>("website_id"),
                "sessionId": row.get::<Uuid, _>("session_id"),
                "createdAt": row.get::<Option<DateTime<Utc>>, _>("created_at"),
                "startedAt": row.get::<DateTime<Utc>, _>("started_at"),
                "endedAt": row.get::<DateTime<Utc>, _>("ended_at"),
                "duration": row.get::<i64, _>("duration"),
                "eventCount": row.get::<i64, _>("event_count"),
                "chunkCount": row.get::<i64, _>("chunk_count"),
                "browser": row.get::<Option<String>, _>("browser"),
                "os": row.get::<Option<String>, _>("os"),
                "device": row.get::<Option<String>, _>("device"),
                "country": row.get::<Option<String>, _>("country"),
                "city": row.get::<Option<String>, _>("city"),
            });
            if include_distinct_ids {
                replay["distinctIds"] = json!(distinct_ids.get(&id).cloned().unwrap_or_default());
            }
            replay
        })
        .collect::<Vec<_>>();
    json_response(json!({
        "data": data,
        "count": count,
        "page": page.page,
        "pageSize": page.page_size
    }))
}

async fn replay_distinct_ids(
    pool: &PgPool,
    website_id: Uuid,
    visit_ids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<String>>, Response> {
    if visit_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query(
        r#"
        SELECT DISTINCT sr.visit_id, sl.distinct_id, true AS linked
        FROM session_replay sr
        JOIN session_link sl
          ON sl.website_id = sr.website_id AND sl.session_id = sr.session_id
        WHERE sr.website_id = $1 AND sr.visit_id = ANY($2::uuid[])
        UNION
        SELECT DISTINCT sr.visit_id, s.distinct_id, false AS linked
        FROM session_replay sr
        JOIN session s
          ON s.website_id = sr.website_id AND s.session_id = sr.session_id
        WHERE sr.website_id = $1
          AND sr.visit_id = ANY($2::uuid[])
          AND s.distinct_id <> ''
        "#,
    )
    .bind(website_id)
    .bind(visit_ids)
    .fetch_all(pool)
    .await
    .map_err(|_| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unable to load replay identities",
        )
    })?;
    let mut linked: HashMap<Uuid, BTreeSet<String>> = HashMap::new();
    let mut own: HashMap<Uuid, BTreeSet<String>> = HashMap::new();
    for row in rows {
        let target = if row.get::<bool, _>("linked") {
            &mut linked
        } else {
            &mut own
        };
        target
            .entry(row.get("visit_id"))
            .or_default()
            .insert(row.get("distinct_id"));
    }
    Ok(visit_ids
        .iter()
        .map(|visit_id| {
            let values = linked
                .remove(visit_id)
                .or_else(|| own.remove(visit_id))
                .unwrap_or_default()
                .into_iter()
                .collect();
            (*visit_id, values)
        })
        .collect())
}

async fn replay_detail(
    State(state): State<AppState>,
    Path((website_id, replay_id)): Path<(Uuid, Uuid)>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let until = match optional_i64(&params, "until", i64::MAX) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let end_chunk_index = match optional_i64(&params, "chunkIndex", i32::MAX as i64) {
        Ok(value) => value.map(|value| value as i32),
        Err(response) => return response,
    };
    let end_event_index = match optional_i64(&params, "eventIndex", i32::MAX as i64) {
        Ok(value) => value.map(|value| value as usize),
        Err(response) => return response,
    };
    let end_at = match until {
        Some(value) => match Utc.timestamp_millis_opt(value).single() {
            Some(value) => Some(value),
            None => return error_response(StatusCode::BAD_REQUEST, "Invalid until"),
        },
        None => None,
    };
    let pool = match website_pool(&state, &headers, website_id, Access::Read).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let rows = match sqlx::query(
        r#"
        SELECT session_id, events, chunk_index, started_at, ended_at
        FROM session_replay
        WHERE website_id = $1
          AND visit_id = $2
          AND ($3::timestamptz IS NULL OR started_at <= $3)
          AND ($4::integer IS NULL OR chunk_index <= $4)
        ORDER BY chunk_index ASC
        "#,
    )
    .bind(website_id)
    .bind(replay_id)
    .bind(end_at)
    .bind(end_chunk_index)
    .fetch_all(&pool)
    .await
    {
        Ok(rows) => rows,
        Err(_) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Unable to load replay")
        }
    };
    let mut chunks = Vec::with_capacity(rows.len());
    for row in rows {
        let compressed = row.get::<Vec<u8>, _>("events");
        let decoded = match gzip_decode(&compressed, MAX_REPLAY_BYTES)
            .and_then(|bytes| serde_json::from_slice::<Vec<Value>>(&bytes).map_err(|_| ()))
        {
            Ok(events) => events,
            Err(()) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Unable to decode replay events",
                )
            }
        };
        chunks.push(ReplayChunk {
            session_id: row.get("session_id"),
            chunk_index: row.get("chunk_index"),
            events: decoded,
            started_at: row.get("started_at"),
            ended_at: row.get("ended_at"),
        });
    }
    let session_id = chunks.first().map(|chunk| chunk.session_id);
    let started_at = chunks.first().map(|chunk| chunk.started_at);
    let ended_at = chunks.last().map(|chunk| chunk.ended_at);
    let events = restore_fragments(merge_events(
        &chunks,
        until,
        end_chunk_index,
        end_event_index,
    ));
    json_response(json!({
        "sessionId": session_id,
        "events": events,
        "startedAt": started_at,
        "endedAt": ended_at,
        "eventCount": events.len(),
        "chunkCount": chunks.len()
    }))
}

fn optional_i64(
    params: &HashMap<String, String>,
    key: &str,
    max: i64,
) -> Result<Option<i64>, Response> {
    params
        .get(key)
        .map(|value| {
            value
                .parse::<i64>()
                .ok()
                .filter(|value| *value >= 0 && *value <= max)
                .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, format!("Invalid {key}")))
        })
        .transpose()
}

fn merge_events(
    chunks: &[ReplayChunk],
    until: Option<i64>,
    end_chunk_index: Option<i32>,
    end_event_index: Option<usize>,
) -> Vec<Value> {
    let mut events = Vec::new();
    let mut last_timestamp = f64::NEG_INFINITY;
    let mut sorted = true;
    for chunk in chunks {
        for (index, event) in chunk.events.iter().enumerate() {
            if chunk.chunk_index == end_chunk_index.unwrap_or(i32::MIN)
                && end_event_index.is_some_and(|end| index > end)
            {
                continue;
            }
            let timestamp = event_timestamp(event);
            if until.is_some_and(|until| timestamp.is_some_and(|value| value > until as f64)) {
                continue;
            }
            if let Some(timestamp) = timestamp {
                if timestamp < last_timestamp {
                    sorted = false;
                } else {
                    last_timestamp = timestamp;
                }
            }
            events.push(event.clone());
        }
    }
    if !sorted {
        events.sort_by(|left, right| {
            event_timestamp(left)
                .unwrap_or(0.0)
                .total_cmp(&event_timestamp(right).unwrap_or(0.0))
        });
    }
    events
}

#[derive(Debug)]
struct Fragment<'a> {
    id: &'a str,
    index: usize,
    total: usize,
    value: &'a str,
}

fn fragment(event: &Value) -> Option<Fragment<'_>> {
    let object = event.as_object()?;
    if object.get("type")?.as_str()? != "umami:rrweb-event-fragment" {
        return None;
    }
    let data = object.get("data")?.as_object()?;
    let total = data.get("total")?.as_u64()? as usize;
    let index = data.get("index")?.as_u64()? as usize;
    if total == 0 || total > 10_000 || index >= total {
        return None;
    }
    Some(Fragment {
        id: data.get("id")?.as_str()?,
        index,
        total,
        value: data.get("value")?.as_str()?,
    })
}

fn restore_fragments(events: Vec<Value>) -> Vec<Value> {
    struct Pending {
        total: usize,
        values: HashMap<usize, String>,
    }
    let mut restored = Vec::new();
    let mut pending: HashMap<String, Pending> = HashMap::new();
    for event in events {
        let Some(part) = fragment(&event) else {
            restored.push(event);
            continue;
        };
        let id = part.id.to_string();
        let index = part.index;
        let total = part.total;
        let value = part.value.to_string();
        let group = pending.entry(id.clone()).or_insert_with(|| Pending {
            total,
            values: HashMap::new(),
        });
        if group.total != total {
            *group = Pending {
                total,
                values: HashMap::new(),
            };
        }
        group.values.entry(index).or_insert(value);
        if group.values.len() == group.total {
            let mut serialized = String::new();
            for index in 0..group.total {
                let Some(value) = group.values.get(&index) else {
                    serialized.clear();
                    break;
                };
                if serialized.len() + value.len() > MAX_REPLAY_BYTES {
                    serialized.clear();
                    break;
                }
                serialized.push_str(value);
            }
            pending.remove(&id);
            if let Ok(value) = serde_json::from_str(&serialized) {
                restored.push(value);
            }
        }
    }
    restored
}

async fn saved_replay_status(
    State(state): State<AppState>,
    Path((website_id, replay_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Response {
    let pool = match website_pool(&state, &headers, website_id, Access::Read).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    match sqlx::query_scalar::<_, bool>(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM session_replay_saved
            WHERE website_id = $1 AND visit_id = $2
        )
        "#,
    )
    .bind(website_id)
    .bind(replay_id)
    .fetch_one(&pool)
    .await
    {
        Ok(is_saved) => json_response(json!({ "isSaved": is_saved })),
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unable to load saved replay",
        ),
    }
}

async fn set_saved_replay(
    State(state): State<AppState>,
    Path((website_id, replay_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    request: Request,
) -> Response {
    let bytes = match to_bytes(request.into_body(), MAX_SAVED_REQUEST_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid request body"),
    };
    let body = match serde_json::from_slice::<Value>(&bytes) {
        Ok(Value::Object(body)) => body,
        _ => return error_response(StatusCode::BAD_REQUEST, "Invalid request body"),
    };
    let Some(is_saved) = body.get("isSaved").and_then(Value::as_bool) else {
        return error_response(StatusCode::BAD_REQUEST, "isSaved is required");
    };
    let name = match body.get("name") {
        None | Some(Value::Null) => "",
        Some(Value::String(name)) if name.chars().count() <= 100 => name,
        _ => return error_response(StatusCode::BAD_REQUEST, "Invalid replay name"),
    };
    let access = if is_saved {
        Access::Write
    } else {
        Access::Delete
    };
    let pool = match website_pool(&state, &headers, website_id, access).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let result = if is_saved {
        sqlx::query(
            r#"
            INSERT INTO session_replay_saved (
                saved_replay_id, name, website_id, visit_id
            )
            VALUES ($1, $2, $3, $4)
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(name)
        .bind(website_id)
        .bind(replay_id)
        .execute(&pool)
        .await
    } else {
        sqlx::query("DELETE FROM session_replay_saved WHERE website_id = $1 AND visit_id = $2")
            .bind(website_id)
            .bind(replay_id)
            .execute(&pool)
            .await
    };
    match result {
        Ok(_) => json_response(json!({ "ok": true })),
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unable to update saved replay",
        ),
    }
}

async fn list_saved_replays(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let page = match positive_param(&params, "page", 1, i32::MAX as i64) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let page_size = match positive_param(&params, "pageSize", 20, 500) {
        Ok(value) => value,
        Err(response) => return response,
    };
    if let Some(value) = params.get("maxResults") {
        if value
            .parse::<i64>()
            .ok()
            .filter(|value| *value > 0)
            .is_none()
        {
            return error_response(
                StatusCode::BAD_REQUEST,
                "maxResults must be a positive integer",
            );
        }
    }
    let search = params
        .get("search")
        .filter(|value| !value.is_empty())
        .map(|value| format!("%{value}%"));
    let pool = match website_pool(&state, &headers, website_id, Access::Read).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let count = match sqlx::query_scalar::<_, i64>(
        r#"
        SELECT count(*)::bigint
        FROM session_replay_saved
        WHERE website_id = $1 AND ($2::text IS NULL OR name ILIKE $2)
        "#,
    )
    .bind(website_id)
    .bind(search.as_deref())
    .fetch_one(&pool)
    .await
    {
        Ok(count) => count,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to count saved replays",
            )
        }
    };
    let offset = match (page - 1).checked_mul(page_size) {
        Some(offset) => offset,
        None => return error_response(StatusCode::BAD_REQUEST, "Invalid pagination"),
    };
    let rows = match sqlx::query(
        r#"
        SELECT saved_replay_id, name, website_id, visit_id, created_at, updated_at
        FROM session_replay_saved
        WHERE website_id = $1 AND ($2::text IS NULL OR name ILIKE $2)
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
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to load saved replays",
            )
        }
    };
    let visit_ids = rows
        .iter()
        .map(|row| row.get::<Uuid, _>("visit_id"))
        .collect::<Vec<_>>();
    let distinct_ids = match replay_distinct_ids(&pool, website_id, &visit_ids).await {
        Ok(ids) => ids,
        Err(response) => return response,
    };
    let data = rows
        .into_iter()
        .map(|row| {
            let visit_id = row.get::<Uuid, _>("visit_id");
            json!({
                "id": row.get::<Uuid, _>("saved_replay_id"),
                "name": row.get::<String, _>("name"),
                "websiteId": row.get::<Uuid, _>("website_id"),
                "visitId": visit_id,
                "createdAt": row.get::<Option<DateTime<Utc>>, _>("created_at"),
                "updatedAt": row.get::<Option<DateTime<Utc>>, _>("updated_at"),
                "distinctIds": distinct_ids.get(&visit_id).cloned().unwrap_or_default()
            })
        })
        .collect::<Vec<_>>();
    json_response(json!({
        "data": data,
        "count": count,
        "page": page,
        "pageSize": page_size
    }))
}

fn gzip_encode(input: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(input)?;
    encoder.finish()
}

fn gzip_decode(input: &[u8], max_output: usize) -> Result<Vec<u8>, ()> {
    let limit = max_output.checked_add(1).ok_or(())? as u64;
    let mut decoder = GzDecoder::new(input).take(limit);
    let mut output = Vec::new();
    decoder.read_to_end(&mut output).map_err(|_| ())?;
    if output.len() > max_output {
        return Err(());
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, EncodingKey, Header};

    #[test]
    fn gzip_is_compatible_with_node_output() {
        let expected = br#"[{"type":4,"timestamp":1700000000000},{"type":2,"data":{"node":{"type":0,"childNodes":[]}},"timestamp":1700000000010}]"#;
        let node_gzip = hex::decode("1f8b08000000000002ff8bae562aa92c4855b232d1512ac9cc4d2d2e49cc2d50b232343740805a1d9822231da594c4924425ab6aa5bcfc9454100d9130d0514acec8cc49f1038a162b5945c7d6d6e232cfd0a0361600ba310d9276000000").unwrap();
        assert_eq!(gzip_decode(&node_gzip, 1024).unwrap(), expected);
        assert_eq!(
            gzip_decode(&gzip_encode(expected).unwrap(), 1024).unwrap(),
            expected
        );
    }

    #[test]
    fn fragments_count_and_restore_as_one_event() {
        let event = json!({"type": 2, "timestamp": 10, "data": {"node": {"type": 0}}});
        let serialized = serde_json::to_string(&event).unwrap();
        let split = serialized.len() / 2;
        let events = vec![
            json!({"type": 4, "timestamp": 1}),
            json!({"type": "umami:rrweb-event-fragment", "data": {"id": "a", "index": 0, "total": 2, "value": &serialized[..split]}}),
            json!({"type": "umami:rrweb-event-fragment", "data": {"id": "a", "index": 1, "total": 2, "value": &serialized[split..]}}),
        ];
        assert_eq!(replay_event_count(&events), 2);
        assert_eq!(
            restore_fragments(events),
            vec![json!({"type": 4, "timestamp": 1}), event]
        );
    }

    #[test]
    fn cache_tokens_are_bound_to_their_website() {
        let website_id = Uuid::new_v4();
        let secret = b"recording-test-secret";
        let claims = json!({
            "type": "cache",
            "websiteId": website_id,
            "sessionId": Uuid::new_v4(),
            "visitId": Uuid::new_v4(),
            "iat": 1
        });
        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(secret),
        )
        .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-umami-cache", token.parse().unwrap());
        assert!(parse_cache_with_secret(&headers, website_id, secret).is_some());
        assert!(parse_cache_with_secret(&headers, Uuid::new_v4(), secret).is_none());
    }
}
