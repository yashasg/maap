use aes::Aes256;
use aes_gcm::{
    aead::{consts::U16, Aead, KeyInit},
    AesGcm, Nonce,
};
use axum::{
    body::{to_bytes, Body},
    extract::{Path, Query, Request, State},
    http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode},
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
    str::FromStr,
    sync::{Arc, OnceLock},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::RwLock;
use url::Url;
use uuid::Uuid;

type Aes256Gcm16 = AesGcm<Aes256, U16>;

const HISTORICAL_CUTOFF_MS: i64 = 5 * 60 * 1000;
const ICON_CACHE_TTL_SECS: u64 = 7 * 24 * 60 * 60;
const ICON_CACHE_CONTROL: &str = "public, max-age=86400, s-maxage=604800";

#[derive(Clone)]
pub struct AppState {
    pub db: Option<PgPool>,
    pub http: reqwest::Client,
    pub td_options: TdOptions,
    pub node_upstream: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SendEnvelope {
    #[serde(rename = "type")]
    pub kind: String,
    pub payload: SendPayload,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SendPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screen: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub referrer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Map<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,
}

pub fn sha512_hex(parts: &[&str]) -> String {
    let mut hasher = Sha512::new();
    for p in parts {
        hasher.update(p.as_bytes());
    }
    hex::encode(hasher.finalize())
}

pub fn app_secret() -> String {
    for key in ["APP_SECRET", "DATABASE_URL"] {
        if let Ok(val) = env::var(key) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    "secret".to_string()
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

pub fn get_salt(created_at: DateTime<Utc>) -> String {
    if let Ok(rot) = env::var("SALT_ROTATION") {
        let trimmed = rot.trim();
        if !trimmed.is_empty() && !matches!(trimmed.to_ascii_lowercase().as_str(), "month" | "monthly") {
            return sha512_hex(&[trimmed]);
        }
    }
    sha512_hex(&[&to_js_utc_string(start_of_month_utc(created_at))])
}

pub fn resolve_session_and_visit(
    website_id: &str,
    ip: &str,
    user_agent: &str,
    distinct_id: &str,
    created_at: DateTime<Utc>,
) -> (Uuid, Uuid, i64) {
    let now = Utc::now().timestamp();
    let session_salt = get_salt(created_at);
    let visit_salt = sha512_hex(&[&to_js_utc_string(start_of_hour_utc(created_at))]);
    let session_id = deterministic_uuid(&[website_id, ip, user_agent, &session_salt, distinct_id]);
    let visit_id = deterministic_uuid(&[&session_id.to_string(), &visit_salt]);
    (session_id, visit_id, now)
}

pub fn create_secure_token(claims: &Value, secret: &str) -> Option<String> {
    let mut claims_obj = claims.as_object()?.clone();
    if !claims_obj.contains_key("iat") {
        claims_obj.insert("iat".to_string(), json!(Utc::now().timestamp()));
    }
    let jwt = encode(
        &Header::new(Algorithm::HS256),
        &Value::Object(claims_obj),
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .ok()?;

    let mut salt = [0u8; 64];
    let mut iv = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut salt);
    rand::thread_rng().fill_bytes(&mut iv);

    let mut key = [0u8; 32];
    pbkdf2_hmac::<Sha512>(secret.as_bytes(), &salt, 10_000, &mut key);
    let cipher = Aes256Gcm16::new_from_slice(&key).ok()?;
    let ciphertext_with_tag = cipher
        .encrypt(Nonce::from_slice(&iv), jwt.as_bytes())
        .ok()?;
    if ciphertext_with_tag.len() < 16 {
        return None;
    }
    let split = ciphertext_with_tag.len() - 16;
    let (encrypted, tag) = ciphertext_with_tag.split_at(split);

    let mut out = Vec::with_capacity(64 + 16 + 16 + encrypted.len());
    out.extend_from_slice(&salt);
    out.extend_from_slice(&iv);
    out.extend_from_slice(tag);
    out.extend_from_slice(encrypted);
    Some(BASE64.encode(out))
}

pub fn parse_secure_token(token: &str, secret: &str) -> Option<Value> {
    let raw = BASE64.decode(token.trim()).ok()?;
    if raw.len() < 96 {
        return None;
    }
    let salt = &raw[0..64];
    let iv = &raw[64..80];
    let tag = &raw[80..96];
    let encrypted = &raw[96..];

    let mut key = [0u8; 32];
    pbkdf2_hmac::<Sha512>(secret.as_bytes(), salt, 10_000, &mut key);
    let cipher = Aes256Gcm16::new_from_slice(&key).ok()?;

    let mut combined = Vec::with_capacity(encrypted.len() + 16);
    combined.extend_from_slice(encrypted);
    combined.extend_from_slice(tag);

    let decrypted = cipher.decrypt(Nonce::from_slice(iv), combined.as_ref()).ok()?;
    let jwt_str = String::from_utf8(decrypted).ok()?;

    let mut validation = Validation::new(Algorithm::HS256);
    validation.required_spec_claims.clear();
    validation.validate_exp = false;
    let decoded = decode::<Value>(
        &jwt_str,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .ok()?;
    Some(decoded.claims)
}

fn parse_sdk_timestamp_ms(value: Option<&str>) -> Option<i64> {
    let s = value?.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp_millis());
    }
    // Support "+0000" / "-0700" without colon (e.g. "2026-03-01T10:15:00+0000")
    static TZ_RE: OnceLock<Regex> = OnceLock::new();
    let re = TZ_RE.get_or_init(|| Regex::new(r"([+-]\d{2})(\d{2})$").unwrap());
    let normalized = re.replace(s, "$1:$2");
    DateTime::parse_from_rfc3339(&normalized)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

fn dimension(val: Option<&String>) -> Option<u32> {
    let n: f64 = val?.parse().ok()?;
    if n.is_finite() && n > 0.0 && n <= 99999.0 {
        Some(n.round() as u32)
    } else {
        None
    }
}

pub fn map_signal(signal: &Value, options: &TdOptions, now_ms: i64) -> SignalDisposition {
    let Some(obj) = signal.as_object() else {
        return SignalDisposition::Rejected;
    };
    let Some(sig_type_raw) = obj.get("type").and_then(|v| v.as_str()) else {
        return SignalDisposition::Rejected;
    };
    let Some(app_id_raw) = obj.get("appID").and_then(|v| v.as_str()) else {
        return SignalDisposition::Rejected;
    };
    if sig_type_raw.trim().is_empty() || app_id_raw.trim().is_empty() {
        return SignalDisposition::Rejected;
    }

    let is_test = match obj.get("isTestMode") {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) if s == "true" => true,
        Some(Value::String(s)) if s == "false" => false,
        _ => return SignalDisposition::Rejected,
    };

    let test_website = options
        .test_website_id
        .as_deref()
        .filter(|s| !s.is_empty());

    let website = if is_test {
        if let Some(tw) = test_website {
            if tw.eq_ignore_ascii_case("drop") {
                return SignalDisposition::Dropped;
            }
            tw.to_ascii_lowercase()
        } else {
            app_id_raw.to_ascii_lowercase()
        }
    } else {
        app_id_raw.to_ascii_lowercase()
    };

    let mut props = HashMap::new();
    match obj.get("payload") {
        None => {}
        Some(Value::Object(map)) => {
            for (k, v) in map {
                let Some(s) = v.as_str() else {
                    return SignalDisposition::Rejected;
                };
                props.insert(k.clone(), s.to_string());
            }
        }
        _ => return SignalDisposition::Rejected,
    }

    const DEFAULT_SDK_KEYS: &[&str] = &[
        "TelemetryDeck.AppInfo.version",
        "TelemetryDeck.AppInfo.buildNumber",
        "TelemetryDeck.Device.modelName",
        "TelemetryDeck.Device.systemVersion",
    ];

    let mut data = Map::new();
    for (key, value) in &props {
        let allowed = if !options.allowed_keys.is_empty() {
            options.allowed_keys.contains(key)
        } else {
            !key.starts_with("TelemetryDeck.") || DEFAULT_SDK_KEYS.contains(&key.as_str())
        };
        if allowed {
            data.insert(key.clone(), Value::String(value.clone()));
        }
    }

    if let Some(fv_val) = obj.get("floatValue") {
        let Some(fv) = fv_val.as_f64() else {
            return SignalDisposition::Rejected;
        };
        if !fv.is_finite() {
            return SignalDisposition::Rejected;
        }
        if let Some(num) = serde_json::Number::from_f64(fv) {
            data.insert("floatValue".to_string(), Value::Number(num));
        }
    }

    static HEX64_RE: OnceLock<Regex> = OnceLock::new();
    let hex64_re = HEX64_RE.get_or_init(|| Regex::new(r"(?i)^[0-9a-f]{64}$").unwrap());
    let id = match obj.get("clientUser") {
        None => None,
        Some(Value::String(s)) if hex64_re.is_match(s) => Some(s.clone()),
        _ => return SignalDisposition::Rejected,
    };

    let timestamp = match obj.get("receivedAt") {
        None => None,
        Some(Value::String(s)) => {
            static ISO_RE: OnceLock<Regex> = OnceLock::new();
            let iso_re = ISO_RE.get_or_init(|| {
                Regex::new(r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?(?:Z|[+-]\d\d:?\d\d)$")
                    .unwrap()
            });
            if !iso_re.is_match(s) {
                return SignalDisposition::Rejected;
            }
            let Some(ms) = parse_sdk_timestamp_ms(Some(s)) else {
                return SignalDisposition::Rejected;
            };
            if now_ms - ms > HISTORICAL_CUTOFF_MS {
                Some(ms.div_euclid(1000))
            } else {
                None
            }
        }
        _ => return SignalDisposition::Rejected,
    };

    let model = props
        .get("TelemetryDeck.Device.modelName")
        .or_else(|| props.get("modelName"))
        .map(String::as_str);
    let app_version = props
        .get("TelemetryDeck.AppInfo.version")
        .or_else(|| props.get("appVersion"))
        .map(String::as_str);
    let os_name = props
        .get("TelemetryDeck.Device.operatingSystem")
        .or_else(|| props.get("operatingSystem"))
        .map(String::as_str);

    let is_mac = os_name
        .map(|s| s.to_ascii_lowercase().starts_with("mac"))
        .unwrap_or(false)
        || model.map(|m| m.starts_with("Mac")).unwrap_or(false);
    let is_android = os_name
        .map(|s| s.to_ascii_lowercase().starts_with("android"))
        .unwrap_or(false);
    let device = if is_mac {
        "desktop"
    } else if model.map(|m| m.starts_with("iPad")).unwrap_or(false) {
        "tablet"
    } else {
        "mobile"
    };

    let version = props
        .get("TelemetryDeck.Device.systemMajorMinorVersion")
        .or_else(|| props.get("majorMinorSystemVersion"))
        .map(String::as_str);

    static VER_SUFFIX_RE: OnceLock<Regex> = OnceLock::new();
    static AGENT_VER_RE: OnceLock<Regex> = OnceLock::new();
    let ver_suffix_re = VER_SUFFIX_RE.get_or_init(|| Regex::new(r"\d+(?:\.\d+)?$").unwrap());
    let agent_ver_re = AGENT_VER_RE.get_or_init(|| Regex::new(r"\d+(?:\.\d+){0,2}$").unwrap());

    let major_minor = version.and_then(|v| ver_suffix_re.find(v)).map(|m| m.as_str());
    let os_joined: String = [os_name, major_minor]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let os = if os_joined.is_empty() {
        None
    } else {
        Some(os_joined)
    };

    let sys_ver_prop = props
        .get("TelemetryDeck.Device.systemVersion")
        .or_else(|| props.get("systemVersion"))
        .map(String::as_str);
    let agent_version = sys_ver_prop
        .and_then(|s| agent_ver_re.find(s))
        .map(|m| m.as_str())
        .or(major_minor)
        .unwrap_or("17.0");
    let mm_or_agent = major_minor.unwrap_or(agent_version);
    let agent_device = if device == "tablet" {
        "iPad; CPU OS"
    } else {
        "iPhone; CPU iPhone OS"
    };

    let user_agent = if is_mac {
        format!(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X {}) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/{} Safari/605.1.15",
            agent_version.replace('.', "_"),
            mm_or_agent
        )
    } else if is_android {
        let m = model.filter(|s| !s.is_empty()).unwrap_or("Pixel");
        format!(
            "Mozilla/5.0 (Linux; Android {agent_version}; {m}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Mobile Safari/537.36"
        )
    } else {
        format!(
            "Mozilla/5.0 ({agent_device} {} like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/{mm_or_agent} Mobile/15E148 Safari/604.1",
            agent_version.replace('.', "_")
        )
    };

    let width = dimension(props.get("TelemetryDeck.Device.screenResolutionWidth"));
    let height = dimension(props.get("TelemetryDeck.Device.screenResolutionHeight"));
    let screen = match (width, height) {
        (Some(w), Some(h)) => Some(format!("{w}x{h}")),
        _ => None,
    };

    let nav = sig_type_raw == "TelemetryDeck.Navigation.pathChanged";
    let destination = if nav {
        props
            .get("TelemetryDeck.Navigation.destinationPath")
            .map(String::as_str)
            .unwrap_or("")
    } else {
        ""
    };
    let clean_dest = destination.trim_start_matches(['/', '\\']);
    let url = format!("/{clean_dest}");

    let hostname = options
        .hostname
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            if is_mac {
                "app.macos".to_string()
            } else if is_android {
                "app.android".to_string()
            } else {
                "app.ios".to_string()
            }
        });

    let language = props
        .get("TelemetryDeck.UserPreference.language")
        .or_else(|| props.get("preferredLanguage"))
        .cloned();

    SignalDisposition::Mapped(SendEnvelope {
        kind: "event".to_string(),
        payload: SendPayload {
            website: Some(website),
            hostname: Some(hostname),
            id,
            timestamp,
            user_agent: Some(user_agent),
            browser: app_version.map(|v| format!("App {v}")),
            os,
            device: Some(device.to_string()),
            screen,
            language,
            url: Some(url),
            referrer: None,
            title: None,
            name: if nav { None } else { Some(sig_type_raw.to_string()) },
            tag: if is_test && test_website.is_none() {
                Some("test".to_string())
            } else {
                None
            },
            data: Some(data),
            ip: None,
        },
    })
}

pub fn extract_client_ip(headers: &HeaderMap) -> String {
    for name in [
        "cf-connecting-ip",
        "true-client-ip",
        "x-real-ip",
        "x-forwarded-for",
        "x-client-ip",
    ] {
        if let Some(val) = headers.get(name).and_then(|v| v.to_str().ok()) {
            if let Some(first) = val.split(',').next() {
                let trimmed = first.trim();
                if !trimmed.is_empty() {
                    return trimmed.to_string();
                }
            }
        }
    }
    "127.0.0.1".to_string()
}

pub fn extract_location_headers(headers: &HeaderMap) -> (Option<String>, Option<String>, Option<String>) {
    if env::var("SKIP_LOCATION_HEADERS").is_ok() {
        return (None, None, None);
    }
    let providers = [
        ("cf-ipcountry", "cf-region-code", "cf-ipcity"),
        ("x-vercel-ip-country", "x-vercel-ip-country-region", "x-vercel-ip-city"),
        ("cloudfront-viewer-country", "cloudfront-viewer-country-region", "cloudfront-viewer-city"),
        ("eo-ipcountry", "eo-region-code", "eo-ipcity"),
    ];
    for (c_hdr, r_hdr, city_hdr) in providers {
        if let Some(country) = headers
            .get(c_hdr)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let region = headers
                .get(r_hdr)
                .and_then(|v| v.to_str().ok())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|r| {
                    if r.contains('-') {
                        r.to_string()
                    } else {
                        format!("{country}-{r}")
                    }
                });
            let city = headers
                .get(city_hdr)
                .and_then(|v| v.to_str().ok())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            return (Some(country.to_string()), region, city);
        }
    }
    (None, None, None)
}

#[derive(Debug, Clone, PartialEq)]
pub struct FlattenedField {
    pub key: String,
    pub data_type: i32,
    pub string_value: Option<String>,
    pub number_value: Option<Decimal>,
    pub date_value: Option<DateTime<Utc>>,
}

pub fn flatten_event_data(map: &Map<String, Value>, prefix: &str, out: &mut Vec<FlattenedField>) {
    for (k, v) in map {
        let full_key = if prefix.is_empty() {
            k.clone()
        } else {
            format!("{prefix}.{k}")
        };
        let trunc_key: String = full_key.chars().take(500).collect();
        match v {
            Value::Object(nested) => flatten_event_data(nested, &full_key, out),
            Value::Number(n) => {
                if let Some(f) = n.as_f64() {
                    if f.is_finite() {
                        let s = format!("{f:.4}");
                        let dec = Decimal::from_str(&s).ok();
                        out.push(FlattenedField {
                            key: trunc_key,
                            data_type: 2,
                            string_value: Some(s.chars().take(500).collect()),
                            number_value: dec,
                            date_value: None,
                        });
                    }
                }
            }
            Value::Bool(b) => {
                out.push(FlattenedField {
                    key: trunc_key,
                    data_type: 3,
                    string_value: Some(if *b { "true" } else { "false" }.to_string()),
                    number_value: None,
                    date_value: None,
                });
            }
            Value::Array(_) => {
                let s = v.to_string();
                let stored = if s.chars().count() > 500 { None } else { Some(s) };
                out.push(FlattenedField {
                    key: trunc_key,
                    data_type: 5,
                    string_value: stored,
                    number_value: None,
                    date_value: None,
                });
            }
            Value::String(s) => {
                out.push(FlattenedField {
                    key: trunc_key,
                    data_type: 1,
                    string_value: Some(s.chars().take(500).collect()),
                    number_value: None,
                    date_value: None,
                });
            }
            Value::Null => {}
        }
    }
}

fn infer_ua_fields(ua: &str, screen: Option<&str>) -> (Option<String>, Option<String>, String) {
    let lower = ua.to_ascii_lowercase();
    let browser = if lower.contains("edg/") {
        Some("edge".to_string())
    } else if lower.contains("opr/") || lower.contains("opera") {
        Some("opera".to_string())
    } else if lower.contains("crios") {
        Some("crios".to_string())
    } else if lower.contains("chrome/") {
        Some("chrome".to_string())
    } else if lower.contains("fxios") {
        Some("fxios".to_string())
    } else if lower.contains("firefox/") {
        Some("firefox".to_string())
    } else if lower.contains("safari/") && lower.contains("version/") {
        Some("ios".to_string())
    } else {
        None
    };

    let os = if lower.contains("iphone") || lower.contains("ipad") || lower.contains("ipod") {
        Some("iOS".to_string())
    } else if lower.contains("android") {
        Some("Android OS".to_string())
    } else if lower.contains("mac os x") || lower.contains("macintosh") {
        Some("Mac OS".to_string())
    } else if lower.contains("windows") {
        Some("Windows 10".to_string())
    } else if lower.contains("linux") {
        Some("Linux".to_string())
    } else {
        None
    };

    let device = if lower.contains("ipad") || lower.contains("tablet") {
        "tablet".to_string()
    } else if lower.contains("mobi") || lower.contains("iphone") || lower.contains("android") {
        "mobile".to_string()
    } else if let Some(s) = screen {
        let w: u32 = s.split('x').next().unwrap_or("0").parse().unwrap_or(0);
        if w > 0 && w <= 1920 {
            "laptop".to_string()
        } else {
            "desktop".to_string()
        }
    } else {
        "desktop".to_string()
    };

    (browser, os, device)
}

fn trunc_opt(val: Option<&str>, max: usize) -> Option<String> {
    val.map(|s| s.chars().take(max).collect())
}

async fn persist_send_envelope(
    pool: &PgPool,
    envelope: &SendEnvelope,
    headers: &HeaderMap,
) -> Result<Value, (StatusCode, String)> {
    let payload = &envelope.payload;
    let website_str = payload.website.as_deref().unwrap_or("");
    let website_id = Uuid::parse_str(website_str)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid website UUID.".to_string()))?;

    let exists: Option<(Uuid,)> = sqlx::query_as(
        "SELECT website_id FROM website WHERE website_id = $1 AND deleted_at IS NULL",
    )
    .bind(website_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if exists.is_none() {
        return Err((StatusCode::BAD_REQUEST, "Website not found.".to_string()));
    }

    let ip = payload
        .ip
        .clone()
        .unwrap_or_else(|| extract_client_ip(headers));
    let user_agent = payload
        .user_agent
        .clone()
        .or_else(|| {
            headers
                .get(header::USER_AGENT)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        })
        .unwrap_or_default();

    let (country, region, city) = extract_location_headers(headers);
    let created_at = payload
        .timestamp
        .and_then(|ts| Utc.timestamp_opt(ts, 0).single())
        .unwrap_or_else(Utc::now);

    let distinct_id = trunc_opt(payload.id.as_deref(), 50);
    let (session_id, visit_id, iat) = resolve_session_and_visit(
        &website_id.to_string(),
        &ip,
        &user_agent,
        distinct_id.as_deref().unwrap_or(""),
        created_at,
    );

    let (ua_browser, ua_os, ua_device) = infer_ua_fields(&user_agent, payload.screen.as_deref());
    let browser = trunc_opt(payload.browser.as_deref().or(ua_browser.as_deref()), 20);
    let os = trunc_opt(payload.os.as_deref().or(ua_os.as_deref()), 20);
    let device = trunc_opt(payload.device.as_deref(), 20).or(Some(ua_device));
    let screen = trunc_opt(payload.screen.as_deref(), 11);
    let language = trunc_opt(payload.language.as_deref(), 35);
    let country_c = trunc_opt(country.as_deref(), 2);
    let region_c = trunc_opt(region.as_deref(), 20);
    let city_c = trunc_opt(city.as_deref(), 50);

    sqlx::query(
        r#"
        INSERT INTO session (
            session_id, website_id, browser, os, device, screen,
            language, country, region, city, distinct_id, created_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
        ON CONFLICT (session_id) DO NOTHING
        "#,
    )
    .bind(session_id)
    .bind(website_id)
    .bind(&browser)
    .bind(&os)
    .bind(&device)
    .bind(&screen)
    .bind(&language)
    .bind(&country_c)
    .bind(&region_c)
    .bind(&city_c)
    .bind(&distinct_id)
    .bind(created_at)
    .execute(pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if envelope.kind == "event" {
        let base_host = payload.hostname.as_deref().unwrap_or("localhost");
        let base_url = Url::parse(&format!("https://{base_host}"))
            .unwrap_or_else(|_| Url::parse("https://localhost").unwrap());
        let parsed_url = base_url
            .join(payload.url.as_deref().unwrap_or("/"))
            .unwrap_or(base_url.clone());

        let url_path = if parsed_url.path() == "/undefined" {
            String::new()
        } else if let Some(frag) = parsed_url.fragment() {
            format!("{}#{frag}", parsed_url.path())
        } else {
            parsed_url.path().to_string()
        };
        let url_query = parsed_url.query().map(|q| trunc_opt(Some(q), 500)).flatten();
        let hostname = trunc_opt(
            payload
                .hostname
                .as_deref()
                .or_else(|| parsed_url.host_str()),
            100,
        );
        let event_name = trunc_opt(payload.name.as_deref(), 50);
        let event_type: i32 = if event_name.is_some() { 2 } else { 1 };
        let tag = trunc_opt(payload.tag.as_deref(), 50);
        let page_title = trunc_opt(payload.title.as_deref(), 500);
        let event_id = Uuid::new_v4();

        sqlx::query(
            r#"
            INSERT INTO website_event (
                event_id, website_id, session_id, visit_id, created_at,
                url_path, url_query, page_title, event_type, event_name, tag, hostname
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            "#,
        )
        .bind(event_id)
        .bind(website_id)
        .bind(session_id)
        .bind(visit_id)
        .bind(created_at)
        .bind(trunc_opt(Some(&url_path), 500).unwrap_or_else(|| "/".to_string()))
        .bind(&url_query)
        .bind(&page_title)
        .bind(event_type)
        .bind(&event_name)
        .bind(&tag)
        .bind(&hostname)
        .execute(pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        if let Some(data_map) = &payload.data {
            let mut fields = Vec::new();
            flatten_event_data(data_map, "", &mut fields);
            for field in fields {
                sqlx::query(
                    r#"
                    INSERT INTO event_data (
                        event_data_id, website_id, website_event_id,
                        data_key, string_value, number_value, date_value, data_type, created_at
                    )
                    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                    "#,
                )
                .bind(Uuid::new_v4())
                .bind(website_id)
                .bind(event_id)
                .bind(&field.key)
                .bind(&field.string_value)
                .bind(field.number_value)
                .bind(field.date_value)
                .bind(field.data_type)
                .bind(created_at)
                .execute(pool)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            }
        }
    }

    let cache_claims = json!({
        "websiteId": website_id.to_string(),
        "sessionId": session_id.to_string(),
        "visitId": visit_id.to_string(),
        "iat": iat,
        "type": "cache"
    });
    let sec = secret_hash();
    let cache = encode(
        &Header::new(Algorithm::HS256),
        &cache_claims,
        &EncodingKey::from_secret(sec.as_bytes()),
    )
    .unwrap_or_default();

    Ok(json!({
        "cache": cache,
        "sessionId": session_id.to_string(),
        "visitId": visit_id.to_string()
    }))
}

fn with_rust_header(mut res: Response) -> Response {
    res.headers_mut()
        .insert("x-maap-server", HeaderValue::from_static("rust"));
    res
}

async fn heartbeat(State(state): State<AppState>) -> Response {
    if let Some(upstream) = &state.node_upstream {
        let url = format!("{upstream}/api/heartbeat");
        let Ok(res) = state.http.get(&url).send().await else {
            return with_rust_header(StatusCode::SERVICE_UNAVAILABLE.into_response());
        };
        if !res.status().is_success() {
            return with_rust_header(StatusCode::SERVICE_UNAVAILABLE.into_response());
        }
    }
    with_rust_header(Json(json!({ "ok": true })).into_response())
}

async fn handle_send(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(envelope): Json<SendEnvelope>,
) -> Response {
    let Some(pool) = &state.db else {
        return with_rust_header(
            (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "Database unavailable" })))
                .into_response(),
        );
    };
    match persist_send_envelope(pool, &envelope, &headers).await {
        Ok(body) => with_rust_header(Json(body).into_response()),
        Err((status, message)) => with_rust_header(
            (status, Json(json!({ "error": { "message": message } }))).into_response(),
        ),
    }
}

async fn handle_v2(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let Ok(Value::Array(batch)) = serde_json::from_slice::<Value>(&body) else {
        return with_rust_header(
            (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Expected a JSON array of signals." })),
            )
                .into_response(),
        );
    };
    let Some(pool) = &state.db else {
        return with_rust_header(
            (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "Database unavailable" })))
                .into_response(),
        );
    };

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let mut accepted = 0usize;
    let mut rejected = 0usize;
    let mut dropped = 0usize;

    for signal in &batch {
        match map_signal(signal, &state.td_options, now_ms) {
            SignalDisposition::Dropped => dropped += 1,
            SignalDisposition::Rejected => rejected += 1,
            SignalDisposition::Mapped(env) => {
                match persist_send_envelope(pool, &env, &headers).await {
                    Ok(_) => accepted += 1,
                    Err(_) => rejected += 1,
                }
            }
        }
    }

    let status = if accepted == 0 && rejected > 0 && dropped == 0 {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::OK
    };

    with_rust_header(
        (
            status,
            Json(json!({
                "accepted": accepted,
                "rejected": rejected,
                "dropped": dropped
            })),
        )
            .into_response(),
    )
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
        Regex::new(r"(?i)^https?://play\.google\.com/store/apps/details\?[^#]*\bid=([A-Za-z0-9._]+)")
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

async fn check_bearer_auth(pool: &PgPool, headers: &HeaderMap, path: &str) -> Option<(Uuid, String)> {
    let auth = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let token = auth.strip_prefix("Bearer ")?.trim();
    if token.starts_with("umami_") {
        if path.starts_with("/api/me/api-keys") || path.starts_with("/api/auth") {
            return None;
        }
        let key_hash = sha512_hex(&[token]);
        let row = sqlx::query(
            r#"
            SELECT u.user_id, u.role
            FROM api_key k
            JOIN "user" u ON u.user_id = k.user_id
            WHERE k.key_hash = $1 AND u.deleted_at IS NULL
            "#,
        )
        .bind(key_hash)
        .fetch_optional(pool)
        .await
        .ok()??;
        return Some((row.get("user_id"), row.get("role")));
    }

    let claims = parse_secure_token(token, &secret_hash())?;
    if claims.get("type").and_then(|v| v.as_str()) == Some("partial-auth") {
        return None;
    }
    let user_id_str = claims.get("userId")?.as_str()?;
    let user_id = Uuid::parse_str(user_id_str).ok()?;
    let row = sqlx::query(
        r#"SELECT user_id, role, password FROM "user" WHERE user_id = $1 AND deleted_at IS NULL"#,
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .ok()??;
    let password: String = row.get("password");
    if let Some(pwd_claim) = claims.get("pwd").and_then(|v| v.as_str()) {
        if sha512_hex(&[&password]) != pwd_claim {
            return None;
        }
    }
    Some((row.get("user_id"), row.get("role")))
}

async fn handle_login(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    let Some(pool) = &state.db else {
        return with_rust_header(StatusCode::SERVICE_UNAVAILABLE.into_response());
    };
    let username = body.get("username").and_then(|v| v.as_str()).unwrap_or("");
    let password = body.get("password").and_then(|v| v.as_str()).unwrap_or("");

    let Ok(Some(row)) = sqlx::query(
        r#"SELECT user_id, username, password, role, created_at FROM "user" WHERE username = $1 AND deleted_at IS NULL"#,
    )
    .bind(username)
    .fetch_optional(pool)
    .await
    else {
        return with_rust_header(
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": { "code": "incorrect-username-password" } })),
            )
                .into_response(),
        );
    };

    let stored_hash: String = row.get("password");
    if !bcrypt::verify(password, &stored_hash).unwrap_or(false) {
        return with_rust_header(
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": { "code": "incorrect-username-password" } })),
            )
                .into_response(),
        );
    }

    let user_id: Uuid = row.get("user_id");
    let role: String = row.get("role");
    let created_at: Option<DateTime<Utc>> = row.get("created_at");
    let pwd = sha512_hex(&[&stored_hash]);
    let session_claims = json!({
        "userId": user_id.to_string(),
        "role": role,
        "pwd": pwd
    });
    let Some(token) = create_secure_token(&session_claims, &secret_hash()) else {
        return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response());
    };

    with_rust_header(
        Json(json!({
            "token": token,
            "user": {
                "id": user_id,
                "username": username,
                "role": role,
                "createdAt": created_at,
                "isAdmin": role == "admin",
                "passwordChangeRequired": username == "admin" && bcrypt::verify("umami", &stored_hash).unwrap_or(false),
                "teams": []
            }
        }))
        .into_response(),
    )
}

async fn handle_create_website(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let Some(pool) = &state.db else {
        return with_rust_header(StatusCode::SERVICE_UNAVAILABLE.into_response());
    };
    let Some((user_id, _)) = check_bearer_auth(pool, &headers, "/api/websites").await else {
        return with_rust_header(StatusCode::UNAUTHORIZED.into_response());
    };
    let id = body
        .get("id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .unwrap_or_else(Uuid::new_v4);
    let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("").trim();
    let domain = body.get("domain").and_then(|v| v.as_str()).map(str::trim);
    if name.is_empty() {
        return with_rust_header(StatusCode::BAD_REQUEST.into_response());
    }
    let now = Utc::now();
    if let Err(e) = sqlx::query(
        r#"
        INSERT INTO website (website_id, name, domain, user_id, created_by, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $4, $5, $5)
        "#,
    )
    .bind(id)
    .bind(name)
    .bind(domain)
    .bind(user_id)
    .bind(now)
    .execute(pool)
    .await
    {
        return with_rust_header(
            (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
        );
    }

    with_rust_header(
        Json(json!({
            "id": id,
            "name": name,
            "domain": domain,
            "userId": user_id,
            "createdBy": user_id,
            "createdAt": now,
            "updatedAt": now,
            "shareId": null
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

async fn handle_website_metrics(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let Some(pool) = &state.db else {
        return with_rust_header(StatusCode::SERVICE_UNAVAILABLE.into_response());
    };
    if check_bearer_auth(pool, &headers, "/api/websites").await.is_none() {
        return with_rust_header(StatusCode::UNAUTHORIZED.into_response());
    }
    let metric_type = params.get("type").map(String::as_str).unwrap_or("");
    let Some(start_date) = parse_ms_param(&params, "startAt") else {
        return with_rust_header(StatusCode::BAD_REQUEST.into_response());
    };
    let Some(end_date) = parse_ms_param(&params, "endAt") else {
        return with_rust_header(StatusCode::BAD_REQUEST.into_response());
    };
    let test_only = params.get("testMode").map(String::as_str) == Some("true")
        || params.get("tag").map(String::as_str) == Some("test");

    let sql = match metric_type {
        "path" | "url" => {
            if test_only {
                "SELECT url_path AS x, COUNT(DISTINCT session_id)::bigint AS y FROM website_event WHERE website_id = $1 AND created_at BETWEEN $2 AND $3 AND event_type NOT IN (2, 5) AND tag = 'test' AND url_path != '' GROUP BY 1 ORDER BY 2 DESC"
            } else {
                "SELECT url_path AS x, COUNT(DISTINCT session_id)::bigint AS y FROM website_event WHERE website_id = $1 AND created_at BETWEEN $2 AND $3 AND event_type NOT IN (2, 5) AND COALESCE(tag, '') != 'test' AND url_path != '' GROUP BY 1 ORDER BY 2 DESC"
            }
        }
        "event" => {
            if test_only {
                "SELECT event_name AS x, COUNT(*)::bigint AS y FROM website_event WHERE website_id = $1 AND created_at BETWEEN $2 AND $3 AND event_type = 2 AND tag = 'test' AND event_name IS NOT NULL GROUP BY 1 ORDER BY 2 DESC"
            } else {
                "SELECT event_name AS x, COUNT(*)::bigint AS y FROM website_event WHERE website_id = $1 AND created_at BETWEEN $2 AND $3 AND event_type = 2 AND COALESCE(tag, '') != 'test' AND event_name IS NOT NULL GROUP BY 1 ORDER BY 2 DESC"
            }
        }
        "browser" | "os" | "device" | "screen" | "country" | "region" | "city" | "language" => {
            let col = if metric_type == "language" {
                "lower(left(s.language, 2))"
            } else {
                match metric_type {
                    "browser" => "s.browser",
                    "os" => "s.os",
                    "device" => "s.device",
                    "screen" => "s.screen",
                    "country" => "s.country",
                    "region" => "s.region",
                    "city" => "s.city",
                    _ => unreachable!(),
                }
            };
            let tag_cond = if test_only {
                "e.tag = 'test'"
            } else {
                "COALESCE(e.tag, '') != 'test'"
            };
            let query_str = format!(
                "SELECT {col} AS x, COUNT(DISTINCT e.session_id)::bigint AS y FROM website_event e JOIN session s ON s.session_id = e.session_id AND s.website_id = e.website_id WHERE e.website_id = $1 AND e.created_at BETWEEN $2 AND $3 AND e.event_type NOT IN (2, 5) AND {tag_cond} AND COALESCE({col}, '') != '' GROUP BY 1 ORDER BY 2 DESC"
            );
            let Ok(rows) = sqlx::query(&query_str)
                .bind(website_id)
                .bind(start_date)
                .bind(end_date)
                .fetch_all(pool)
                .await
            else {
                return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response());
            };
            let out: Vec<Value> = rows
                .into_iter()
                .map(|r| json!({ "x": r.get::<String, _>("x"), "y": r.get::<i64, _>("y") }))
                .collect();
            return with_rust_header(Json(out).into_response());
        }
        _ => return with_rust_header(StatusCode::BAD_REQUEST.into_response()),
    };

    let Ok(rows) = sqlx::query(sql)
        .bind(website_id)
        .bind(start_date)
        .bind(end_date)
        .fetch_all(pool)
        .await
    else {
        return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response());
    };
    let out: Vec<Value> = rows
        .into_iter()
        .map(|r| json!({ "x": r.get::<String, _>("x"), "y": r.get::<i64, _>("y") }))
        .collect();
    with_rust_header(Json(out).into_response())
}

async fn query_stats_window(
    pool: &PgPool,
    website_id: Uuid,
    start_date: DateTime<Utc>,
    end_date: DateTime<Utc>,
    browser: Option<&str>,
    test_only: bool,
) -> Value {
    let tag_cond = if test_only {
        "e.tag = 'test'"
    } else {
        "COALESCE(e.tag, '') != 'test'"
    };
    let browser_clean = browser.map(|b| b.strip_prefix("eq.").unwrap_or(b));
    let sql = format!(
        r#"
        SELECT
            COALESCE(SUM(t.c), 0)::bigint AS pageviews,
            COUNT(DISTINCT t.session_id)::bigint AS visitors,
            COUNT(DISTINCT t.visit_id)::bigint AS visits,
            COALESCE(SUM(CASE WHEN t.c = 1 AND t.has_custom = 0 THEN 1 ELSE 0 END), 0)::bigint AS bounces,
            COALESCE(SUM(EXTRACT(EPOCH FROM (t.max_time - t.min_time)))::bigint, 0) AS totaltime
        FROM (
            SELECT
                e.session_id,
                e.visit_id,
                SUM(CASE WHEN e.event_type NOT IN (2, 5) THEN 1 ELSE 0 END) AS c,
                MIN(CASE WHEN e.event_type NOT IN (2, 5) THEN e.created_at END) AS min_time,
                MAX(CASE WHEN e.event_type NOT IN (2, 5) THEN e.created_at END) AS max_time,
                MAX(CASE WHEN e.event_type = 2 THEN 1 ELSE 0 END) AS has_custom
            FROM website_event e
            JOIN session s ON s.session_id = e.session_id AND s.website_id = e.website_id
            WHERE e.website_id = $1
              AND e.created_at BETWEEN $2 AND $3
              AND e.event_type != 5
              AND {tag_cond}
              AND ($4::text IS NULL OR s.browser = $4)
            GROUP BY 1, 2
            HAVING SUM(CASE WHEN e.event_type NOT IN (2, 5) THEN 1 ELSE 0 END) > 0
        ) t
        "#
    );

    if let Ok(row) = sqlx::query(&sql)
        .bind(website_id)
        .bind(start_date)
        .bind(end_date)
        .bind(browser_clean)
        .fetch_one(pool)
        .await
    {
        json!({
            "pageviews": row.get::<i64, _>("pageviews"),
            "visitors": row.get::<i64, _>("visitors"),
            "visits": row.get::<i64, _>("visits"),
            "bounces": row.get::<i64, _>("bounces"),
            "totaltime": row.get::<i64, _>("totaltime")
        })
    } else {
        json!({ "pageviews": 0, "visitors": 0, "visits": 0, "bounces": 0, "totaltime": 0 })
    }
}

async fn handle_website_stats(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let Some(pool) = &state.db else {
        return with_rust_header(StatusCode::SERVICE_UNAVAILABLE.into_response());
    };
    if check_bearer_auth(pool, &headers, "/api/websites").await.is_none() {
        return with_rust_header(StatusCode::UNAUTHORIZED.into_response());
    }
    let Some(start_date) = parse_ms_param(&params, "startAt") else {
        return with_rust_header(StatusCode::BAD_REQUEST.into_response());
    };
    let Some(end_date) = parse_ms_param(&params, "endAt") else {
        return with_rust_header(StatusCode::BAD_REQUEST.into_response());
    };
    let test_only = params.get("testMode").map(String::as_str) == Some("true")
        || params.get("tag").map(String::as_str) == Some("test");
    let browser = params.get("browser").map(String::as_str);

    let current = query_stats_window(pool, website_id, start_date, end_date, browser, test_only).await;
    let diff = end_date - start_date;
    let prev_end = start_date;
    let prev_start = prev_end - diff;
    let comparison = query_stats_window(pool, website_id, prev_start, prev_end, browser, test_only).await;

    let mut out = current.as_object().cloned().unwrap_or_default();
    out.insert("comparison".to_string(), comparison);
    with_rust_header(Json(Value::Object(out)).into_response())
}

async fn handle_create_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let Some(pool) = &state.db else {
        return with_rust_header(StatusCode::SERVICE_UNAVAILABLE.into_response());
    };
    let Some((user_id, _)) = check_bearer_auth(pool, &headers, "/api/websites").await else {
        return with_rust_header(StatusCode::UNAUTHORIZED.into_response());
    };
    let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("").trim();
    if name.is_empty() {
        return with_rust_header(StatusCode::BAD_REQUEST.into_response());
    }
    let random_part: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect();
    let key = format!("umami_{random_part}");
    let key_hash = sha512_hex(&[&key]);
    let key_prefix: String = key.chars().take(14).collect();
    let id = Uuid::new_v4();
    let now = Utc::now();

    if sqlx::query(
        r#"
        INSERT INTO api_key (api_key_id, user_id, name, key_hash, key_prefix, created_at)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(id)
    .bind(user_id)
    .bind(name)
    .bind(&key_hash)
    .bind(&key_prefix)
    .bind(now)
    .execute(pool)
    .await
    .is_err()
    {
        return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }

    with_rust_header(
        Json(json!({
            "id": id,
            "userId": user_id,
            "name": name,
            "keyPrefix": key_prefix,
            "createdAt": now,
            "key": key
        }))
        .into_response(),
    )
}

async fn handle_mcp(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if env::var("MCP_ENABLED").ok().as_deref() != Some("1") {
        return with_rust_header(StatusCode::NOT_FOUND.into_response());
    }
    let Some(pool) = &state.db else {
        return with_rust_header(StatusCode::SERVICE_UNAVAILABLE.into_response());
    };
    if check_bearer_auth(pool, &headers, "/mcp").await.is_none() {
        return with_rust_header(StatusCode::UNAUTHORIZED.into_response());
    };

    let id = body.get("id").cloned().unwrap_or(Value::Null);
    let method = body.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let params = body.get("params").and_then(|v| v.as_object());

    if method == "tools/call" {
        if let Some(p) = params {
            if p.get("name").and_then(|v| v.as_str()) == Some("get_website_stats") {
                if let Some(args) = p.get("arguments").and_then(|v| v.as_object()) {
                    let website_id = args
                        .get("websiteId")
                        .and_then(|v| v.as_str())
                        .and_then(|s| Uuid::parse_str(s).ok());
                    let start_at = args
                        .get("startAt")
                        .and_then(|v| v.as_str())
                        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                        .map(|d| d.with_timezone(&Utc));
                    let end_at = args
                        .get("endAt")
                        .and_then(|v| v.as_str())
                        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                        .map(|d| d.with_timezone(&Utc));
                    if let (Some(wid), Some(start), Some(end)) = (website_id, start_at, end_at) {
                        let current = query_stats_window(pool, wid, start, end, None, false).await;
                        let diff = end - start;
                        let comparison =
                            query_stats_window(pool, wid, start - diff, start, None, false).await;
                        let structured = json!({
                            "current": current,
                            "comparison": comparison
                        });
                        return with_rust_header(
                            Json(json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": {
                                    "content": [{ "type": "text", "text": structured.to_string() }],
                                    "structuredContent": structured
                                }
                            }))
                            .into_response(),
                        );
                    }
                }
            }
        }
    }

    if let Some(upstream) = &state.node_upstream {
        return proxy_json_to_node(&state.http, upstream, "/mcp", &headers, &body).await;
    }

    with_rust_header(StatusCode::BAD_REQUEST.into_response())
}

async fn proxy_json_to_node(
    client: &reqwest::Client,
    upstream: &str,
    path: &str,
    headers: &HeaderMap,
    body: &Value,
) -> Response {
    let url = format!("{upstream}{path}");
    let mut rb = client.post(&url);
    for (k, v) in headers {
        if k != header::HOST && k != header::CONTENT_LENGTH {
            rb = rb.header(k.as_str(), v.as_bytes());
        }
    }
    match rb.json(body).send().await {
        Ok(resp) => convert_reqwest_response(resp).await,
        Err(_) => StatusCode::BAD_GATEWAY.into_response(),
    }
}

async fn convert_reqwest_response(resp: reqwest::Response) -> Response {
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::OK);
    let headers = resp.headers().clone();
    let bytes = resp.bytes().await.unwrap_or_default();
    let mut out = Response::new(Body::from(bytes));
    *out.status_mut() = status;
    for (k, v) in headers {
        if let Some(name) = k {
            if let (Ok(hn), Ok(hv)) = (
                HeaderName::from_bytes(name.as_str().as_bytes()),
                HeaderValue::from_bytes(v.as_bytes()),
            ) {
                out.headers_mut().insert(hn, hv);
            }
        }
    }
    out
}

async fn fallback_proxy(State(state): State<AppState>, req: Request) -> Response {
    let Some(upstream) = &state.node_upstream else {
        return with_rust_header(StatusCode::NOT_FOUND.into_response());
    };
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");
    let url = format!("{upstream}{path_and_query}");
    let method = reqwest::Method::from_bytes(req.method().as_str().as_bytes())
        .unwrap_or(reqwest::Method::GET);
    let headers = req.headers().clone();
    let body_bytes = to_bytes(req.into_body(), 16 * 1024 * 1024)
        .await
        .unwrap_or_default();

    let mut rb = state.http.request(method, &url);
    for (k, v) in &headers {
        if k != header::HOST && k != header::CONTENT_LENGTH {
            rb = rb.header(k.as_str(), v.as_bytes());
        }
    }
    if !body_bytes.is_empty() {
        rb = rb.body(body_bytes.to_vec());
    }

    match rb.send().await {
        Ok(resp) => convert_reqwest_response(resp).await,
        Err(_) => StatusCode::BAD_GATEWAY.into_response(),
    }
}

pub fn build_router(state: AppState) -> Router {
    let router = Router::new()
        .route("/api/heartbeat", get(heartbeat))
        .route("/api/send", post(handle_send))
        .route("/v2", post(handle_v2))
        .route("/v2/", post(handle_v2))
        .route("/v2/namespace/{*namespace}", post(handle_v2))
        .route("/app-icon", get(handle_app_icon));

    if state.node_upstream.is_none() {
        router
            .route("/api/auth/login", post(handle_login))
            .route("/api/websites", post(handle_create_website))
            .route("/api/websites/{website_id}/metrics", get(handle_website_metrics))
            .route("/api/websites/{website_id}/stats", get(handle_website_stats))
            .route("/api/me/api-keys", post(handle_create_api_key))
            .route("/mcp", post(handle_mcp))
            .with_state(state)
    } else {
        router.fallback(any(fallback_proxy)).with_state(state)
    }
}

#[tokio::main]
async fn main() {
    let db_url = env::var("DATABASE_URL").unwrap_or_default();
    let db = if !db_url.is_empty() {
        PgPoolOptions::new()
            .max_connections(10)
            .connect(&db_url)
            .await
            .ok()
    } else {
        None
    };

    let allowed_keys: HashSet<String> = env::var("TD_ALLOWED_KEYS")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();

    let td_options = TdOptions {
        hostname: env::var("TD_HOSTNAME").ok().filter(|s| !s.is_empty()),
        test_website_id: env::var("TD_TEST_WEBSITE_ID").ok().filter(|s| !s.is_empty()),
        allowed_keys,
    };

    let node_upstream = if env::var("MAAP_NODE_FALLBACK").ok().as_deref() == Some("0") {
        None
    } else {
        let port = env::var("MAAP_NODE_PORT").unwrap_or_else(|_| "3001".to_string());
        Some(format!("http://127.0.0.1:{port}"))
    };

    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(5))
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("Mozilla/5.0 (compatible; MaapIconBot/1.0)")
        .build()
        .expect("http client");

    let state = AppState {
        db,
        http,
        td_options,
        node_upstream,
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
    axum::serve(listener, build_router(state))
        .await
        .expect("serve");
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(mapped_android.payload.hostname.as_deref(), Some("app.android"));
        assert_eq!(mapped_android.payload.device.as_deref(), Some("mobile"));
        assert_eq!(mapped_android.payload.os.as_deref(), Some("Android 15"));
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
}
