use aes::Aes256;
use aes_gcm::{
    aead::{consts::U16, Aead, KeyInit},
    AesGcm, Nonce,
};
use axum::{
    body::{to_bytes, Body},
    extract::{Path, Query, Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{any, get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use chrono::{DateTime, Datelike, TimeZone, Timelike, Utc};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use pbkdf2::pbkdf2_hmac;
use rand::{distributions::Alphanumeric, Rng, RngCore};
use regex::Regex;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha512};
use sqlx::{postgres::PgPoolOptions, PgPool, Row};
use std::{
    collections::{HashMap, HashSet},
    env,
    path::{Path as FsPath, PathBuf},
    str::FromStr,
    sync::{Arc, OnceLock},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::RwLock;
use url::Url;
use uuid::Uuid;

mod audience;
mod auth;
mod cohort;
mod collection;
mod mcp;
mod metrics;
mod recording;
mod reports;
mod teams;
mod trackables;
use auth::*;
use collection::*;
use metrics::*;

mod access;
mod acquisition_report;
mod activation_report;
mod analytics_api;
mod app_overview;
mod app_reports_accessibility;
mod app_reports_devices;
mod app_reports_versions;
mod app_settings;
mod boards;
mod dashboard_api;
mod error_reports;
mod event_types;
mod insights;
mod lifecycle;
mod notebooks;
mod properties;
mod property_predicate;
mod retention_report;
mod shares;
mod website_ops;
mod workspace_card_metrics;
mod workspace_folders;
mod workspace_poll;

type Aes256Gcm16 = AesGcm<Aes256, U16>;

const HISTORICAL_CUTOFF_MS: i64 = 5 * 60 * 1000;
const ICON_CACHE_TTL_SECS: u64 = 7 * 24 * 60 * 60;
const ICON_CACHE_CONTROL: &str = "public, max-age=86400, s-maxage=604800";

#[derive(Clone)]
pub struct AppState {
    pub db: Option<PgPool>,
    pub http: reqwest::Client,
    pub td_options: TdOptions,
    pub node_upstream: Option<(String, reqwest::Client)>,
    pub static_root: PathBuf,
    pub icon_cache: Arc<RwLock<HashMap<String, CachedIcon>>>,
}

#[derive(Clone)]
pub struct CachedIcon {
    pub bytes: Vec<u8>,
    pub content_type: String,
    pub expires_at: Instant,
}

#[derive(Clone, Default)]
pub struct TdOptions {
    pub hostname: Option<String>,
    pub test_website_id: Option<String>,
    pub allowed_keys: HashSet<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SignalDisposition {
    Mapped(SendEnvelope),
    Rejected,
    Dropped,
}

pub fn sha512_hex(parts: &[&str]) -> String {
    let mut hasher = Sha512::new();
    for p in parts {
        hasher.update(p.as_bytes());
    }
    hex::encode(hasher.finalize())
}

pub fn app_secret() -> String {
    env::var("APP_SECRET")
        .ok()
        .filter(|s| !s.trim().is_empty() && s.trim() != "replace-me-with-a-random-string")
        .expect("Set APP_SECRET to a nonempty secret before starting Maap")
}

pub fn secret_hash() -> String {
    sha512_hex(&[&app_secret()])
}

pub fn deterministic_uuid(parts: &[&str]) -> Uuid {
    let sec = secret_hash();
    let mut all: Vec<&str> = parts.to_vec();
    all.push(&sec);
    let digest = sha512_hex(&all);
    Uuid::new_v5(&Uuid::NAMESPACE_DNS, digest.as_bytes())
}

pub fn to_js_utc_string(dt: DateTime<Utc>) -> String {
    dt.format("%a, %d %b %Y %H:%M:%S GMT").to_string()
}

pub fn start_of_hour_utc(dt: DateTime<Utc>) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(dt.year(), dt.month(), dt.day(), dt.hour(), 0, 0)
        .single()
        .unwrap_or(dt)
}

pub fn start_of_month_utc(dt: DateTime<Utc>) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(dt.year(), dt.month(), 1, 0, 0, 0)
        .single()
        .unwrap_or(dt)
}

// Mirrors the tracker's safeDecodeURI/safeDecodeURIComponent: a malformed URI stays intact.
fn decode_uri(value: &str, preserve_delimiters: bool) -> String {
    let bytes = value.as_bytes();
    if !bytes.contains(&b'%') {
        return value.to_owned();
    }
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            decoded.push(bytes[index]);
            index += 1;
            continue;
        }
        let Some(pair) = value
            .get(index + 1..index + 3)
            .filter(|s| s.bytes().all(|b| b.is_ascii_hexdigit()))
        else {
            return value.to_owned();
        };
        let byte = u8::from_str_radix(pair, 16).expect("two hex digits");
        if preserve_delimiters && b";,/?:@&=+$#".contains(&byte) {
            decoded.extend_from_slice(&bytes[index..index + 3]);
        } else {
            decoded.push(byte);
        }
        index += 3;
    }
    String::from_utf8(decoded).unwrap_or_else(|_| value.to_owned())
}

pub fn with_rust_header(mut res: Response) -> Response {
    res.headers_mut()
        .insert("x-maap-server", HeaderValue::from_static("rust"));
    res
}

pub fn json_response(value: Value) -> Response {
    with_rust_header(Json(value).into_response())
}

pub fn status_response(status: StatusCode) -> Response {
    if status.is_client_error() || status.is_server_error() {
        return error_response(
            status,
            status.canonical_reason().unwrap_or("Request failed"),
        );
    }
    with_rust_header(status.into_response())
}

pub fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    let message = message.into();
    let code = match status {
        StatusCode::BAD_REQUEST => "bad-request",
        StatusCode::UNAUTHORIZED => "unauthorized",
        StatusCode::FORBIDDEN => "forbidden",
        StatusCode::NOT_FOUND => "not-found",
        StatusCode::SERVICE_UNAVAILABLE => "service-unavailable",
        StatusCode::NOT_IMPLEMENTED => "not-implemented",
        _ => "server-error",
    };
    let message = if status == StatusCode::INTERNAL_SERVER_ERROR {
        eprintln!("API error: {message}");
        "Server error".to_owned()
    } else {
        message
    };
    with_rust_header(
        (
            status,
            Json(json!({
                "error": {
                    "message": message,
                    "code": code,
                    "status": status.as_u16()
                }
            })),
        )
            .into_response(),
    )
}

pub fn page_params(params: &HashMap<String, String>) -> (i64, i64, i64) {
    let page = params
        .get("page")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(1)
        .max(1);
    let page_size = params
        .get("pageSize")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(20)
        .clamp(1, 500);
    (page, page_size, (page - 1).saturating_mul(page_size))
}

async fn heartbeat(State(state): State<AppState>) -> Response {
    if let Some((upstream, client)) = &state.node_upstream {
        let url = format!("{upstream}/api/heartbeat");
        let Ok(res) = client.get(&url).send().await else {
            return with_rust_header(StatusCode::SERVICE_UNAVAILABLE.into_response());
        };
        if !res.status().is_success() {
            return with_rust_header(StatusCode::SERVICE_UNAVAILABLE.into_response());
        }
    }
    with_rust_header(Json(json!({ "ok": true })).into_response())
}

pub fn fallback_os_icon(store_url: &str) -> &'static str {
    static PLAY_RE: OnceLock<Regex> = OnceLock::new();
    static MAC_RE: OnceLock<Regex> = OnceLock::new();
    let play_re = PLAY_RE.get_or_init(|| Regex::new(r"(?i)play\.google\.com").unwrap());
    let mac_re = MAC_RE.get_or_init(|| Regex::new(r"(?i)(/mac/|mt=12)").unwrap());
    if play_re.is_match(store_url) {
        "/images/os/android-os.png"
    } else if mac_re.is_match(store_url) {
        "/images/os/mac-os.png"
    } else {
        "/images/os/ios.png"
    }
}

pub async fn resolve_app_icon_url(client: &reqwest::Client, store_url: &str) -> Option<String> {
    static APPLE_RE: OnceLock<Regex> = OnceLock::new();
    static PLAY_RE: OnceLock<Regex> = OnceLock::new();
    static OG_RE_1: OnceLock<Regex> = OnceLock::new();
    static OG_RE_2: OnceLock<Regex> = OnceLock::new();

    let apple_re = APPLE_RE.get_or_init(|| {
        Regex::new(r"(?i)^https?://(?:itunes|apps)\.apple\.com/(?:([a-z]{2})/)?[^?#]*?\bid(\d+)")
            .unwrap()
    });
    if let Some(caps) = apple_re.captures(store_url) {
        let country = caps
            .get(1)
            .map(|m| m.as_str().to_ascii_lowercase())
            .unwrap_or_else(|| "us".to_string());
        let id = caps.get(2)?.as_str();
        let lookup = format!("https://itunes.apple.com/lookup?id={id}&country={country}");
        let res = client.get(&lookup).send().await.ok()?;
        if !res.status().is_success() {
            return None;
        }
        let data: Value = res.json().await.ok()?;
        let item = data.get("results")?.as_array()?.first()?;
        return item
            .get("artworkUrl100")
            .or_else(|| item.get("artworkUrl512"))
            .or_else(|| item.get("artworkUrl60"))
            .and_then(|v| v.as_str())
            .map(str::to_string);
    }

    let play_re = PLAY_RE.get_or_init(|| {
        Regex::new(
            r"(?i)^https?://play\.google\.com/store/apps/details\?[^#]*\bid=([A-Za-z0-9._]+)",
        )
        .unwrap()
    });
    if let Some(caps) = play_re.captures(store_url) {
        let pkg = caps.get(1)?.as_str();
        let page = format!("https://play.google.com/store/apps/details?id={pkg}&hl=en_US&gl=US");
        let res = client.get(&page).send().await.ok()?;
        if !res.status().is_success() {
            return None;
        }
        let html = res.text().await.ok()?;
        let og1 = OG_RE_1.get_or_init(|| {
            Regex::new(r#"(?i)<meta[^>]+property=["']og:image["'][^>]+content=["']([^"']+)["']"#)
                .unwrap()
        });
        let og2 = OG_RE_2.get_or_init(|| {
            Regex::new(r#"(?i)<meta[^>]+content=["']([^"']+)["'][^>]+property=["']og:image["']"#)
                .unwrap()
        });
        return og1
            .captures(&html)
            .or_else(|| og2.captures(&html))
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_string());
    }

    None
}

async fn handle_app_icon(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
    req: Request,
) -> Response {
    let store_url = params.get("url").map(String::as_str).unwrap_or("").trim();
    if store_url.is_empty() {
        return with_rust_header(StatusCode::BAD_REQUEST.into_response());
    }

    {
        let cache = state.icon_cache.read().await;
        if let Some(hit) = cache.get(store_url) {
            if hit.expires_at > Instant::now() {
                let mut res = Response::new(Body::from(hit.bytes.clone()));
                res.headers_mut().insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_str(&hit.content_type)
                        .unwrap_or_else(|_| HeaderValue::from_static("image/png")),
                );
                res.headers_mut().insert(
                    header::CACHE_CONTROL,
                    HeaderValue::from_static(ICON_CACHE_CONTROL),
                );
                return with_rust_header(res);
            }
        }
    }

    if let Some(icon_url) = resolve_app_icon_url(&state.http, store_url).await {
        if let Ok(upstream) = state.http.get(&icon_url).send().await {
            if upstream.status().is_success() {
                let content_type = upstream
                    .headers()
                    .get(header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("image/png")
                    .to_string();
                if let Ok(bytes) = upstream.bytes().await {
                    let vec = bytes.to_vec();
                    state.icon_cache.write().await.insert(
                        store_url.to_string(),
                        CachedIcon {
                            bytes: vec.clone(),
                            content_type: content_type.clone(),
                            expires_at: Instant::now()
                                + std::time::Duration::from_secs(ICON_CACHE_TTL_SECS),
                        },
                    );
                    let mut res = Response::new(Body::from(vec));
                    res.headers_mut().insert(
                        header::CONTENT_TYPE,
                        HeaderValue::from_str(&content_type)
                            .unwrap_or_else(|_| HeaderValue::from_static("image/png")),
                    );
                    res.headers_mut().insert(
                        header::CACHE_CONTROL,
                        HeaderValue::from_static(ICON_CACHE_CONTROL),
                    );
                    return with_rust_header(res);
                }
            }
        }
    }

    let fallback_path = fallback_os_icon(store_url);
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("127.0.0.1:3000");
    let scheme = req
        .headers()
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("http");
    let location = format!("{scheme}://{host}{fallback_path}");
    let mut res = Response::new(Body::empty());
    *res.status_mut() = StatusCode::FOUND;
    if let Ok(loc) = HeaderValue::from_str(&location) {
        res.headers_mut().insert(header::LOCATION, loc);
    }
    with_rust_header(res)
}

async fn handle_create_website(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let Some(pool) = &state.db else {
        return with_rust_header(StatusCode::SERVICE_UNAVAILABLE.into_response());
    };
    let Some((user_id, role)) = check_bearer_auth(pool, &headers, "/api/websites").await else {
        return with_rust_header(StatusCode::UNAUTHORIZED.into_response());
    };
    if !matches!(role.as_str(), "admin" | "user") {
        return error_response(StatusCode::UNAUTHORIZED, "Unauthorized");
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Website {
        id: Option<Uuid>,
        name: String,
        domain: String,
        team_id: Option<Uuid>,
        share_id: Option<String>,
    }
    let Ok(input) = serde_json::from_value::<Website>(body) else {
        return error_response(StatusCode::BAD_REQUEST, "Invalid website");
    };
    let (name, domain) = (input.name.trim(), input.domain.trim());
    static DOMAIN: OnceLock<Regex> = OnceLock::new();
    let domain_pattern = DOMAIN.get_or_init(|| Regex::new(r"(?i)^(localhost(:[1-9][0-9]{0,4})?|([a-z0-9_-]{1,63}\.)+[a-z0-9_-]{2,63}|(https?://)?((apps|itunes)\.apple\.com/([a-z]{2}/)?(mac-)?app/([^/\s?#]+/)?id[0-9]+([/?#]\S*)?|play\.google\.com/store/apps/details\?([^#\s]*&)?id=[a-z0-9._]+([&#]\S*)?))$").unwrap());
    if name.is_empty()
        || name.chars().count() > 100
        || domain.chars().count() > 500
        || !domain_pattern.is_match(domain)
        || input
            .share_id
            .as_deref()
            .is_some_and(|s| s.chars().count() > 50)
    {
        return error_response(StatusCode::BAD_REQUEST, "Invalid website");
    }
    let team_id = input.team_id;
    if let Some(team) = team_id {
        let allowed = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM team t WHERE t.team_id=$1 AND t.deleted_at IS NULL AND
             ($3 OR EXISTS(SELECT 1 FROM team_user u WHERE u.team_id=t.team_id AND u.user_id=$2
             AND u.role IN ('team-owner','team-manager','team-member'))))",
        )
        .bind(team)
        .bind(user_id)
        .bind(role == "admin")
        .fetch_one(pool)
        .await;
        match allowed {
            Ok(true) => (),
            Ok(false) => return error_response(StatusCode::UNAUTHORIZED, "Unauthorized"),
            Err(_) => {
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Unable to verify team")
            }
        }
    }
    let id = input.id.unwrap_or_else(Uuid::new_v4);
    let owner = team_id.is_none().then_some(user_id);
    let now = Utc::now();
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to create website",
            )
        }
    };
    let inserted = sqlx::query(r#"INSERT INTO website (website_id,name,domain,user_id,team_id,created_by,created_at,updated_at)
        SELECT $1,$2,$3,$4,$5,$6,$7,$7 WHERE NOT EXISTS(
        SELECT 1 FROM link WHERE link_id=$1 UNION ALL SELECT 1 FROM pixel WHERE pixel_id=$1
        UNION ALL SELECT 1 FROM board WHERE board_id=$1 UNION ALL SELECT 1 FROM "user" WHERE user_id=$1)
        ON CONFLICT (website_id) DO NOTHING"#)
        .bind(id).bind(name).bind(domain).bind(owner).bind(team_id).bind(user_id).bind(now).execute(&mut *tx).await;
    match inserted {
        Ok(r) if r.rows_affected() == 1 => (),
        Ok(_) => return error_response(StatusCode::BAD_REQUEST, "ID already in use."),
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to create website",
            )
        }
    }
    let slug = input.share_id.filter(|s| !s.is_empty());
    if let Some(ref slug) = slug {
        let inserted = sqlx::query("INSERT INTO share (share_id,entity_id,share_type,name,slug,parameters,created_at,updated_at)
            VALUES ($1,$2,1,$3,$4,'{\"overview\":true,\"events\":true}'::jsonb,$5,$5) ON CONFLICT (slug) DO NOTHING")
            .bind(Uuid::new_v4()).bind(id).bind(name).bind(slug).bind(now).execute(&mut *tx).await;
        match inserted {
            Ok(r) if r.rows_affected() == 1 => (),
            Ok(_) => {
                return error_response(StatusCode::BAD_REQUEST, "That share ID is already taken.")
            }
            Err(_) => {
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Unable to create share")
            }
        }
    }
    if tx.commit().await.is_err() {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unable to create website",
        );
    }
    with_rust_header(
        Json(json!({
            "id": id,
            "name": name,
            "domain": domain,
            "userId": owner,
            "teamId": team_id,
            "createdBy": user_id,
            "createdAt": now,
            "updatedAt": now,
            "shareId": slug,
            "recorderEnabled": false, "replayConfig": null, "resetAt": null, "deletedAt": null
        }))
        .into_response(),
    )
}

fn parse_ms_param(params: &HashMap<String, String>, key: &str) -> Option<DateTime<Utc>> {
    let raw = params.get(key)?;
    if let Ok(ms) = raw.parse::<i64>() {
        return Utc.timestamp_millis_opt(ms).single();
    }
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn is_hop_by_hop(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "content-length"
            | "host"
    )
}

async fn convert_reqwest_response(resp: reqwest::Response) -> Response {
    let status = resp.status();
    let headers = resp.headers().clone();
    let Ok(bytes) = resp.bytes().await else {
        return status_response(StatusCode::BAD_GATEWAY);
    };
    let mut out = Response::new(Body::from(bytes));
    *out.status_mut() = status;
    for (name, value) in &headers {
        if !is_hop_by_hop(name.as_str()) {
            out.headers_mut().append(name, value.clone());
        }
    }
    out
}

fn content_type_for_path(path: &FsPath) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "webmanifest" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "txt" | "map" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn sanitize_rel_path(raw: &str) -> Option<String> {
    let trimmed = raw.trim_matches('/');
    if trimmed.contains('\\') {
        return None;
    }
    let mut parts = Vec::new();
    for seg in trimmed.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            return None;
        }
        parts.push(seg);
    }
    Some(parts.join("/"))
}

async fn serve_file_bytes(
    file_path: &FsPath,
    cache_control: Option<&'static str>,
) -> Option<Response> {
    let meta = tokio::fs::metadata(file_path).await.ok()?;
    if !meta.is_file() {
        return None;
    }
    let bytes = tokio::fs::read(file_path).await.ok()?;
    let mut res = Response::new(Body::from(bytes));
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(content_type_for_path(file_path)),
    );
    if let Some(cc) = cache_control {
        res.headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static(cc));
    }
    Some(with_rust_header(res))
}

async fn try_serve_static_or_spa(
    static_root: &FsPath,
    method: &axum::http::Method,
    uri_path: &str,
    has_rsc_header: bool,
    severed: bool,
) -> Option<Response> {
    if *method != axum::http::Method::GET && *method != axum::http::Method::HEAD {
        return None;
    }
    if uri_path.starts_with("/api/")
        || uri_path == "/v2"
        || uri_path.starts_with("/v2/")
        || uri_path == "/mcp"
        || uri_path == "/app-icon"
    {
        return None;
    }

    if let Some(rest) = uri_path.strip_prefix("/_next/static/") {
        let rel = sanitize_rel_path(rest)?;
        if rel.is_empty() {
            return None;
        }
        let target = static_root.join(".next/static").join(rel);
        return serve_file_bytes(&target, Some("public, max-age=31536000, immutable")).await;
    }

    let rel = sanitize_rel_path(uri_path)?;
    if !rel.is_empty() {
        let pub_file = static_root.join("public").join(&rel);
        let cache = if rel == "script.js" || rel == "recorder.js" {
            Some("public, max-age=86400, must-revalidate")
        } else {
            None
        };
        if let Some(res) = serve_file_bytes(&pub_file, cache).await {
            return Some(res);
        }
    }

    // Never serve full HTML documents to Next.js RSC / segment-prefetch requests.
    if has_rsc_header {
        return None;
    }

    let app_dir = static_root.join(".next/server/app");
    // Next rewrites team-scoped URLs to the same pages; the browser retains the team path.
    let segments: Vec<_> = rel.split('/').collect();
    let rel = match segments.as_slice() {
        ["teams", id, rest @ ..] if Uuid::parse_str(id).is_ok() => {
            if rest.is_empty() {
                "websites".to_owned()
            } else {
                rest.join("/")
            }
        }
        _ => rel,
    };
    let mut candidates = Vec::new();
    if severed && rel == "logout" {
        candidates.push(app_dir.join("login.html"));
    }
    if severed && rel.starts_with("share/") {
        candidates.push(app_dir.join("share.html"));
    }
    if rel.is_empty() {
        candidates.push(app_dir.join("index.html"));
    } else {
        candidates.push(app_dir.join(format!("{rel}.html")));
        candidates.push(app_dir.join(&rel).join("index.html"));
        candidates.push(app_dir.join("(main)").join(format!("{rel}.html")));
        candidates.push(app_dir.join("(main)").join(&rel).join("index.html"));
    }

    let parts: Vec<_> = rel.split('/').collect();
    let shell_route = match parts.as_slice() {
        ["websites" | "links" | "pixels" | "boards" | "console", id, ..]
        | ["admin" | "settings", "users" | "teams" | "websites", id] => Uuid::parse_str(id).is_ok(),
        _ => false,
    };
    if severed && shell_route {
        candidates.push(app_dir.join("(main)/websites.html"));
        candidates.push(app_dir.join("websites.html"));
    }

    for candidate in candidates {
        if let Some(res) = serve_file_bytes(&candidate, Some("no-cache")).await {
            return Some(res);
        }
    }

    None
}

async fn fallback_proxy(State(state): State<AppState>, req: Request) -> Response {
    let severed = state.node_upstream.is_none();
    let req_method = req.method().clone();
    let req_path = req.uri().path().to_string();
    let has_rsc = req.headers().contains_key("rsc");
    if let Some(static_res) =
        try_serve_static_or_spa(&state.static_root, &req_method, &req_path, has_rsc, severed).await
    {
        return static_res;
    }
    let Some((upstream, client)) = &state.node_upstream else {
        return with_rust_header(StatusCode::NOT_FOUND.into_response());
    };
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");
    let url = format!("{upstream}{path_and_query}");
    let method = req.method().clone();
    let headers = req.headers().clone();
    let Ok(body_bytes) = to_bytes(req.into_body(), 16 * 1024 * 1024).await else {
        return status_response(StatusCode::PAYLOAD_TOO_LARGE);
    };

    let mut rb = client.request(method, &url);
    for (k, v) in &headers {
        if !is_hop_by_hop(k.as_str()) && k != header::ACCEPT_ENCODING {
            rb = rb.header(k.as_str(), v.as_bytes());
        }
    }
    if !body_bytes.is_empty() {
        rb = rb.body(body_bytes);
    }

    match rb.send().await {
        Ok(resp) => convert_reqwest_response(resp).await,
        Err(_) => StatusCode::BAD_GATEWAY.into_response(),
    }
}

pub fn build_router(state: AppState) -> Router {
    let collection = Router::new()
        .route(
            "/api/send",
            post(handle_send).options(|| async { StatusCode::NO_CONTENT }),
        )
        .route(
            "/api/batch",
            post(handle_batch).options(|| async { StatusCode::NO_CONTENT }),
        )
        .route(
            "/v2",
            post(handle_v2).options(|| async { StatusCode::NO_CONTENT }),
        )
        .route(
            "/v2/",
            post(handle_v2).options(|| async { StatusCode::NO_CONTENT }),
        )
        .route(
            "/v2/namespace/{*namespace}",
            post(handle_v2).options(|| async { StatusCode::NO_CONTENT }),
        )
        .layer(axum::middleware::map_response(
            |mut res: Response| async move {
                let headers = res.headers_mut();
                headers.insert(
                    header::ACCESS_CONTROL_ALLOW_ORIGIN,
                    HeaderValue::from_static("*"),
                );
                headers.insert(
                    header::ACCESS_CONTROL_ALLOW_HEADERS,
                    HeaderValue::from_static("*"),
                );
                headers.insert(
                    header::ACCESS_CONTROL_ALLOW_METHODS,
                    HeaderValue::from_static("POST, OPTIONS"),
                );
                headers.insert(
                    header::ACCESS_CONTROL_MAX_AGE,
                    HeaderValue::from_static("86400"),
                );
                headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
                res
            },
        ));
    let router = Router::new()
        .merge(collection)
        .route("/api/heartbeat", get(heartbeat))
        .route("/app-icon", get(handle_app_icon))
        .route("/api/auth/login", post(handle_login))
        .route(
            "/api/websites/{website_id}/metrics",
            get(handle_website_metrics),
        )
        .route(
            "/api/websites/{website_id}/stats",
            get(handle_website_stats),
        );
    let router = dashboard_api::register_dashboard_routes(router);
    let router = analytics_api::register_analytics_routes(router);
    let router = router.route(
        "/api/websites/{website_id}/app-reports/overview",
        get(app_overview::get_app_overview),
    );
    let router = acquisition_report::register_routes(router);
    let router = activation_report::register_routes(router);
    let router = retention_report::register_routes(router);
    let router = app_reports_versions::register_versions_routes(router);
    let router = app_reports_devices::register_devices_routes(router);
    let router = app_settings::register_routes(router);
    let router = app_reports_accessibility::register_accessibility_routes(router);
    let router = event_types::register_event_type_routes(router);
    let router = notebooks::register_routes(router);
    let router = auth::register_routes(router);
    let router = reports::register_routes(router);
    let router = recording::register_routes(router);
    let router = teams::register_routes(router);
    let router = trackables::register_routes(router);
    let router = boards::register_routes(router);
    let router = shares::register_routes(router);
    let router = error_reports::register_routes(router);
    let router = insights::register_routes(router);
    let router = properties::register_routes(router);
    let router = website_ops::register_routes(router);
    let router = workspace_card_metrics::register_routes(router);
    let router = workspace_folders::register_routes(router);
    let router = workspace_poll::register_routes(router);
    let api: Router = router.with_state(state.clone());
    let mcp_api = api.clone();
    let mcp_state = state.clone();
    api.route(
        "/mcp",
        any(move |req: Request| {
            let state = mcp_state.clone();
            let api = mcp_api.clone();
            async move { mcp::handle(state, api, req).await }
        }),
    )
    .fallback(any(fallback_proxy).with_state(state))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    for key in ["CLICKHOUSE_URL", "REDIS_URL", "CLOUD_MODE"] {
        if env::var_os(key).is_some_and(|value| !value.is_empty()) {
            return Err(format!("{key} requires the Node runtime; set MAAP_SERVER=node").into());
        }
    }
    let _ = app_secret();
    let db = Some(
        PgPoolOptions::new()
            .max_connections(10)
            .connect(&env::var("DATABASE_URL")?)
            .await?,
    );

    let allowed_keys: HashSet<String> = env::var("TD_ALLOWED_KEYS")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();

    let td_options = TdOptions {
        hostname: env::var("TD_HOSTNAME").ok().filter(|s| !s.is_empty()),
        test_website_id: env::var("TD_TEST_WEBSITE_ID")
            .ok()
            .filter(|s| !s.is_empty()),
        allowed_keys,
    };

    let node_upstream = if env::var("MAAP_NODE_FALLBACK").ok().as_deref() == Some("1") {
        let port = env::var("MAAP_NODE_PORT").unwrap_or_else(|_| "3001".to_string());
        Some((format!("http://127.0.0.1:{port}"), node_client()?))
    } else {
        None
    };

    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(5))
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("Mozilla/5.0 (compatible; MaapIconBot/1.0)")
        .build()
        .expect("http client");

    let static_root = env::var("MAAP_STATIC_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));

    let state = AppState {
        db,
        http,
        td_options,
        node_upstream,
        static_root,
        icon_cache: Arc::new(RwLock::new(HashMap::new())),
    };

    let host = env::var("HOSTNAME").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);
    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("bind listener");
    println!("maap-server (rust) listening on http://{addr}");
    axum::serve(listener, build_router(state)).await?;
    Ok(())
}

fn node_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(15))
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    #[tokio::test]
    async fn browser_ingestion_cors_covers_preflight_and_rejections() {
        let router = build_router(AppState {
            db: None,
            http: reqwest::Client::new(),
            td_options: TdOptions::default(),
            node_upstream: None,
            static_root: PathBuf::from("."),
            icon_cache: Arc::new(RwLock::new(HashMap::new())),
        });
        for path in [
            "/api/send",
            "/api/batch",
            "/v2",
            "/v2/",
            "/v2/namespace/browser",
            "/v2/namespace/browser/",
        ] {
            let valid_body = if path == "/api/send" {
                r#"{"type":"event","payload":{"website":"11111111-1111-4111-8111-111111111111","url":"/"}}"#
            } else {
                "[]"
            };
            for (method, body, status) in [
                ("OPTIONS", "", StatusCode::NO_CONTENT),
                ("POST", "[", StatusCode::BAD_REQUEST),
                ("POST", valid_body, StatusCode::SERVICE_UNAVAILABLE),
                (
                    "POST",
                    &" ".repeat(2 * 1024 * 1024 + 1),
                    StatusCode::PAYLOAD_TOO_LARGE,
                ),
                ("DELETE", "", StatusCode::METHOD_NOT_ALLOWED),
            ] {
                let response = router
                    .clone()
                    .oneshot(
                        Request::builder()
                            .uri(path)
                            .method(method)
                            .header(header::ORIGIN, "https://browser.example")
                            .header(header::CONTENT_TYPE, "application/json")
                            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                            .header(
                                header::ACCESS_CONTROL_REQUEST_HEADERS,
                                "content-type,x-umami-cache",
                            )
                            .body(Body::from(body.to_owned()))
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(response.status(), status, "{method} {path}");
                assert_eq!(response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
                assert_eq!(response.headers()[header::ACCESS_CONTROL_ALLOW_HEADERS], "*");
                assert_eq!(
                    response.headers()[header::ACCESS_CONTROL_ALLOW_METHODS],
                    "POST, OPTIONS"
                );
                assert!(!response
                    .headers()
                    .contains_key(header::ACCESS_CONTROL_ALLOW_CREDENTIALS));
                if method == "OPTIONS" {
                    assert!(to_bytes(response.into_body(), 1024).await.unwrap().is_empty());
                }
            }
        }

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/me")
                    .header(header::ORIGIN, "https://browser.example")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(!response
            .headers()
            .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN));
    }

    #[test]
    fn malformed_uri_returns_the_original_not_a_partially_decoded_path() {
        for path in ["/My%20App/%zz", "/My%20App/%C3", "/My%20App/%"] {
            assert_eq!(decode_uri(path, true), path);
        }
    }

    #[test]
    fn maps_navigation_and_custom_signals_with_test_mode_and_platforms() {
        let now_ms = DateTime::parse_from_rfc3339("2026-03-01T10:23:00Z")
            .unwrap()
            .timestamp_millis();
        let batch: Vec<Value> = serde_json::from_value(json!([
            {
                "receivedAt": "2026-03-01T10:20:30+0000",
                "appID": "3D452373-9D34-41F4-A80D-5B03ADD59B1C",
                "clientUser": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "sessionID": "DB99DEC6-7289-421E-96BD-E9824658F4BD",
                "type": "TelemetryDeck.Navigation.pathChanged",
                "isTestMode": "false",
                "payload": {
                    "TelemetryDeck.Navigation.destinationPath": "/library",
                    "TelemetryDeck.AppInfo.version": "1.2.3",
                    "TelemetryDeck.AppInfo.buildNumber": "42",
                    "TelemetryDeck.Device.modelName": "iPad14,3",
                    "TelemetryDeck.Device.operatingSystem": "iOS",
                    "TelemetryDeck.Device.systemMajorMinorVersion": "iOS 18.4",
                    "TelemetryDeck.Device.systemVersion": "iOS 18.4.1",
                    "TelemetryDeck.Device.screenResolutionWidth": "834.0",
                    "TelemetryDeck.Device.screenResolutionHeight": "1194.0",
                    "TelemetryDeck.UserPreference.language": "fr-CA",
                    "shelf": "fiction"
                }
            },
            {
                "receivedAt": "2026-03-01T10:20:31+0000",
                "appID": "3D452373-9D34-41F4-A80D-5B03ADD59B1C",
                "clientUser": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "sessionID": "DB99DEC6-7289-421E-96BD-E9824658F4BD",
                "type": "MAAP.acceptanceTapped",
                "isTestMode": "false",
                "floatValue": 2.5,
                "payload": {
                    "TelemetryDeck.AppInfo.version": "1.2.3",
                    "source": "library"
                }
            }
        ]))
        .unwrap();
        let nav = &batch[0];

        let opts = TdOptions::default();
        let SignalDisposition::Mapped(mapped) = map_signal(nav, &opts, now_ms) else {
            panic!("expected mapped navigation");
        };
        assert_eq!(mapped.kind, "event");
        assert_eq!(mapped.payload.name, None);
        assert_eq!(mapped.payload.url.as_deref(), Some("/library"));
        assert_eq!(
            mapped.payload.website.as_deref(),
            Some("3d452373-9d34-41f4-a80d-5b03add59b1c")
        );
        assert_eq!(mapped.payload.hostname.as_deref(), Some("app.ios"));
        assert_eq!(mapped.payload.device.as_deref(), Some("tablet"));
        assert_eq!(mapped.payload.os.as_deref(), Some("iOS 18.4"));
        assert_eq!(mapped.payload.screen.as_deref(), Some("834x1194"));
        assert_eq!(mapped.payload.language.as_deref(), Some("fr-CA"));
        assert_eq!(mapped.payload.browser.as_deref(), Some("App 1.2.3"));
        assert_eq!(mapped.payload.tag, None);
        assert_eq!(mapped.payload.timestamp, None);
        assert_eq!(
            mapped.payload.data.as_ref().unwrap().get("shelf"),
            Some(&Value::String("fiction".to_string()))
        );

        // Test mode tags event with "test" on same website
        let mut test_sig = batch[1].clone();
        test_sig["isTestMode"] = json!("true");
        test_sig["receivedAt"] = json!("2026-03-01T10:10:00+0000");
        let SignalDisposition::Mapped(mapped_test) = map_signal(&test_sig, &opts, now_ms) else {
            panic!("expected mapped test signal");
        };
        assert_eq!(mapped_test.payload.tag.as_deref(), Some("test"));
        assert_eq!(
            mapped_test.payload.name.as_deref(),
            Some("MAAP.acceptanceTapped")
        );
        assert!(mapped_test.payload.timestamp.is_some());
        assert_eq!(
            mapped_test.payload.data.as_ref().unwrap().get("floatValue"),
            Some(&json!(2.5))
        );

        // macOS and Android platform mapping
        let mac_sig = json!({
            "appID": "CF80C54B-E250-4D04-BA0F-568C176EAD2B",
            "type": "AppLaunched",
            "clientUser": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "payload": {
                "TelemetryDeck.Device.operatingSystem": "macOS",
                "TelemetryDeck.Device.systemMajorMinorVersion": "macOS 15.3",
                "TelemetryDeck.Device.systemVersion": "macOS 15.3.1",
                "TelemetryDeck.Device.modelName": "Mac16,1"
            }
        });
        let SignalDisposition::Mapped(mapped_mac) =
            map_signal(&mac_sig, &TdOptions::default(), now_ms)
        else {
            panic!("expected mac signal");
        };
        assert_eq!(mapped_mac.payload.hostname.as_deref(), Some("app.macos"));
        assert_eq!(mapped_mac.payload.device.as_deref(), Some("desktop"));
        assert_eq!(mapped_mac.payload.os.as_deref(), Some("macOS 15.3"));

        let android_sig = json!({
            "appID": "CF80C54B-E250-4D04-BA0F-568C176EAD2B",
            "type": "AppLaunched",
            "clientUser": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "payload": {
                "TelemetryDeck.Device.operatingSystem": "Android",
                "TelemetryDeck.Device.systemMajorMinorVersion": "Android 15",
                "TelemetryDeck.Device.systemVersion": "Android 15.0.1",
                "TelemetryDeck.Device.modelName": "Pixel 9"
            }
        });
        let SignalDisposition::Mapped(mapped_android) =
            map_signal(&android_sig, &TdOptions::default(), now_ms)
        else {
            panic!("expected android signal");
        };
        assert_eq!(
            mapped_android.payload.hostname.as_deref(),
            Some("app.android")
        );
        assert_eq!(mapped_android.payload.device.as_deref(), Some("mobile"));
        assert_eq!(mapped_android.payload.os.as_deref(), Some("Android 15"));
    }

    #[test]
    fn retains_app_sdk_and_device_report_values_per_event_without_widening_the_allowlist() {
        let now_ms = DateTime::parse_from_rfc3339("2026-03-01T10:23:00Z")
            .unwrap()
            .timestamp_millis();
        let client_user = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let first_signal = json!({
            "appID": "3D452373-9D34-41F4-A80D-5B03ADD59B1C",
            "clientUser": client_user,
            "type": "TelemetryDeck.Navigation.pathChanged",
            "payload": {
                "TelemetryDeck.Navigation.destinationPath": "/first",
                "TelemetryDeck.AppInfo.version": "1.2.3",
                "TelemetryDeck.AppInfo.buildNumber": "42",
                "TelemetryDeck.SDK.name": "SwiftSDK",
                "TelemetryDeck.SDK.version": "2.9.0",
                "TelemetryDeck.Device.operatingSystem": "iOS",
                "TelemetryDeck.Device.platform": "iOS",
                "TelemetryDeck.Device.orientation": "portrait",
                "TelemetryDeck.Device.screenResolutionWidth": "834.0",
                "TelemetryDeck.Device.screenResolutionHeight": "1194.0",
                "TelemetryDeck.RunContext.language": "en",
                "TelemetryDeck.UserPreference.colorScheme": "dark",
                "TelemetryDeck.UserPreference.language": "fr",
                "TelemetryDeck.UserPreference.region": "CA",
                "TelemetryDeck.UserPreference.layoutDirection": "leftToRight",
                "TelemetryDeck.Error.category": "thrown-exception",
                "TelemetryDeck.Error.id": "ImportObject.jsonDecode",
                "TelemetryDeck.Error.message": "/Users/alice/private.json could not be decoded",
                "TelemetryDeck.Accessibility.isBoldTextEnabled": "true",
                "TelemetryDeck.SDK.nameAndVersion": "SwiftSDK 2.9.0",
                "TelemetryDeck.Private.installID": "private-first",
                "releaseChannel": "stable"
            }
        });
        let second_signal = json!({
            "appID": "3D452373-9D34-41F4-A80D-5B03ADD59B1C",
            "clientUser": client_user,
            "type": "MAAP.secondLaunch",
            "payload": {
                "TelemetryDeck.AppInfo.version": "1.3.0",
                "TelemetryDeck.AppInfo.buildNumber": "43",
                "TelemetryDeck.SDK.version": "3.0.0",
                "TelemetryDeck.Device.operatingSystem": "iOS",
                "TelemetryDeck.Device.platform": "iOS",
                "TelemetryDeck.Device.orientation": "landscape",
                "TelemetryDeck.Device.screenResolutionWidth": "1194.0",
                "TelemetryDeck.Device.screenResolutionHeight": "834.0",
                "TelemetryDeck.UserPreference.language": "de",
                "TelemetryDeck.UserPreference.region": "DE",
                "TelemetryDeck.UserPreference.layoutDirection": "rightToLeft",
                "TelemetryDeck.UserPreference.colorScheme": "light",
                "TelemetryDeck.Localization.locale": "de_DE",
                "TelemetryDeck.Private.installID": "private-second",
                "releaseChannel": "beta"
            }
        });

        let SignalDisposition::Mapped(first) =
            map_signal(&first_signal, &TdOptions::default(), now_ms)
        else {
            panic!("expected first signal");
        };
        let SignalDisposition::Mapped(second) =
            map_signal(&second_signal, &TdOptions::default(), now_ms)
        else {
            panic!("expected second signal");
        };
        assert_eq!(first.payload.name, None);
        assert_eq!(second.payload.name.as_deref(), Some("MAAP.secondLaunch"));

        let first_data = first.payload.data.as_ref().unwrap();
        assert_eq!(
            first_data.get("TelemetryDeck.AppInfo.version"),
            Some(&json!("1.2.3"))
        );
        assert_eq!(
            first_data.get("TelemetryDeck.AppInfo.buildNumber"),
            Some(&json!("42"))
        );
        assert_eq!(
            first_data.get("TelemetryDeck.SDK.name"),
            Some(&json!("SwiftSDK"))
        );
        assert_eq!(
            first_data.get("TelemetryDeck.SDK.version"),
            Some(&json!("2.9.0"))
        );
        assert_eq!(
            first_data.get("TelemetryDeck.Device.operatingSystem"),
            Some(&json!("iOS"))
        );
        assert_eq!(
            first_data.get("TelemetryDeck.Device.platform"),
            Some(&json!("iOS"))
        );
        assert_eq!(
            first_data.get("TelemetryDeck.Device.orientation"),
            Some(&json!("portrait"))
        );
        assert_eq!(
            first_data.get("TelemetryDeck.Device.screenResolutionWidth"),
            Some(&json!("834.0"))
        );
        assert_eq!(
            first_data.get("TelemetryDeck.Device.screenResolutionHeight"),
            Some(&json!("1194.0"))
        );
        assert_eq!(
            first_data.get("TelemetryDeck.UserPreference.colorScheme"),
            Some(&json!("dark"))
        );
        assert_eq!(
            first_data.get("TelemetryDeck.RunContext.language"),
            Some(&json!("en"))
        );
        assert_eq!(
            first_data.get("TelemetryDeck.UserPreference.language"),
            Some(&json!("fr"))
        );
        assert_eq!(
            first_data.get("TelemetryDeck.UserPreference.region"),
            Some(&json!("CA"))
        );
        assert_eq!(
            first_data.get("TelemetryDeck.UserPreference.layoutDirection"),
            Some(&json!("leftToRight"))
        );
        assert_eq!(
            first_data.get("TelemetryDeck.Error.category"),
            Some(&json!("thrown-exception"))
        );
        assert_eq!(first_data.get("releaseChannel"), Some(&json!("stable")));
        assert!(!first_data.contains_key("TelemetryDeck.SDK.nameAndVersion"));
        assert!(!first_data.contains_key("TelemetryDeck.Error.id"));
        assert!(!first_data.contains_key("TelemetryDeck.Error.message"));
        assert!(!first_data.contains_key("TelemetryDeck.Accessibility.isBoldTextEnabled"));
        assert!(!first_data.contains_key("TelemetryDeck.Private.installID"));

        let second_data = second.payload.data.as_ref().unwrap();
        assert_eq!(
            second_data.get("TelemetryDeck.AppInfo.version"),
            Some(&json!("1.3.0"))
        );
        assert_eq!(
            second_data.get("TelemetryDeck.AppInfo.buildNumber"),
            Some(&json!("43"))
        );
        assert_eq!(
            second_data.get("TelemetryDeck.SDK.version"),
            Some(&json!("3.0.0"))
        );
        assert_eq!(
            second_data.get("TelemetryDeck.Device.operatingSystem"),
            Some(&json!("iOS"))
        );
        assert_eq!(
            second_data.get("TelemetryDeck.Device.platform"),
            Some(&json!("iOS"))
        );
        assert_eq!(
            second_data.get("TelemetryDeck.Device.orientation"),
            Some(&json!("landscape"))
        );
        assert_eq!(
            second_data.get("TelemetryDeck.Device.screenResolutionWidth"),
            Some(&json!("1194.0"))
        );
        assert_eq!(
            second_data.get("TelemetryDeck.Device.screenResolutionHeight"),
            Some(&json!("834.0"))
        );
        assert_eq!(
            second_data.get("TelemetryDeck.UserPreference.colorScheme"),
            Some(&json!("light"))
        );
        assert_eq!(
            second_data.get("TelemetryDeck.UserPreference.language"),
            Some(&json!("de"))
        );
        assert_eq!(
            second_data.get("TelemetryDeck.UserPreference.region"),
            Some(&json!("DE"))
        );
        assert_eq!(
            second_data.get("TelemetryDeck.UserPreference.layoutDirection"),
            Some(&json!("rightToLeft"))
        );
        assert_eq!(second_data.get("releaseChannel"), Some(&json!("beta")));
        assert!(!second_data.contains_key("TelemetryDeck.SDK.name"));
        assert!(!second_data.contains_key("TelemetryDeck.RunContext.language"));
        assert!(!second_data.contains_key("TelemetryDeck.Error.category"));
        assert!(!second_data.contains_key("TelemetryDeck.Localization.locale"));
        assert!(!second_data.contains_key("TelemetryDeck.Private.installID"));

        let mut fields = Vec::new();
        flatten_event_data(second_data, "", &mut fields);
        for key in [
            "TelemetryDeck.AppInfo.version",
            "TelemetryDeck.AppInfo.buildNumber",
            "TelemetryDeck.SDK.version",
            "TelemetryDeck.Device.operatingSystem",
            "TelemetryDeck.Device.platform",
            "TelemetryDeck.Device.orientation",
            "TelemetryDeck.Device.screenResolutionWidth",
            "TelemetryDeck.Device.screenResolutionHeight",
            "TelemetryDeck.UserPreference.language",
            "TelemetryDeck.UserPreference.region",
            "TelemetryDeck.UserPreference.layoutDirection",
            "TelemetryDeck.UserPreference.colorScheme",
        ] {
            let field = fields.iter().find(|field| field.key == key).unwrap();
            assert_eq!(field.data_type, 1);
            assert!(field.string_value.is_some());
        }

        let options = TdOptions {
            allowed_keys: HashSet::from([
                "releaseChannel".to_string(),
                "TelemetryDeck.Error.id".to_string(),
                "TelemetryDeck.Accessibility.isBoldTextEnabled".to_string(),
            ]),
            ..Default::default()
        };
        let SignalDisposition::Mapped(replaced) = map_signal(&first_signal, &options, now_ms)
        else {
            panic!("expected replacement allowlist signal");
        };
        assert_eq!(
            replaced.payload.data,
            Some(Map::from_iter([
                ("releaseChannel".to_string(), json!("stable")),
                (
                    "TelemetryDeck.Error.id".to_string(),
                    json!("ImportObject.jsonDecode")
                ),
                (
                    "TelemetryDeck.Accessibility.isBoldTextEnabled".to_string(),
                    json!("true")
                )
            ]))
        );
    }

    #[test]
    fn secure_token_roundtrip_and_cloudflare_headers() {
        let secret = sha512_hex(&["unit-test-secret"]);
        let claims = json!({ "userId": "92807305-f74b-4e88-8ba9-c85688f7cc91", "role": "admin" });
        let tok = create_secure_token(&claims, &secret).expect("create token");
        let parsed = parse_secure_token(&tok, &secret).expect("parse token");
        assert_eq!(
            parsed.get("userId").and_then(|v| v.as_str()),
            Some("92807305-f74b-4e88-8ba9-c85688f7cc91")
        );

        let mut headers = HeaderMap::new();
        headers.insert("cf-ipcountry", HeaderValue::from_static("US"));
        headers.insert("cf-region-code", HeaderValue::from_static("WA"));
        headers.insert("cf-ipcity", HeaderValue::from_static("Seattle"));
        let (country, region, city) = extract_location_headers(&headers);
        assert_eq!(country.as_deref(), Some("US"));
        assert_eq!(region.as_deref(), Some("US-WA"));
        assert_eq!(city.as_deref(), Some("Seattle"));
    }

    #[test]
    fn fallback_os_icon_selects_matching_platform() {
        assert_eq!(
            fallback_os_icon("https://apps.apple.com/us/app/xpenshare/id6741827585"),
            "/images/os/ios.png"
        );
        assert_eq!(
            fallback_os_icon("https://apps.apple.com/app/id6741827585?mt=12"),
            "/images/os/mac-os.png"
        );
        assert_eq!(
            fallback_os_icon("https://play.google.com/store/apps/details?id=com.example.app"),
            "/images/os/android-os.png"
        );
    }

    #[tokio::test]
    async fn reverse_proxy_strips_chunked_transfer_encoding_cleanly() {
        let upstream_app = Router::new()
            .route(
                "/api/heartbeat",
                get(|| async { Json(json!({ "ok": true })) }),
            )
            .route(
                "/api/custom-fallback-test",
                post(|Json(body): Json<Value>| async move {
                    Json(json!({ "token": "tok-123", "echo": body })).into_response()
                }),
            )
            .route(
                "/redirect-test",
                get(|| async {
                    (
                        StatusCode::TEMPORARY_REDIRECT,
                        axum::response::AppendHeaders([
                            (header::LOCATION, "/login"),
                            (header::SET_COOKIE, "first=1"),
                            (header::SET_COOKIE, "second=2"),
                        ]),
                    )
                }),
            );
        let up_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let up_addr = up_listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(up_listener, upstream_app).await.unwrap();
        });

        let state = AppState {
            db: None,
            http: reqwest::Client::new(),
            td_options: TdOptions::default(),
            node_upstream: Some((format!("http://{up_addr}"), node_client().unwrap())),
            static_root: PathBuf::from("."),
            icon_cache: Arc::new(RwLock::new(HashMap::new())),
        };
        let proxy_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = proxy_listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(proxy_listener, build_router(state))
                .await
                .unwrap();
        });

        let client = node_client().unwrap();
        let redirect = client
            .get(format!("http://{proxy_addr}/redirect-test"))
            .send()
            .await
            .unwrap();
        assert_eq!(redirect.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(redirect.headers()[header::LOCATION], "/login");
        assert_eq!(
            redirect
                .headers()
                .get_all(header::SET_COOKIE)
                .iter()
                .count(),
            2
        );
        let hb = client
            .get(format!("http://{proxy_addr}/api/heartbeat"))
            .send()
            .await
            .unwrap();
        assert_eq!(hb.status(), StatusCode::OK);
        assert_eq!(
            hb.headers()
                .get("x-maap-server")
                .and_then(|v| v.to_str().ok()),
            Some("rust")
        );

        let login = client
            .post(format!("http://{proxy_addr}/api/custom-fallback-test"))
            .json(&json!({ "username": "admin", "password": "umami" }))
            .send()
            .await
            .unwrap();
        assert_eq!(login.status(), StatusCode::OK);
        let payload: Value = login.json().await.unwrap();
        assert_eq!(
            payload.get("token").and_then(|v| v.as_str()),
            Some("tok-123")
        );
    }

    #[tokio::test]
    async fn serves_static_app_routes_and_assets_when_node_is_severed() {
        let dir = std::env::temp_dir().join(format!("maap-static-test-{}", Uuid::new_v4()));
        tokio::fs::create_dir_all(dir.join(".next/server/app/(main)"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(dir.join(".next/static/chunks"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(dir.join("public/images/os"))
            .await
            .unwrap();

        tokio::fs::write(
            dir.join(".next/server/app/login.html"),
            "<!doctype html><html><head><title>Login | Maap</title></head><body><script src=\"/_next/static/chunks/app.js\"></script></body></html>",
        )
        .await
        .unwrap();
        tokio::fs::write(
            dir.join(".next/server/app/(main)/websites.html"),
            "<!doctype html><html><head><title>Websites | Maap</title></head><body>Maap SPA</body></html>",
        )
        .await
        .unwrap();
        tokio::fs::write(
            dir.join(".next/static/chunks/app.js"),
            "console.log('maap-static');",
        )
        .await
        .unwrap();
        tokio::fs::write(dir.join("public/images/os/ios.png"), b"\x89PNG\r\n\x1a\n")
            .await
            .unwrap();

        let state = AppState {
            db: None,
            http: reqwest::Client::new(),
            td_options: TdOptions::default(),
            node_upstream: None,
            static_root: dir.clone(),
            icon_cache: Arc::new(RwLock::new(HashMap::new())),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, build_router(state)).await.unwrap();
        });

        let client = reqwest::Client::new();

        // 1. Pre-rendered /login route served directly by Rust with Node severed
        let login_res = client
            .get(format!("http://{addr}/login"))
            .send()
            .await
            .unwrap();
        assert_eq!(login_res.status(), StatusCode::OK);
        assert_eq!(login_res.headers().get("x-maap-server").unwrap(), "rust");
        let login_html = login_res.text().await.unwrap();
        assert!(login_html.contains("Login | Maap"));

        // 2. Route-group /(main)/websites route & dynamic SPA fallback /websites/<uuid>
        for route in [
            "/websites",
            "/websites/cf80c54b-e250-4d04-ba0f-568c176ead2b",
        ] {
            let res = client
                .get(format!("http://{addr}{route}"))
                .send()
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK);
            assert_eq!(res.headers().get("x-maap-server").unwrap(), "rust");
            assert!(res.text().await.unwrap().contains("Websites | Maap"));
        }

        // 3. Immutable /_next/static chunk served by Rust
        let chunk_res = client
            .get(format!("http://{addr}/_next/static/chunks/app.js"))
            .send()
            .await
            .unwrap();
        assert_eq!(chunk_res.status(), StatusCode::OK);
        assert_eq!(chunk_res.headers().get("x-maap-server").unwrap(), "rust");
        assert_eq!(
            chunk_res.headers().get(header::CACHE_CONTROL).unwrap(),
            "public, max-age=31536000, immutable"
        );

        // 4. /app-icon 302 fallback followed all the way to /images/os/ios.png served by Rust
        let icon_res = client
            .get(format!(
                "http://{addr}/app-icon?url=https://apps.apple.com/app/id999999999"
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(icon_res.status(), StatusCode::OK);
        assert_eq!(icon_res.headers().get("x-maap-server").unwrap(), "rust");
        assert_eq!(
            icon_res.bytes().await.unwrap().as_ref(),
            b"\x89PNG\r\n\x1a\n"
        );

        let _ = tokio::fs::remove_dir_all(dir).await;
    }
}
