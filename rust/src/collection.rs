use super::*;
use sha2::Sha256;
use sqlx::{Postgres, Transaction};
use std::net::IpAddr;

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
    pub link: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pixel: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub salt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lcp: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inp: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cls: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fcp: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttfb: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Map<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<Vec<String>>,
}

pub fn get_salt(created_at: DateTime<Utc>) -> String {
    if let Ok(rot) = env::var("SALT_ROTATION") {
        let trimmed = rot.trim();
        if !trimmed.is_empty() {
            let period = match trimmed.to_ascii_lowercase().as_str() {
                "day" | "daily" => created_at
                    .date_naive()
                    .and_hms_opt(0, 0, 0)
                    .map(|dt| dt.and_utc()),
                "week" | "weekly" => {
                    let start = created_at
                        - chrono::Duration::days(created_at.weekday().num_days_from_sunday() as i64);
                    start
                        .date_naive()
                        .and_hms_opt(0, 0, 0)
                        .map(|dt| dt.and_utc())
                }
                "month" | "monthly" => Some(start_of_month_utc(created_at)),
                _ => return sha512_hex(&[trimmed]),
            };
            if let Some(period) = period {
                return sha512_hex(&[&to_js_utc_string(period)]);
            }
        }
    }
    sha512_hex(&[&to_js_utc_string(start_of_month_utc(created_at))])
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

    let test_website = options.test_website_id.as_deref().filter(|s| !s.is_empty());

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

    let major_minor = version
        .and_then(|v| ver_suffix_re.find(v))
        .map(|m| m.as_str());
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
            link: None,
            pixel: None,
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
            name: if nav {
                None
            } else {
                Some(sig_type_raw.to_string())
            },
            tag: if is_test {
                Some("test".to_string())
            } else {
                None
            },
            data: Some(data),
            ip: None,
            salt: None,
            lcp: None,
            inp: None,
            cls: None,
            fcp: None,
            ttfb: None,
            version: None,
            event_id: None,
            message: None,
            stack: None,
            handled: None,
            release: None,
            environment: None,
            tags: None,
            fingerprint: None,
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

pub fn extract_location_headers(
    headers: &HeaderMap,
) -> (Option<String>, Option<String>, Option<String>) {
    if env::var("SKIP_LOCATION_HEADERS").is_ok() {
        return (None, None, None);
    }
    let providers = [
        ("cf-ipcountry", "cf-region-code", "cf-ipcity"),
        (
            "x-vercel-ip-country",
            "x-vercel-ip-country-region",
            "x-vercel-ip-city",
        ),
        (
            "cloudfront-viewer-country",
            "cloudfront-viewer-country-region",
            "cloudfront-viewer-city",
        ),
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
                .and_then(|v| std::str::from_utf8(v.as_bytes()).ok())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|city| crate::decode_uri(city, false));
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
                let stored = if s.chars().count() > 500 {
                    None
                } else {
                    Some(s)
                };
                out.push(FlattenedField {
                    key: trunc_key,
                    data_type: 5,
                    string_value: stored,
                    number_value: None,
                    date_value: None,
                });
            }
            Value::String(s) => {
                let date = DateTime::parse_from_rfc3339(s)
                    .ok()
                    .map(|value| value.with_timezone(&Utc));
                out.push(FlattenedField {
                    key: trunc_key,
                    data_type: if date.is_some() { 4 } else { 1 },
                    string_value: Some(
                        date.as_ref()
                            .map(|value| value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
                            .unwrap_or_else(|| s.chars().take(500).collect()),
                    ),
                    number_value: None,
                    date_value: date,
                });
            }
            Value::Null => {
                out.push(FlattenedField {
                    key: trunc_key,
                    data_type: 5,
                    string_value: Some("null".to_string()),
                    number_value: None,
                    date_value: None,
                });
            }
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

type CollectionError = (StatusCode, String);

#[derive(Debug, Clone)]
struct CacheClaims {
    session_id: Uuid,
    visit_id: Uuid,
    iat: i64,
    session_link_id: Option<String>,
}

#[derive(Debug, Clone)]
struct ResolvedSession {
    session_id: Uuid,
    visit_id: Uuid,
    iat: i64,
    session_drift: bool,
}

fn bad_request(message: impl Into<String>) -> CollectionError {
    (StatusCode::BAD_REQUEST, message.into())
}

fn collection_error_body(status: StatusCode, message: impl Into<String>) -> Value {
    let code = match status {
        StatusCode::BAD_REQUEST => "bad-request",
        StatusCode::FORBIDDEN => "forbidden",
        StatusCode::PAYLOAD_TOO_LARGE => "payload-too-large",
        StatusCode::TOO_MANY_REQUESTS => "rate-limited",
        StatusCode::SERVICE_UNAVAILABLE => "service-unavailable",
        _ => "server-error",
    };
    json!({
        "error": {
            "message": message.into(),
            "code": code,
            "status": status.as_u16()
        }
    })
}

fn validate_data_value(value: &Value, depth: usize, in_array: bool) -> Result<(), CollectionError> {
    if depth > 32 {
        return Err(bad_request("Data must not be nested more than 32 levels."));
    }
    match value {
        Value::Number(number) if !in_array => {
            let value = number
                .as_f64()
                .filter(|value| value.is_finite())
                .ok_or_else(|| bad_request("Number is out of range."))?;
            if value.abs() >= 1e15 {
                return Err(bad_request("Number is out of range."));
            }
        }
        Value::Object(map) => {
            for value in map.values() {
                validate_data_value(value, depth + 1, in_array)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                validate_data_value(value, depth + 1, true)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_envelope(envelope: &SendEnvelope) -> Result<(), CollectionError> {
    let payload = &envelope.payload;
    if envelope.kind == "error" {
        return validate_error_payload(payload);
    }
    if !matches!(envelope.kind.as_str(), "event" | "identify" | "performance") {
        return Err(bad_request("Invalid collection type."));
    }
    if [
        payload.website.as_ref(),
        payload.link.as_ref(),
        payload.pixel.as_ref(),
    ]
    .into_iter()
    .filter(|value| value.is_some())
    .count()
        != 1
    {
        return Err(bad_request(
            "Exactly one of website, link, or pixel must be provided.",
        ));
    }
    for value in [&payload.website, &payload.link, &payload.pixel]
        .into_iter()
        .flatten()
    {
        Uuid::parse_str(value).map_err(|_| bad_request("Invalid source UUID."))?;
    }
    if payload
        .timestamp
        .is_some_and(|timestamp| !(0..=2_147_483_647).contains(&timestamp))
    {
        return Err(bad_request("Invalid timestamp."));
    }
    if payload
        .salt
        .as_ref()
        .is_some_and(|salt| salt.chars().count() > 100)
    {
        return Err(bad_request("Salt is too long."));
    }
    for value in [&payload.name, &payload.tag].into_iter().flatten() {
        if value
            .chars()
            .next()
            .is_some_and(|first| matches!(first, '=' | '+' | '-' | '@' | '\t' | '\r'))
        {
            return Err(bad_request("Unsafe event field."));
        }
    }
    if let Some(hostname) = payload.hostname.as_deref() {
        Url::parse(&format!("https://{hostname}")).map_err(|_| bad_request("Invalid hostname."))?;
    }
    for value in [&payload.url, &payload.referrer].into_iter().flatten() {
        Url::parse(value)
            .or_else(|_| Url::parse("https://localhost").and_then(|base| base.join(value)))
            .map_err(|_| bad_request("Invalid URL."))?;
    }
    if let Some(data) = &payload.data {
        validate_data_value(&Value::Object(data.clone()), 1, false)?;
        if envelope.kind == "event" {
            let revenue = data.get("revenue").and_then(|value| match value {
                Value::Number(number) => number.as_f64(),
                Value::String(value) => value.parse().ok(),
                _ => None,
            });
            let has_currency = data
                .get("currency")
                .and_then(Value::as_str)
                .is_some_and(|value| !value.trim().is_empty());
            if has_currency && revenue.is_some_and(|value| value.is_finite() && value >= 1e14) {
                return Err(bad_request("Revenue is out of range."));
            }
        }
    }
    let metric_valid = |value: Option<f64>, max: f64| {
        value.is_none_or(|value| value.is_finite() && value >= 0.0 && value <= max)
    };
    if !metric_valid(payload.lcp, 60_000.0)
        || !metric_valid(payload.inp, 60_000.0)
        || !metric_valid(payload.cls, 100.0)
        || !metric_valid(payload.fcp, 60_000.0)
        || !metric_valid(payload.ttfb, 60_000.0)
    {
        return Err(bad_request("Invalid performance metric."));
    }
    Ok(())
}

fn validate_error_payload(payload: &SendPayload) -> Result<(), CollectionError> {
    if payload.version != Some(1) {
        return Err(bad_request("Invalid error version."));
    }
    let website = payload
        .website
        .as_deref()
        .ok_or_else(|| bad_request("Missing website."))?;
    Uuid::parse_str(website).map_err(|_| bad_request("Invalid website UUID."))?;
    let event_id = payload
        .event_id
        .as_deref()
        .ok_or_else(|| bad_request("Missing eventId."))?;
    Uuid::parse_str(event_id).map_err(|_| bad_request("Invalid eventId."))?;
    let timestamp = payload
        .timestamp
        .ok_or_else(|| bad_request("Missing timestamp."))?;
    if timestamp <= 0 {
        return Err(bad_request("Invalid timestamp."));
    }
    let name = payload.name.as_deref().unwrap_or("");
    if name.is_empty() || name.chars().count() > 200 {
        return Err(bad_request("Invalid error name."));
    }
    if payload
        .message
        .as_ref()
        .is_none_or(|value| value.chars().count() > 2000)
        || payload
            .stack
            .as_ref()
            .is_some_and(|value| value.chars().count() > 16_000)
        || payload
            .url
            .as_ref()
            .is_none_or(|value| value.chars().count() > 2000)
        || payload
            .release
            .as_ref()
            .is_some_and(|value| value.chars().count() > 100)
        || payload
            .environment
            .as_ref()
            .is_some_and(|value| value.chars().count() > 50)
        || payload
            .salt
            .as_ref()
            .is_some_and(|value| value.chars().count() > 100)
        || payload
            .language
            .as_ref()
            .is_some_and(|value| value.chars().count() > 35)
        || payload
            .screen
            .as_ref()
            .is_some_and(|value| value.chars().count() > 11)
    {
        return Err(bad_request("Invalid error payload."));
    }
    if payload.tags.as_ref().is_some_and(|tags| {
        tags.len() > 20
            || tags.iter().any(|(key, value)| {
                key.chars().count() > 50 || value.as_str().is_none_or(|v| v.chars().count() > 200)
            })
    }) {
        return Err(bad_request("Invalid error tags."));
    }
    if payload.fingerprint.as_ref().is_some_and(|values| {
        values.is_empty()
            || values.len() > 5
            || values
                .iter()
                .any(|value| value.is_empty() || value.chars().count() > 200)
    }) {
        return Err(bad_request("Invalid error fingerprint."));
    }
    Ok(())
}

fn parse_cache(headers: &HeaderMap, website_id: Uuid) -> Option<CacheClaims> {
    let token = headers.get("x-umami-cache")?.to_str().ok()?;
    let mut validation = Validation::new(Algorithm::HS256);
    validation.required_spec_claims.clear();
    let data = decode::<Value>(
        token,
        &DecodingKey::from_secret(secret_hash().as_bytes()),
        &validation,
    )
    .ok()?;
    let claims = data.claims.as_object()?;
    if claims.get("type")?.as_str()? != "cache"
        || Uuid::parse_str(claims.get("websiteId")?.as_str()?).ok()? != website_id
    {
        return None;
    }
    Some(CacheClaims {
        session_id: Uuid::parse_str(claims.get("sessionId")?.as_str()?).ok()?,
        visit_id: Uuid::parse_str(claims.get("visitId")?.as_str()?).ok()?,
        iat: claims.get("iat")?.as_i64()?,
        session_link_id: claims
            .get("sessionLinkId")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

fn session_id_for(
    source_id: Uuid,
    ip: &str,
    user_agent: &str,
    distinct_id: Option<&str>,
    salt: Option<&str>,
    created_at: DateTime<Utc>,
) -> Uuid {
    let base_salt = get_salt(created_at);
    let session_salt = if env::var("ENABLE_CLIENT_SALT").is_ok() {
        salt.filter(|value| !value.is_empty())
            .map(|value| sha512_hex(&[&base_salt, value]))
            .unwrap_or(base_salt)
    } else {
        base_salt
    };
    deterministic_uuid(&[
        &source_id.to_string(),
        ip,
        user_agent,
        &session_salt,
        distinct_id.unwrap_or(""),
    ])
}

fn resolve_cached_session(
    source_id: Uuid,
    ip: &str,
    user_agent: &str,
    distinct_id: Option<&str>,
    salt: Option<&str>,
    created_at: DateTime<Utc>,
    historical: bool,
    cache: Option<&CacheClaims>,
) -> ResolvedSession {
    let now = Utc::now().timestamp();
    let session_id = session_id_for(source_id, ip, user_agent, distinct_id, salt, created_at);
    let session_drift = cache.is_some_and(|cache| cache.session_id != session_id);
    let mut visit_id = cache.map(|cache| cache.visit_id).unwrap_or_else(|| {
        let visit_salt = sha512_hex(&[&to_js_utc_string(start_of_hour_utc(created_at))]);
        deterministic_uuid(&[&session_id.to_string(), &visit_salt])
    });
    let mut iat = cache.map(|cache| cache.iat).unwrap_or(now);
    if session_drift || (!historical && now - iat > 1800) {
        let visit_salt = sha512_hex(&[&to_js_utc_string(start_of_hour_utc(created_at))]);
        visit_id = deterministic_uuid(&[&session_id.to_string(), &visit_salt]);
        iat = now;
    }
    ResolvedSession {
        session_id,
        visit_id,
        iat,
        session_drift,
    }
}

fn ip_matches_cidr(ip: IpAddr, cidr: &str) -> bool {
    let Some((network, prefix)) = cidr.split_once('/') else {
        return false;
    };
    let Ok(network) = network.parse::<IpAddr>() else {
        return false;
    };
    let Ok(prefix) = prefix.parse::<u32>() else {
        return false;
    };
    match (ip, network) {
        (IpAddr::V4(ip), IpAddr::V4(network)) if prefix <= 32 => {
            let mask = if prefix == 0 {
                0
            } else {
                u32::MAX << (32 - prefix)
            };
            u32::from(ip) & mask == u32::from(network) & mask
        }
        (IpAddr::V6(ip), IpAddr::V6(network)) if prefix <= 128 => {
            let ip = u128::from_be_bytes(ip.octets());
            let network = u128::from_be_bytes(network.octets());
            let mask = if prefix == 0 {
                0
            } else {
                u128::MAX << (128 - prefix)
            };
            ip & mask == network & mask
        }
        _ => false,
    }
}

pub(super) fn is_blocked_ip(ip: &str) -> bool {
    let Ok(client_ip) = ip.parse::<IpAddr>() else {
        return false;
    };
    env::var("IGNORE_IP")
        .ok()
        .into_iter()
        .flat_map(|value| {
            value
                .split(',')
                .map(str::trim)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .any(|value| value == ip || ip_matches_cidr(client_ip, &value))
}

pub(super) fn is_bot(user_agent: &str) -> bool {
    if env::var("DISABLE_BOT_CHECK").is_ok() {
        return false;
    }
    let ua = user_agent.to_ascii_lowercase();
    [
        "bot",
        "crawler",
        "spider",
        "headlesschrome",
        "lighthouse",
        "slurp",
        "facebookexternalhit",
        "preview",
    ]
    .iter()
    .any(|needle| ua.contains(needle))
}

async fn ensure_session(
    tx: &mut Transaction<'_, Postgres>,
    source_id: Uuid,
    session: &ResolvedSession,
    browser: &Option<String>,
    os: &Option<String>,
    device: &Option<String>,
    screen: &Option<String>,
    language: &Option<String>,
    country: &Option<String>,
    region: &Option<String>,
    city: &Option<String>,
    distinct_id: &Option<String>,
    created_at: DateTime<Utc>,
    cache: Option<&CacheClaims>,
) -> Result<(), sqlx::Error> {
    if cache.is_some() && !session.session_drift {
        return Ok(());
    }
    sqlx::query(
        r#"
        INSERT INTO session (
            session_id, website_id, browser, os, device, screen,
            language, country, region, city, distinct_id, created_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
        ON CONFLICT (session_id) DO UPDATE SET
            country = COALESCE(session.country, excluded.country),
            region = COALESCE(session.region, excluded.region),
            city = COALESCE(session.city, excluded.city)
        WHERE session.website_id = excluded.website_id AND excluded.country IS NOT NULL
          AND (session.country IS NULL OR session.country = excluded.country)
          AND (session.region IS NULL OR excluded.region IS NULL OR session.region = excluded.region)
          AND (session.city IS NULL OR excluded.city IS NULL OR session.city = excluded.city)
          AND (session.country IS NULL OR (session.region IS NULL AND excluded.region IS NOT NULL)
               OR (session.city IS NULL AND excluded.city IS NOT NULL))
        "#,
    )
    .bind(session.session_id)
    .bind(source_id)
    .bind(browser)
    .bind(os)
    .bind(device)
    .bind(screen)
    .bind(language)
    .bind(country)
    .bind(region)
    .bind(city)
    .bind(distinct_id)
    .bind(created_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn try_claim_session(
    tx: &mut Transaction<'_, Postgres>,
    website_id: Uuid,
    session_id: Uuid,
    distinct_id: &str,
    created_at: DateTime<Utc>,
) -> Result<bool, sqlx::Error> {
    sqlx::query(
        "SELECT pg_advisory_xact_lock(('x' || substr(replace($1, '-', ''), 1, 16))::bit(64)::bigint)",
    )
    .bind(session_id.to_string())
    .execute(&mut **tx)
    .await?;
    let linked: Vec<(String,)> = sqlx::query_as(
        "SELECT distinct_id FROM session_link WHERE website_id = $1 AND session_id = $2",
    )
    .bind(website_id)
    .bind(session_id)
    .fetch_all(&mut **tx)
    .await?;
    let session_distinct: Option<(Option<String>,)> =
        sqlx::query_as("SELECT distinct_id FROM session WHERE website_id = $1 AND session_id = $2")
            .bind(website_id)
            .bind(session_id)
            .fetch_optional(&mut **tx)
            .await?;
    let can_reuse = linked.iter().all(|(value,)| value == distinct_id)
        && session_distinct
            .and_then(|(value,)| value)
            .is_none_or(|value| value == distinct_id);
    if can_reuse {
        sqlx::query(
            "INSERT INTO session_link (website_id, session_id, distinct_id, created_at) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
        )
        .bind(website_id)
        .bind(session_id)
        .bind(distinct_id)
        .bind(created_at)
        .execute(&mut **tx)
        .await?;
    }
    Ok(can_reuse)
}

async fn best_effort_claim_session(
    tx: &mut Transaction<'_, Postgres>,
    website_id: Uuid,
    session_id: Uuid,
    distinct_id: &str,
    created_at: DateTime<Utc>,
) -> Result<bool, sqlx::Error> {
    sqlx::query("SAVEPOINT claim_session")
        .execute(&mut **tx)
        .await?;
    match try_claim_session(tx, website_id, session_id, distinct_id, created_at).await {
        Ok(claimed) => {
            sqlx::query("RELEASE SAVEPOINT claim_session")
                .execute(&mut **tx)
                .await?;
            Ok(claimed)
        }
        Err(_) => {
            sqlx::query("ROLLBACK TO SAVEPOINT claim_session")
                .execute(&mut **tx)
                .await?;
            Ok(false)
        }
    }
}

async fn insert_flattened_data(
    tx: &mut Transaction<'_, Postgres>,
    table: &str,
    website_id: Uuid,
    owner_id: Uuid,
    data: &Map<String, Value>,
    distinct_id: Option<&str>,
    created_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    let mut fields = Vec::new();
    flatten_event_data(data, "", &mut fields);
    for field in fields {
        if table == "event_data" {
            sqlx::query(
                r#"INSERT INTO event_data (
                    event_data_id, website_id, website_event_id, data_key,
                    string_value, number_value, date_value, data_type, created_at
                ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)"#,
            )
            .bind(Uuid::new_v4())
            .bind(website_id)
            .bind(owner_id)
            .bind(field.key)
            .bind(field.string_value)
            .bind(field.number_value)
            .bind(field.date_value)
            .bind(field.data_type)
            .bind(created_at)
            .execute(&mut **tx)
            .await?;
        } else {
            sqlx::query(
                r#"INSERT INTO session_data (
                    session_data_id, website_id, session_id, data_key,
                    string_value, number_value, date_value, data_type, distinct_id, created_at
                ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
                ON CONFLICT (session_id, data_key) DO UPDATE SET
                    website_id = excluded.website_id,
                    string_value = excluded.string_value,
                    number_value = excluded.number_value,
                    date_value = excluded.date_value,
                    data_type = excluded.data_type,
                    distinct_id = excluded.distinct_id,
                    created_at = excluded.created_at"#,
            )
            .bind(Uuid::new_v4())
            .bind(website_id)
            .bind(owner_id)
            .bind(field.key)
            .bind(field.string_value)
            .bind(field.number_value)
            .bind(field.date_value)
            .bind(field.data_type)
            .bind(distinct_id)
            .bind(created_at)
            .execute(&mut **tx)
            .await?;
        }
    }
    Ok(())
}

fn parse_event_url(payload: &SendPayload) -> Result<(Url, String), CollectionError> {
    let base = Url::parse(&format!(
        "https://{}",
        payload.hostname.as_deref().unwrap_or("localhost")
    ))
    .map_err(|_| bad_request("Invalid hostname."))?;
    let url = base
        .join(payload.url.as_deref().unwrap_or("undefined"))
        .map_err(|_| bad_request("Invalid URL."))?;
    let mut path = if url.path() == "/undefined" {
        String::new()
    } else {
        url.path().to_string()
    };
    if let Some(fragment) = url.fragment() {
        path.push('#');
        path.push_str(fragment);
    }
    if env::var("REMOVE_TRAILING_SLASH").is_ok() && path != "/" {
        if let Some(hash) = path.find('#') {
            let (before, after) = path.split_at(hash);
            path = if before == "/" {
                format!("/{after}")
            } else {
                format!("{}{after}", before.trim_end_matches('/'))
            };
        } else {
            path = path.trim_end_matches('/').to_string();
        }
    }
    Ok((url, path))
}

async fn save_event(
    tx: &mut Transaction<'_, Postgres>,
    payload: &SendPayload,
    source_id: Uuid,
    session: &ResolvedSession,
    created_at: DateTime<Utc>,
    event_type: i32,
) -> Result<(), CollectionError> {
    let (url, mut url_path) = parse_event_url(payload)?;
    if event_type == 5 {
        url_path = if url.path() == "/undefined" {
            String::new()
        } else {
            url.path().to_string()
        };
    } else {
        url_path = crate::decode_uri(&url_path, true);
    }
    let event_domain = payload
        .hostname
        .as_deref()
        .and_then(|hostname| Url::parse(&format!("https://{hostname}")).ok())
        .and_then(|url| url.host_str().map(str::to_string))
        .or_else(|| url.host_str().map(str::to_string))
        .map(|value| value.trim_start_matches("www.").to_ascii_lowercase());
    let (referrer_path, referrer_query, referrer_domain) =
        if let Some(referrer) = payload.referrer.as_deref() {
            let base = Url::parse(&format!(
                "https://{}",
                event_domain.as_deref().unwrap_or("localhost")
            ))
            .map_err(|_| bad_request("Invalid referrer."))?;
            let parsed = base
                .join(referrer)
                .map_err(|_| bad_request("Invalid referrer."))?;
            let domain = parsed
                .host_str()
                .map(|value| value.trim_start_matches("www.").to_ascii_lowercase())
                .filter(|value| Some(value.as_str()) != event_domain.as_deref());
            (
                trunc_opt(Some(&crate::decode_uri(parsed.path(), true)), 500),
                trunc_opt(parsed.query(), 500),
                trunc_opt(domain.as_deref(), 500),
            )
        } else {
            (None, None, None)
        };
    let query_value = |key: &str| {
        url.query_pairs()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.chars().take(255).collect::<String>())
    };
    let event_id = Uuid::new_v4();
    let event_name = trunc_opt(payload.name.as_deref(), 50);
    sqlx::query(
        r#"INSERT INTO website_event (
            event_id, website_id, session_id, visit_id, created_at,
            url_path, url_query, utm_source, utm_medium, utm_campaign,
            utm_content, utm_term, referrer_path, referrer_query, referrer_domain,
            page_title, gclid, fbclid, msclkid, ttclid, li_fat_id, twclid,
            event_type, event_name, tag, hostname, lcp, inp, cls, fcp, ttfb
        ) VALUES (
            $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,
            $17,$18,$19,$20,$21,$22,$23,$24,$25,$26,$27,$28,$29,$30,$31
        )"#,
    )
    .bind(event_id)
    .bind(source_id)
    .bind(session.session_id)
    .bind(session.visit_id)
    .bind(created_at)
    .bind(trunc_opt(Some(&url_path), 500).unwrap_or_default())
    .bind(if event_type == 5 {
        None
    } else {
        trunc_opt(url.query(), 500)
    })
    .bind(if event_type == 5 {
        None
    } else {
        query_value("utm_source")
    })
    .bind(if event_type == 5 {
        None
    } else {
        query_value("utm_medium")
    })
    .bind(if event_type == 5 {
        None
    } else {
        query_value("utm_campaign")
    })
    .bind(if event_type == 5 {
        None
    } else {
        query_value("utm_content")
    })
    .bind(if event_type == 5 {
        None
    } else {
        query_value("utm_term")
    })
    .bind(if event_type == 5 { None } else { referrer_path })
    .bind(if event_type == 5 {
        None
    } else {
        referrer_query
    })
    .bind(if event_type == 5 {
        None
    } else {
        referrer_domain
    })
    .bind(trunc_opt(
        payload
            .title
            .as_deref()
            .map(|title| crate::decode_uri(title, false))
            .as_deref(),
        500,
    ))
    .bind(query_value("gclid"))
    .bind(query_value("fbclid"))
    .bind(query_value("msclkid"))
    .bind(query_value("ttclid"))
    .bind(query_value("li_fat_id"))
    .bind(query_value("twclid"))
    .bind(event_type)
    .bind(if event_type == 5 {
        None
    } else {
        event_name.clone()
    })
    .bind(if event_type == 5 {
        None
    } else {
        trunc_opt(payload.tag.as_deref(), 50)
    })
    .bind(if event_type == 5 {
        None
    } else {
        trunc_opt(payload.hostname.as_deref().or_else(|| url.host_str()), 100)
    })
    .bind(payload.lcp.and_then(Decimal::from_f64_retain))
    .bind(payload.inp.and_then(Decimal::from_f64_retain))
    .bind(payload.cls.and_then(Decimal::from_f64_retain))
    .bind(payload.fcp.and_then(Decimal::from_f64_retain))
    .bind(payload.ttfb.and_then(Decimal::from_f64_retain))
    .execute(&mut **tx)
    .await
    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    if event_type != 5 {
        if let Some(data) = &payload.data {
            insert_flattened_data(
                tx,
                "event_data",
                source_id,
                event_id,
                data,
                None,
                created_at,
            )
            .await
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
            let revenue = data.get("revenue").and_then(|value| match value {
                Value::Number(number) => number.as_f64(),
                Value::String(value) => value.parse().ok(),
                _ => None,
            });
            let currency = data
                .get("currency")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            if let (Some(revenue), Some(currency)) = (revenue, currency) {
                if revenue.is_finite() && revenue > 0.0 {
                    sqlx::query(
                        r#"INSERT INTO revenue (
                        revenue_id, website_id, session_id, event_id,
                        event_name, currency, revenue, created_at
                    ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)"#,
                    )
                    .bind(Uuid::new_v4())
                    .bind(source_id)
                    .bind(session.session_id)
                    .bind(event_id)
                    .bind(event_name.unwrap_or_default())
                    .bind(currency.chars().take(10).collect::<String>())
                    .bind(Decimal::from_str(&format!("{revenue:.4}")).ok())
                    .bind(created_at)
                    .execute(&mut **tx)
                    .await
                    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
                }
            }
        }
    }
    Ok(())
}

fn scrub_error_text(value: &str, limit: usize) -> String {
    static URL_RE: OnceLock<Regex> = OnceLock::new();
    static EMAIL_RE: OnceLock<Regex> = OnceLock::new();
    static TOKEN_RE: OnceLock<Regex> = OnceLock::new();
    static SECRET_RE: OnceLock<Regex> = OnceLock::new();
    let value: String = value.chars().take(limit).collect();
    let value = URL_RE
        .get_or_init(|| Regex::new(r"https?://[^\s)]+").unwrap())
        .replace_all(&value, |captures: &regex::Captures<'_>| {
            let raw = captures.get(0).map(|value| value.as_str()).unwrap_or("");
            static LOCATION_RE: OnceLock<Regex> = OnceLock::new();
            let location = LOCATION_RE
                .get_or_init(|| Regex::new(r":\d+:\d+$").unwrap())
                .find(raw)
                .map(|value| value.as_str())
                .unwrap_or("");
            let url = raw.strip_suffix(location).unwrap_or(raw);
            Url::parse(url)
                .ok()
                .map(|url| {
                    format!(
                        "{}{}{}",
                        url.origin().ascii_serialization(),
                        url.path(),
                        location
                    )
                })
                .unwrap_or_else(|| "[url]".to_string())
        });
    let value = EMAIL_RE
        .get_or_init(|| Regex::new(r"(?i)[\w.+-]+@[\w.-]+\.[a-z]{2,}").unwrap())
        .replace_all(&value, "[email]");
    let value = TOKEN_RE
        .get_or_init(|| Regex::new(r"\beyJ[\w-]+\.[\w-]+\.[\w-]+\b").unwrap())
        .replace_all(&value, "[token]");
    SECRET_RE
        .get_or_init(|| {
            Regex::new(r"(?i)\b(password|secret|token|authorization|api[_-]?key)\s*[:=]\s*[^\s,;]+")
                .unwrap()
        })
        .replace_all(&value, "$1=[redacted]")
        .chars()
        .take(limit)
        .collect()
}

fn error_url_path(value: &str) -> String {
    Url::parse("https://localhost")
        .and_then(|base| base.join(value))
        .ok()
        .map(|url| scrub_error_text(url.path(), 500))
        .unwrap_or_else(|| "/".to_string())
}

fn normalize_error(
    payload: &SendPayload,
    website_id: Uuid,
) -> (Uuid, String, String, String, Value, Value, String) {
    static FRAME_RE: OnceLock<Regex> = OnceLock::new();
    static UUID_RE: OnceLock<Regex> = OnceLock::new();
    static NUMBER_RE: OnceLock<Regex> = OnceLock::new();
    let name = scrub_error_text(payload.name.as_deref().unwrap_or(""), 200);
    let message = scrub_error_text(payload.message.as_deref().unwrap_or(""), 2000);
    let stack = scrub_error_text(payload.stack.as_deref().unwrap_or(""), 16_000);
    let frame_re = FRAME_RE.get_or_init(|| {
        Regex::new(
            r"^(?:at\s+)?(?:(.*?)\s*[@(])?((?:https?://|/|webpack:|file:)[^\s]*?):(\d+):(\d+)\)?$",
        )
        .unwrap()
    });
    let frames: Vec<Value> = stack
        .lines()
        .take(51)
        .filter_map(|line| frame_re.captures(line.trim()))
        .filter_map(|captures| {
            Some(json!({
                "filename": captures.get(2)?.as_str(),
                "function": captures.get(1).map(|value| value.as_str().trim()).unwrap_or(""),
                "line": captures.get(3)?.as_str().parse::<i64>().ok()?,
                "column": captures.get(4)?.as_str().parse::<i64>().ok()?
            }))
        })
        .take(50)
        .collect();
    let components = if let Some(custom) = &payload.fingerprint {
        json!(std::iter::once("custom".to_string())
            .chain(custom.iter().map(|value| scrub_error_text(value, 200)))
            .collect::<Vec<_>>())
    } else if !frames.is_empty() {
        let frame_components: Vec<Value> = frames
            .iter()
            .take(10)
            .map(|frame| {
                let filename = frame.get("filename").and_then(Value::as_str).unwrap_or("");
                static BUNDLE_HASH_RE: OnceLock<Regex> = OnceLock::new();
                let filename = BUNDLE_HASH_RE
                    .get_or_init(|| Regex::new(r"(?i)[.-][a-f\d]{8,}(\.)").unwrap())
                    .replace_all(filename, "$1");
                json!([
                    filename,
                    frame.get("function").and_then(Value::as_str).unwrap_or(""),
                    frame.get("line").and_then(Value::as_i64).unwrap_or(0),
                    frame.get("column").and_then(Value::as_i64).unwrap_or(0)
                ])
            })
            .collect();
        json!([name, frame_components])
    } else {
        let normalized = UUID_RE
            .get_or_init(|| Regex::new(r"(?i)\b[0-9a-f]{8}-[0-9a-f-]{27,}\b").unwrap())
            .replace_all(&message, "<id>");
        let normalized = NUMBER_RE
            .get_or_init(|| Regex::new(r"\b\d+\b").unwrap())
            .replace_all(&normalized, "<n>");
        json!([name, normalized])
    };
    let fingerprint = hex::encode(Sha256::digest(
        serde_json::to_vec(&json!([1, components])).unwrap_or_default(),
    ));
    let issue_id = Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("{website_id}:1:{fingerprint}").as_bytes(),
    );
    let tags = Value::Object(
        payload
            .tags
            .clone()
            .unwrap_or_default()
            .into_iter()
            .take(20)
            .filter(|(key, _)| {
                let key = key.to_ascii_lowercase();
                ![
                    "password",
                    "secret",
                    "token",
                    "authorization",
                    "cookie",
                    "email",
                    "api_key",
                    "apikey",
                ]
                .iter()
                .any(|value| key.contains(value))
            })
            .filter_map(|(key, value)| {
                Some((
                    scrub_error_text(&key, 50),
                    Value::String(scrub_error_text(value.as_str()?, 200)),
                ))
            })
            .collect(),
    );
    (
        issue_id,
        fingerprint,
        name,
        message,
        Value::Array(frames),
        tags,
        stack,
    )
}

async fn consume_error_quota(pool: &PgPool, website_id: Uuid) -> Result<bool, CollectionError> {
    let configured = env::var("ERRORS_PER_MINUTE")
        .ok()
        .and_then(|value| value.parse::<i32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(120)
        .min(100_000);
    let row = sqlx::query(
        r#"INSERT INTO error_rate_limit (website_id, bucket, count)
           VALUES ($1, date_trunc('minute', now()), 1)
           ON CONFLICT (website_id) DO UPDATE SET
             bucket = excluded.bucket,
             count = CASE WHEN error_rate_limit.bucket = excluded.bucket
               THEN least(error_rate_limit.count + 1, $2 + 1) ELSE 1 END
           RETURNING count"#,
    )
    .bind(website_id)
    .bind(configured)
    .fetch_one(pool)
    .await
    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    Ok(row.get::<i32, _>("count") <= configured)
}

async fn persist_error(
    pool: &PgPool,
    payload: &SendPayload,
    headers: &HeaderMap,
) -> Result<Value, CollectionError> {
    if serde_json::to_vec(payload)
        .map(|value| value.len())
        .unwrap_or(usize::MAX)
        > 48_000
    {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            "Error payload exceeds 48000 bytes.".to_string(),
        ));
    }
    let website_id = Uuid::parse_str(payload.website.as_deref().unwrap_or(""))
        .map_err(|_| bad_request("Invalid website UUID."))?;
    let event_id = Uuid::parse_str(payload.event_id.as_deref().unwrap_or(""))
        .map_err(|_| bad_request("Invalid eventId."))?;
    let created_at = DateTime::<Utc>::from_timestamp_millis(payload.timestamp.unwrap_or_default())
        .ok_or_else(|| bad_request("Invalid timestamp."))?;
    let website: Option<(Option<DateTime<Utc>>,)> =
        sqlx::query_as("SELECT reset_at FROM website WHERE website_id = $1 AND deleted_at IS NULL")
            .bind(website_id)
            .fetch_optional(pool)
            .await
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let Some((reset_at,)) = website else {
        return Err(bad_request("Website not found."));
    };
    if created_at > Utc::now() + chrono::Duration::minutes(5)
        || reset_at.is_some_and(|reset_at| created_at <= reset_at)
    {
        return Err(bad_request("Error timestamp is out of range."));
    }
    let cache = if headers.contains_key("x-umami-cache") {
        Some(
            parse_cache(headers, website_id)
                .ok_or_else(|| bad_request("Invalid session token."))?,
        )
    } else {
        None
    };
    // Public error ingestion never trusts caller-provided network identity.
    let ip = extract_client_ip(headers);
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    if is_bot(&user_agent) {
        return Ok(json!({ "ok": false, "reason": "bot" }));
    }
    if is_blocked_ip(&ip) {
        return Err((StatusCode::FORBIDDEN, "Forbidden.".to_string()));
    }
    if !consume_error_quota(pool, website_id).await? {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            "Error quota exceeded.".to_string(),
        ));
    }
    let distinct_id = trunc_opt(payload.id.as_deref(), 50);
    let mut session = resolve_cached_session(
        website_id,
        &ip,
        &user_agent,
        distinct_id.as_deref(),
        payload.salt.as_deref(),
        created_at,
        true,
        cache.as_ref(),
    );
    let (browser, os, device) = infer_ua_fields(&user_agent, payload.screen.as_deref());
    let browser = trunc_opt(browser.as_deref(), 50);
    let os = trunc_opt(os.as_deref(), 50);
    let device = trunc_opt(Some(&device), 50);
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    if let (Some(distinct_id), Some(cache)) = (distinct_id.as_deref(), cache.as_ref()) {
        let fingerprint_session = session_id_for(
            website_id,
            &ip,
            &user_agent,
            None,
            payload.salt.as_deref(),
            created_at,
        );
        if session.session_drift && fingerprint_session == cache.session_id {
            let link_hash = sha512_hex(&[&cache.session_id.to_string(), distinct_id]);
            if cache.session_link_id.as_deref() == Some(link_hash.as_str())
                || best_effort_claim_session(
                    &mut tx,
                    website_id,
                    cache.session_id,
                    distinct_id,
                    created_at,
                )
                .await
                .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
            {
                session = resolve_cached_session(
                    website_id,
                    &ip,
                    &user_agent,
                    None,
                    payload.salt.as_deref(),
                    created_at,
                    true,
                    Some(cache),
                );
            }
        }
    }
    sqlx::query(
        r#"INSERT INTO session (
            session_id, website_id, browser, os, device, screen,
            language, distinct_id, created_at
        ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
        ON CONFLICT (session_id) DO NOTHING"#,
    )
    .bind(session.session_id)
    .bind(website_id)
    .bind(&browser)
    .bind(&os)
    .bind(&device)
    .bind(trunc_opt(payload.screen.as_deref(), 11))
    .bind(trunc_opt(payload.language.as_deref(), 35))
    .bind(&distinct_id)
    .bind(created_at)
    .execute(&mut *tx)
    .await
    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let (issue_id, fingerprint, name, message, frames, tags, stack) =
        normalize_error(payload, website_id);
    let title: String = format!("{name}: {message}").chars().take(2200).collect();
    sqlx::query(
        r#"INSERT INTO error_issue (issue_id, website_id, fingerprint, title)
           VALUES ($1,$2,$3,$4)
           ON CONFLICT (website_id, grouping_version, fingerprint) DO NOTHING"#,
    )
    .bind(issue_id)
    .bind(website_id)
    .bind(&fingerprint)
    .bind(title)
    .execute(&mut *tx)
    .await
    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    sqlx::query(
        r#"INSERT INTO error_event (
            website_id, event_id, issue_id, session_id, visit_id,
            created_at, received_at, name, message, stack, frames, tags,
            handled, url_path, release, environment, browser, os, device
        ) VALUES (
            $1,$2,$3,$4,$5,$6,now(),$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18
        ) ON CONFLICT (website_id, event_id) DO NOTHING"#,
    )
    .bind(website_id)
    .bind(event_id)
    .bind(issue_id)
    .bind(session.session_id)
    .bind(session.visit_id)
    .bind(created_at)
    .bind(name)
    .bind(message)
    .bind(stack)
    .bind(frames)
    .bind(tags)
    .bind(payload.handled.unwrap_or(false))
    .bind(error_url_path(payload.url.as_deref().unwrap_or("/")))
    .bind(scrub_error_text(
        payload.release.as_deref().unwrap_or(""),
        100,
    ))
    .bind(scrub_error_text(
        payload.environment.as_deref().unwrap_or("production"),
        50,
    ))
    .bind(browser.unwrap_or_default())
    .bind(os.unwrap_or_default())
    .bind(device.unwrap_or_default())
    .execute(&mut *tx)
    .await
    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    tx.commit()
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let mut claims = json!({
        "websiteId": website_id.to_string(),
        "sessionId": session.session_id.to_string(),
        "visitId": session.visit_id.to_string(),
        "iat": session.iat,
        "type": "cache"
    });
    if let Some(session_link_id) = cache.and_then(|cache| cache.session_link_id) {
        claims["sessionLinkId"] = json!(session_link_id);
    }
    let cache = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret_hash().as_bytes()),
    )
    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    Ok(json!({
        "ok": true,
        "eventId": event_id.to_string(),
        "issueId": issue_id.to_string(),
        "cache": cache
    }))
}

async fn persist_send_envelope(
    pool: &PgPool,
    envelope: &SendEnvelope,
    headers: &HeaderMap,
) -> Result<Value, CollectionError> {
    validate_envelope(envelope)?;
    if envelope.kind == "error" {
        return persist_error(pool, &envelope.payload, headers).await;
    }
    let payload = &envelope.payload;
    let source_id = [
        payload.website.as_deref(),
        payload.link.as_deref(),
        payload.pixel.as_deref(),
    ]
    .into_iter()
    .flatten()
    .next()
    .and_then(|value| Uuid::parse_str(value).ok())
    .ok_or_else(|| bad_request("Invalid source UUID."))?;
    let website_id = payload
        .website
        .as_deref()
        .and_then(|value| Uuid::parse_str(value).ok());
    let cache = website_id.and_then(|website_id| parse_cache(headers, website_id));
    if website_id.is_some() && cache.is_none() {
        let exists: Option<(Uuid,)> = sqlx::query_as(
            "SELECT website_id FROM website WHERE website_id = $1 AND deleted_at IS NULL",
        )
        .bind(source_id)
        .fetch_optional(pool)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
        if exists.is_none() {
            return Err(bad_request("Website not found."));
        }
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
                .and_then(|value| value.to_str().ok())
                .map(str::to_string)
        })
        .unwrap_or_default();
    if is_bot(&user_agent) {
        return Ok(json!({ "beep": "boop" }));
    }
    if is_blocked_ip(&ip) {
        return Err((StatusCode::FORBIDDEN, "Forbidden.".to_string()));
    }
    let created_at = payload
        .timestamp
        .and_then(|timestamp| Utc.timestamp_opt(timestamp, 0).single())
        .unwrap_or_else(Utc::now);
    let historical = payload.timestamp.is_some();
    let distinct_id = trunc_opt(payload.id.as_deref(), 50);
    let mut session = resolve_cached_session(
        source_id,
        &ip,
        &user_agent,
        distinct_id.as_deref(),
        payload.salt.as_deref(),
        created_at,
        historical,
        cache.as_ref(),
    );
    let (ua_browser, ua_os, ua_device) = infer_ua_fields(&user_agent, payload.screen.as_deref());
    let browser = trunc_opt(payload.browser.as_deref().or(ua_browser.as_deref()), 20);
    let os = trunc_opt(payload.os.as_deref().or(ua_os.as_deref()), 20);
    let device = trunc_opt(payload.device.as_deref(), 20).or(Some(ua_device));
    let screen = trunc_opt(payload.screen.as_deref(), 11);
    let language = trunc_opt(payload.language.as_deref(), 35);
    let (country, region, city) = extract_location_headers(headers);
    let country = trunc_opt(country.as_deref(), 2);
    let region = trunc_opt(region.as_deref(), 20);
    let city = trunc_opt(city.as_deref(), 50);
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    if let (Some(website_id), Some(distinct_id), Some(cache)) =
        (website_id, distinct_id.as_deref(), cache.as_ref())
    {
        let fingerprint_session = session_id_for(
            source_id,
            &ip,
            &user_agent,
            None,
            payload.salt.as_deref(),
            created_at,
        );
        if session.session_drift && fingerprint_session == cache.session_id {
            let link_hash = sha512_hex(&[&cache.session_id.to_string(), distinct_id]);
            let can_reuse = cache.session_link_id.as_deref() == Some(link_hash.as_str())
                || best_effort_claim_session(
                    &mut tx,
                    website_id,
                    cache.session_id,
                    distinct_id,
                    created_at,
                )
                .await
                .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
            if can_reuse {
                session = resolve_cached_session(
                    source_id,
                    &ip,
                    &user_agent,
                    None,
                    payload.salt.as_deref(),
                    created_at,
                    historical,
                    Some(cache),
                );
            }
        }
    }
    ensure_session(
        &mut tx,
        source_id,
        &session,
        &browser,
        &os,
        &device,
        &screen,
        &language,
        &country,
        &region,
        &city,
        &distinct_id,
        created_at,
        cache.as_ref(),
    )
    .await
    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    let mut session_link_id = cache
        .as_ref()
        .and_then(|cache| cache.session_link_id.clone());
    match envelope.kind.as_str() {
        "event" => {
            let event_type = if payload.link.is_some() {
                3
            } else if payload.pixel.is_some() {
                4
            } else if payload.name.is_some() {
                2
            } else {
                1
            };
            save_event(
                &mut tx, payload, source_id, &session, created_at, event_type,
            )
            .await?;
        }
        "performance" => {
            save_event(&mut tx, payload, source_id, &session, created_at, 5).await?;
        }
        "identify" => {
            if let (Some(website_id), Some(distinct_id)) = (website_id, distinct_id.as_deref()) {
                let link_hash = sha512_hex(&[&session.session_id.to_string(), distinct_id]);
                if session_link_id.as_deref() != Some(link_hash.as_str()) {
                    sqlx::query("SAVEPOINT identity_write")
                        .execute(&mut *tx)
                        .await
                        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
                    let linked = sqlx::query(
                        "INSERT INTO session_link (website_id, session_id, distinct_id, created_at) VALUES ($1,$2,$3,$4) ON CONFLICT DO NOTHING",
                    )
                    .bind(website_id)
                    .bind(session.session_id)
                    .bind(distinct_id)
                    .bind(created_at)
                    .execute(&mut *tx)
                    .await;
                    let linked = match linked {
                        Ok(_) => sqlx::query(
                            "UPDATE session SET distinct_id = $1 WHERE website_id = $2 AND session_id = $3 AND COALESCE(distinct_id, '') = ''",
                        )
                        .bind(distinct_id)
                        .bind(website_id)
                        .bind(session.session_id)
                        .execute(&mut *tx)
                        .await,
                        Err(error) => Err(error),
                    };
                    match linked {
                        Ok(_) => {
                            sqlx::query("RELEASE SAVEPOINT identity_write")
                                .execute(&mut *tx)
                                .await
                                .map_err(|error| {
                                    (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
                                })?;
                            session_link_id = Some(link_hash);
                        }
                        Err(_) => {
                            sqlx::query("ROLLBACK TO SAVEPOINT identity_write")
                                .execute(&mut *tx)
                                .await
                                .map_err(|error| {
                                    (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
                                })?;
                        }
                    }
                }
            }
            if let Some(data) = &payload.data {
                let website_id =
                    website_id.ok_or_else(|| bad_request("Identify data requires a website."))?;
                insert_flattened_data(
                    &mut tx,
                    "session_data",
                    website_id,
                    session.session_id,
                    data,
                    distinct_id.as_deref(),
                    created_at,
                )
                .await
                .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
            }
        }
        _ => return Err(bad_request("Unsupported collection type.")),
    }
    tx.commit()
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let mut claims = json!({
        "websiteId": website_id.map(|value| value.to_string()),
        "sessionId": session.session_id.to_string(),
        "visitId": session.visit_id.to_string(),
        "iat": session.iat,
        "type": "cache"
    });
    if let Some(session_link_id) = session_link_id {
        claims["sessionLinkId"] = json!(session_link_id);
    }
    let cache = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret_hash().as_bytes()),
    )
    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    Ok(json!({
        "cache": cache,
        "sessionId": session.session_id.to_string(),
        "visitId": session.visit_id.to_string()
    }))
}

pub(super) async fn handle_send(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(envelope): Json<SendEnvelope>,
) -> Response {
    let Some(pool) = &state.db else {
        return with_rust_header(
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(collection_error_body(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Database unavailable",
                )),
            )
                .into_response(),
        );
    };
    match persist_send_envelope(pool, &envelope, &headers).await {
        Ok(body) => with_rust_header(Json(body).into_response()),
        Err((status, message)) => {
            let message = if status == StatusCode::INTERNAL_SERVER_ERROR {
                "Collection failed.".to_string()
            } else {
                message
            };
            let mut response =
                (status, Json(collection_error_body(status, message))).into_response();
            if status == StatusCode::TOO_MANY_REQUESTS {
                response
                    .headers_mut()
                    .insert("retry-after", HeaderValue::from_static("60"));
            }
            with_rust_header(response)
        }
    }
}

pub(super) async fn handle_batch(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let Ok(Value::Array(batch)) = serde_json::from_slice::<Value>(&body) else {
        return with_rust_header(
            (
                StatusCode::BAD_REQUEST,
                Json(collection_error_body(
                    StatusCode::BAD_REQUEST,
                    "Expected a JSON array.",
                )),
            )
                .into_response(),
        );
    };
    if batch.len() > 500 {
        return with_rust_header(
            (
                StatusCode::BAD_REQUEST,
                Json(collection_error_body(
                    StatusCode::BAD_REQUEST,
                    "Batch cannot exceed 500 items.",
                )),
            )
                .into_response(),
        );
    }
    let Some(pool) = &state.db else {
        return with_rust_header(
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(collection_error_body(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Database unavailable",
                )),
            )
                .into_response(),
        );
    };
    let mut details = Vec::new();
    let mut cache = None;
    for (index, value) in batch.iter().enumerate() {
        let envelope = match serde_json::from_value::<SendEnvelope>(value.clone()) {
            Ok(envelope) => envelope,
            Err(_) => {
                details.push(json!({
                    "index": index,
                    "response": collection_error_body(
                        StatusCode::BAD_REQUEST,
                        "Invalid collection payload.",
                    )
                }));
                continue;
            }
        };
        match persist_send_envelope(pool, &envelope, &headers).await {
            Ok(response) => {
                if cache.is_none() {
                    cache = response.get("cache").cloned();
                }
            }
            Err((status, message)) => {
                let message = if status == StatusCode::INTERNAL_SERVER_ERROR {
                    "Collection failed.".to_string()
                } else {
                    message
                };
                details.push(json!({
                    "index": index,
                    "response": collection_error_body(status, message)
                }));
            }
        }
    }
    with_rust_header(
        Json(json!({
            "size": batch.len(),
            "processed": batch.len() - details.len(),
            "errors": details.len(),
            "details": details,
            "cache": cache
        }))
        .into_response(),
    )
}

pub(super) async fn handle_v2(
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
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "Database unavailable" })),
            )
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

    with_rust_header(
        (
            StatusCode::OK,
            Json(json!({
                "accepted": accepted,
                "rejected": rejected,
                "dropped": dropped
            })),
        )
            .into_response(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(payload: SendPayload) -> SendEnvelope {
        SendEnvelope {
            kind: "event".to_string(),
            payload,
        }
    }

    #[test]
    fn validates_collection_source_and_write_bounds() {
        let website = Uuid::new_v4().to_string();
        let mut payload = SendPayload {
            website: Some(website.clone()),
            ..Default::default()
        };
        assert!(validate_envelope(&event(payload.clone())).is_ok());

        payload.link = Some(Uuid::new_v4().to_string());
        assert!(validate_envelope(&event(payload.clone())).is_err());

        payload.link = None;
        payload.name = Some("=cmd()".to_string());
        assert!(validate_envelope(&event(payload.clone())).is_err());

        payload.name = None;
        payload.data = Some(Map::from_iter([
            ("revenue".to_string(), json!(1e14)),
            ("currency".to_string(), json!("USD")),
        ]));
        assert!(validate_envelope(&event(payload)).is_err());
    }

    #[test]
    fn flattens_dates_nulls_and_numbers_like_postgres_queries() {
        let data = Map::from_iter([
            ("date".to_string(), json!("2026-11-19T00:00:00Z")),
            ("null".to_string(), Value::Null),
            ("number".to_string(), json!(12.34567)),
        ]);
        let mut fields = Vec::new();
        flatten_event_data(&data, "", &mut fields);

        assert_eq!(
            fields
                .iter()
                .find(|field| field.key == "date")
                .unwrap()
                .data_type,
            4
        );
        assert_eq!(
            fields
                .iter()
                .find(|field| field.key == "null")
                .unwrap()
                .string_value
                .as_deref(),
            Some("null")
        );
        assert_eq!(
            fields
                .iter()
                .find(|field| field.key == "number")
                .unwrap()
                .string_value
                .as_deref(),
            Some("12.3457")
        );
    }

    #[test]
    fn scrubs_sensitive_error_text() {
        let scrubbed = scrub_error_text(
            "https://example.com/path?token=private user@example.com authorization=secret",
            2000,
        );
        assert_eq!(
            scrubbed,
            "https://example.com/path [email] authorization=[redacted]"
        );
        assert!(!scrubbed.contains("private"));
        assert!(!scrubbed.contains("user@example.com"));
    }

    #[test]
    fn matches_exact_and_cidr_blocked_ips() {
        assert!(ip_matches_cidr(
            "203.0.113.7".parse().unwrap(),
            "203.0.113.0/24"
        ));
        assert!(!ip_matches_cidr(
            "203.0.114.7".parse().unwrap(),
            "203.0.113.0/24"
        ));
        assert!(ip_matches_cidr(
            "2001:db8::1".parse().unwrap(),
            "2001:db8::/32"
        ));
    }
}
