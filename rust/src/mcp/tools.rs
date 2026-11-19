use axum::{
    body::{to_bytes, Body},
    http::{header, HeaderMap, Method, Request, StatusCode},
    Router,
};
use chrono::{DateTime, Duration, NaiveDate, SecondsFormat, TimeZone, Utc};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use tower::ServiceExt;
use url::form_urlencoded;
use uuid::Uuid;

const MAX_RESPONSE_BODY: usize = 1024 * 1024;
const MAX_URI_LENGTH: usize = 16 * 1024;
const DEFAULT_PAGE_SIZE: i64 = 20;
const MAX_PAGE_SIZE: i64 = 100;
const DEFAULT_METRIC_LIMIT: i64 = 20;
const MAX_METRIC_LIMIT: i64 = 500;
const MAX_SESSION_ACTIVITY: usize = 200;

const FILTERS: &[&str] = &[
    "path",
    "referrer",
    "title",
    "query",
    "hostname",
    "event",
    "tag",
    "browser",
    "os",
    "device",
    "country",
    "region",
    "city",
    "language",
    "distinctId",
    "utmSource",
    "utmMedium",
    "utmCampaign",
    "utmContent",
    "utmTerm",
    "segment",
    "cohort",
    "match",
];

const METRIC_TYPES: &[&str] = &[
    "path",
    "entry",
    "exit",
    "title",
    "query",
    "hostname",
    "referrer",
    "domain",
    "channel",
    "event",
    "tag",
    "browser",
    "os",
    "device",
    "screen",
    "language",
    "country",
    "region",
    "city",
    "distinctId",
    "utmSource",
    "utmMedium",
    "utmCampaign",
    "utmContent",
    "utmTerm",
];

const VISITOR_METRICS: &[&str] = &[
    "browser",
    "os",
    "device",
    "screen",
    "language",
    "country",
    "region",
    "city",
    "distinctId",
    "referrer",
    "domain",
    "channel",
    "utmSource",
    "utmMedium",
    "utmCampaign",
    "utmContent",
    "utmTerm",
];

fn schema(
    name: &str,
    title: &str,
    description: &str,
    properties: Map<String, Value>,
    required: &[&str],
) -> Value {
    json!({
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false
        },
        "annotations": {
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": name != "get_realtime",
            "openWorldHint": false
        }
    })
}

fn props(entries: &[(&str, Value)]) -> Map<String, Value> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn string(description: &str) -> Value {
    json!({"type":"string","description":description})
}

fn nonempty(description: &str) -> Value {
    json!({"type":"string","minLength":1,"description":description})
}

fn uuid(description: &str) -> Value {
    json!({"type":"string","format":"uuid","description":description})
}

fn date(description: &str) -> Value {
    json!({"type":"string","minLength":4,"description":description})
}

fn integer(minimum: i64, maximum: Option<i64>, description: &str) -> Value {
    let mut value = json!({"type":"integer","minimum":minimum,"description":description});
    if let Some(maximum) = maximum {
        value["maximum"] = json!(maximum);
    }
    value
}

fn enumeration(values: &[&str], description: &str) -> Value {
    json!({"type":"string","enum":values,"description":description})
}

fn filters(include_event: bool) -> Value {
    let mut properties = Map::new();
    for name in FILTERS {
        if include_event || *name != "event" {
            properties.insert(
                (*name).to_owned(),
                match *name {
                    "segment" | "cohort" => uuid("Saved audience ID."),
                    "match" => enumeration(&["all", "any"], "Require all or any filters."),
                    _ => nonempty("Analytics filter value."),
                },
            );
        }
    }
    json!({
        "type":"object",
        "properties":properties,
        "additionalProperties":false,
        "description":"Optional analytics filters."
    })
}

pub(super) fn stats_filters_schema() -> Value {
    filters(false)
}

pub(super) fn stats_filters(args: &Map<String, Value>) -> Result<Vec<(String, String)>, Value> {
    let input = Input { args };
    let mut query = Vec::new();
    append_filters(&input, &mut query, false)?;
    Ok(query)
}

fn date_props() -> [(&'static str, Value); 2] {
    [
        ("startAt", date("Start of the range (inclusive), ISO 8601.")),
        (
            "endAt",
            date("End of the range (inclusive), ISO 8601. Defaults to now."),
        ),
    ]
}

pub(super) fn catalog() -> Vec<Value> {
    let mut run_funnel = schema(
        "run_funnel",
        "Run a funnel report",
        "Calculate conversion through saved or ad-hoc page/event steps.",
        props(&[
            ("websiteId", uuid("Website ID from list_websites.")),
            date_props()[0].clone(),
            date_props()[1].clone(),
            ("funnelId", uuid("Saved funnel ID from list_funnels.")),
            (
                "steps",
                json!({
                    "type":"array","minItems":2,"maxItems":8,
                    "items":{
                        "type":"object",
                        "properties":{
                            "type":enumeration(&["path","event"],"Step type."),
                            "value":nonempty("Page path or event name.")
                        },
                        "required":["type","value"],
                        "additionalProperties":false
                    }
                }),
            ),
            (
                "windowMinutes",
                integer(1, Some(43_200), "Maximum minutes between funnel steps."),
            ),
            ("filters", filters(true)),
        ]),
        &["websiteId", "startAt"],
    );
    run_funnel["inputSchema"]["oneOf"] = json!([
        {"required":["funnelId"],"not":{"required":["steps"]}},
        {"required":["steps"],"not":{"required":["funnelId"]}}
    ]);

    vec![
        schema(
            "get_website_daterange",
            "Get website data date range",
            "Return the earliest and latest recorded timestamps for a website.",
            props(&[("websiteId", uuid("Website ID from list_websites."))]),
            &["websiteId"],
        ),
        schema(
            "get_website_traffic",
            "Get website traffic over time",
            "Return pageview and visit time series for a date range.",
            props(&[
                ("websiteId", uuid("Website ID from list_websites.")),
                date_props()[0].clone(),
                date_props()[1].clone(),
                (
                    "unit",
                    enumeration(
                        &["minute", "hour", "day", "month", "year"],
                        "Time-series bucket size.",
                    ),
                ),
                ("timezone", string("IANA timezone. Defaults to UTC.")),
                (
                    "compare",
                    enumeration(&["prev", "yoy"], "Optional comparison period."),
                ),
                ("filters", filters(false)),
            ]),
            &["websiteId", "startAt"],
        ),
        schema(
            "get_website_metrics",
            "Get ranked website metrics",
            "Return a ranked analytics breakdown for one dimension.",
            props(&[
                ("websiteId", uuid("Website ID from list_websites.")),
                date_props()[0].clone(),
                date_props()[1].clone(),
                ("type", enumeration(METRIC_TYPES, "Dimension to rank.")),
                (
                    "limit",
                    integer(1, Some(MAX_METRIC_LIMIT), "Maximum rows to return."),
                ),
                ("offset", integer(0, None, "Rows to skip.")),
                ("search", string("Only include matching values.")),
                ("timezone", string("IANA timezone. Defaults to UTC.")),
                ("filters", filters(true)),
            ]),
            &["websiteId", "startAt", "type"],
        ),
        schema(
            "get_realtime",
            "Get current active visitors",
            "Return the number of visitors active in the last few minutes.",
            props(&[("websiteId", uuid("Website ID from list_websites."))]),
            &["websiteId"],
        ),
        schema(
            "get_events",
            "Get tracked events",
            "List pageviews and custom events, newest first.",
            props(&[
                ("websiteId", uuid("Website ID from list_websites.")),
                date_props()[0].clone(),
                date_props()[1].clone(),
                ("event", string("Custom event name.")),
                ("search", string("Free-text event or URL search.")),
                ("page", integer(1, None, "Page number.")),
                (
                    "pageSize",
                    integer(1, Some(MAX_PAGE_SIZE), "Results per page."),
                ),
                ("filters", filters(true)),
            ]),
            &["websiteId", "startAt"],
        ),
        schema(
            "get_event_stats",
            "Get custom event stats",
            "Return custom-event totals and a comparison period.",
            props(&[
                ("websiteId", uuid("Website ID from list_websites.")),
                date_props()[0].clone(),
                date_props()[1].clone(),
                (
                    "compare",
                    enumeration(&["prev", "yoy"], "Comparison period."),
                ),
                ("filters", filters(true)),
            ]),
            &["websiteId", "startAt"],
        ),
        schema(
            "get_event_series",
            "Get custom events over time",
            "Return custom-event counts grouped by name and time bucket.",
            props(&[
                ("websiteId", uuid("Website ID from list_websites.")),
                date_props()[0].clone(),
                date_props()[1].clone(),
                (
                    "unit",
                    enumeration(
                        &["minute", "hour", "day", "month", "year"],
                        "Time-series bucket size.",
                    ),
                ),
                ("timezone", string("IANA timezone. Defaults to UTC.")),
                (
                    "limit",
                    integer(1, Some(MAX_METRIC_LIMIT), "Maximum event names."),
                ),
                ("filters", filters(true)),
            ]),
            &["websiteId", "startAt"],
        ),
        schema(
            "get_event_properties",
            "Get custom event properties and values",
            "List custom-event properties or ranked values for one property.",
            props(&[
                ("websiteId", uuid("Website ID from list_websites.")),
                date_props()[0].clone(),
                date_props()[1].clone(),
                ("propertyName", nonempty("Property to get values for.")),
                ("eventName", nonempty("Custom event name.")),
                (
                    "dataType",
                    enumeration(
                        &["string", "number", "boolean", "date", "array"],
                        "Property data type.",
                    ),
                ),
                ("filters", filters(true)),
            ]),
            &["websiteId", "startAt"],
        ),
        schema(
            "get_sessions",
            "Get visitor sessions",
            "List visitor sessions, newest first.",
            props(&[
                ("websiteId", uuid("Website ID from list_websites.")),
                date_props()[0].clone(),
                date_props()[1].clone(),
                ("search", string("Free-text session search.")),
                ("page", integer(1, None, "Page number.")),
                (
                    "pageSize",
                    integer(1, Some(MAX_PAGE_SIZE), "Results per page."),
                ),
                ("filters", filters(true)),
            ]),
            &["websiteId", "startAt"],
        ),
        schema(
            "get_session_stats",
            "Get session stats",
            "Return session-level visitor, visit, pageview, event and country totals.",
            props(&[
                ("websiteId", uuid("Website ID from list_websites.")),
                date_props()[0].clone(),
                date_props()[1].clone(),
                ("filters", filters(true)),
            ]),
            &["websiteId", "startAt"],
        ),
        schema(
            "get_session",
            "Get session details",
            "Return one session, its properties and optionally its activity timeline.",
            props(&[
                ("websiteId", uuid("Website ID from list_websites.")),
                ("sessionId", uuid("Session ID from get_sessions.")),
                (
                    "includeActivity",
                    json!({"type":"boolean","description":"Include activity (default true)."}),
                ),
            ]),
            &["websiteId", "sessionId"],
        ),
        schema(
            "get_annotations",
            "Get annotations",
            "List dated notes attached to a website timeline.",
            props(&[
                ("websiteId", uuid("Website ID from list_websites.")),
                ("startAt", date("Only annotations on or after this date.")),
                ("endAt", date("Only annotations on or before this date.")),
                ("search", string("Free-text note search.")),
                ("page", integer(1, None, "Page number.")),
                (
                    "pageSize",
                    integer(1, Some(MAX_PAGE_SIZE), "Results per page."),
                ),
            ]),
            &["websiteId"],
        ),
        schema(
            "list_segments",
            "List saved segments and cohorts",
            "List saved website segments and cohorts with their definitions.",
            props(&[
                ("websiteId", uuid("Website ID from list_websites.")),
                (
                    "type",
                    enumeration(&["segment", "cohort"], "Audience kind."),
                ),
                ("search", string("Filter by name.")),
            ]),
            &["websiteId"],
        ),
        schema(
            "list_funnels",
            "List saved funnels",
            "List saved funnel reports and their ordered steps.",
            props(&[
                ("websiteId", uuid("Website ID from list_websites.")),
                ("search", string("Filter by name or description.")),
                ("page", integer(1, None, "Page number.")),
                (
                    "pageSize",
                    integer(1, Some(MAX_PAGE_SIZE), "Results per page."),
                ),
            ]),
            &["websiteId"],
        ),
        run_funnel,
        schema(
            "get_goals",
            "Get saved goals",
            "List saved goals and optionally calculate their conversion results.",
            props(&[
                ("websiteId", uuid("Website ID from list_websites.")),
                ("goalId", uuid("Only return this goal.")),
                ("startAt", date("Start of the range for goal results.")),
                ("endAt", date("End of the range. Defaults to now.")),
                ("search", string("Filter by name or description.")),
                ("page", integer(1, None, "Page number.")),
                (
                    "pageSize",
                    integer(1, Some(MAX_PAGE_SIZE), "Results per page."),
                ),
                ("filters", filters(true)),
            ]),
            &["websiteId"],
        ),
        schema(
            "run_journey",
            "Run a journey report",
            "Return common visitor paths through pages and events.",
            props(&[
                ("websiteId", uuid("Website ID from list_websites.")),
                date_props()[0].clone(),
                date_props()[1].clone(),
                ("steps", integer(2, Some(7), "Journey length.")),
                ("startStep", string("Required starting page or event.")),
                ("endStep", string("Required ending page or event.")),
                ("filters", filters(true)),
            ]),
            &["websiteId", "startAt"],
        ),
        schema(
            "run_retention",
            "Run a retention report",
            "Return cohort retention by first-visit date and elapsed day.",
            props(&[
                ("websiteId", uuid("Website ID from list_websites.")),
                date_props()[0].clone(),
                date_props()[1].clone(),
                ("timezone", string("IANA timezone. Defaults to UTC.")),
                ("filters", filters(true)),
            ]),
            &["websiteId", "startAt"],
        ),
        schema(
            "run_attribution",
            "Run an attribution report",
            "Attribute a page or event conversion to acquisition sources.",
            props(&[
                ("websiteId", uuid("Website ID from list_websites.")),
                date_props()[0].clone(),
                date_props()[1].clone(),
                (
                    "model",
                    enumeration(
                        &["first-click", "last-click"],
                        "Attribution model (default last-click).",
                    ),
                ),
                (
                    "conversionType",
                    enumeration(&["path", "event"], "Conversion type."),
                ),
                (
                    "conversion",
                    nonempty("Conversion page path or event name."),
                ),
                ("filters", filters(true)),
            ]),
            &["websiteId", "startAt", "conversionType", "conversion"],
        ),
        schema(
            "get_revenue",
            "Get revenue report",
            "Return revenue totals, chart and acquisition/location breakdowns.",
            props(&[
                ("websiteId", uuid("Website ID from list_websites.")),
                date_props()[0].clone(),
                date_props()[1].clone(),
                (
                    "currency",
                    json!({"type":"string","minLength":3,"maxLength":3,"description":"ISO 4217 currency code."}),
                ),
                (
                    "unit",
                    enumeration(
                        &["minute", "hour", "day", "month", "year"],
                        "Time-series bucket size.",
                    ),
                ),
                ("timezone", string("IANA timezone. Defaults to UTC.")),
                ("filters", filters(true)),
            ]),
            &["websiteId", "startAt"],
        ),
        schema(
            "get_performance",
            "Get web vitals performance",
            "Return Core Web Vitals percentiles, optional series and optional breakdown.",
            props(&[
                ("websiteId", uuid("Website ID from list_websites.")),
                date_props()[0].clone(),
                date_props()[1].clone(),
                (
                    "metric",
                    enumeration(&["lcp", "inp", "cls", "fcp", "ttfb"], "Web vital."),
                ),
                (
                    "breakdown",
                    enumeration(
                        &["path", "title", "device", "browser"],
                        "Breakdown dimension.",
                    ),
                ),
                (
                    "limit",
                    integer(1, Some(MAX_METRIC_LIMIT), "Maximum breakdown rows."),
                ),
                (
                    "unit",
                    enumeration(
                        &["minute", "hour", "day", "month", "year"],
                        "Time-series bucket size.",
                    ),
                ),
                ("timezone", string("IANA timezone. Defaults to UTC.")),
                ("filters", filters(false)),
            ]),
            &["websiteId", "startAt"],
        ),
    ]
}

struct Input<'a> {
    args: &'a Map<String, Value>,
}

impl<'a> Input<'a> {
    fn new(args: &'a Map<String, Value>, allowed: &[&str]) -> Result<Self, Value> {
        if let Some(key) = args.keys().find(|key| !allowed.contains(&key.as_str())) {
            return Err(invalid(format!("Unknown argument: {key}")));
        }
        Ok(Self { args })
    }

    fn required_string(&self, name: &str, nonempty: bool) -> Result<&'a str, Value> {
        let value = self
            .args
            .get(name)
            .and_then(Value::as_str)
            .ok_or_else(|| invalid(format!("{name} must be a string")))?;
        if nonempty && value.is_empty() {
            return Err(invalid(format!("{name} must not be empty")));
        }
        Ok(value)
    }

    fn optional_string(&self, name: &str, nonempty: bool) -> Result<Option<&'a str>, Value> {
        let Some(value) = self.args.get(name) else {
            return Ok(None);
        };
        let value = value
            .as_str()
            .ok_or_else(|| invalid(format!("{name} must be a string")))?;
        if nonempty && value.is_empty() {
            return Err(invalid(format!("{name} must not be empty")));
        }
        Ok(Some(value))
    }

    fn uuid(&self, name: &str) -> Result<Uuid, Value> {
        Uuid::parse_str(self.required_string(name, true)?)
            .map_err(|_| invalid(format!("{name} must be a UUID")))
    }

    fn optional_uuid(&self, name: &str) -> Result<Option<Uuid>, Value> {
        self.optional_string(name, true)?
            .map(|value| {
                Uuid::parse_str(value).map_err(|_| invalid(format!("{name} must be a UUID")))
            })
            .transpose()
    }

    fn optional_bool(&self, name: &str) -> Result<Option<bool>, Value> {
        self.args
            .get(name)
            .map(|value| {
                value
                    .as_bool()
                    .ok_or_else(|| invalid(format!("{name} must be a boolean")))
            })
            .transpose()
    }

    fn integer(&self, name: &str, default: i64, min: i64, max: Option<i64>) -> Result<i64, Value> {
        let Some(value) = self.args.get(name) else {
            return Ok(default);
        };
        let value = value
            .as_i64()
            .ok_or_else(|| invalid(format!("{name} must be an integer")))?;
        if value < min || max.is_some_and(|max| value > max) {
            return Err(invalid(format!("{name} is out of range")));
        }
        Ok(value)
    }

    fn choice(&self, name: &str, values: &[&str]) -> Result<Option<&'a str>, Value> {
        let Some(value) = self.optional_string(name, false)? else {
            return Ok(None);
        };
        if !values.contains(&value) {
            return Err(invalid(format!("Invalid {name}")));
        }
        Ok(Some(value))
    }

    fn range(&self, required: bool) -> Result<Option<Range>, Value> {
        let Some(start) = self.optional_string("startAt", true)? else {
            return if required {
                Err(invalid("startAt is required"))
            } else {
                Ok(None)
            };
        };
        let start = parse_date(start, "startAt")?;
        let end = match self.optional_string("endAt", true)? {
            Some(end) => parse_date(end, "endAt")?,
            None => Utc::now(),
        };
        if end <= start {
            return Err(invalid("endAt must be after startAt"));
        }
        Ok(Some(Range { start, end }))
    }
}

#[derive(Clone, Copy)]
struct Range {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
}

impl Range {
    fn query(self, query: &mut Vec<(String, String)>) {
        query.push(("startAt".into(), self.start.timestamp_millis().to_string()));
        query.push(("endAt".into(), self.end.timestamp_millis().to_string()));
    }

    fn json(self) -> Value {
        json!({
            "startAt": iso(self.start),
            "endAt": iso(self.end)
        })
    }
}

fn parse_date(value: &str, name: &str) -> Result<DateTime<Utc>, Value> {
    DateTime::parse_from_rfc3339(value)
        .map(|date| date.to_utc())
        .or_else(|_| {
            NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .map_err(|_| ())
                .and_then(|date| date.and_hms_opt(0, 0, 0).ok_or(()))
                .map(|date| date.and_utc())
        })
        .map_err(|_| invalid(format!("{name} is not a valid ISO 8601 date")))
}

fn invalid(message: impl Into<String>) -> Value {
    json!({"code":"invalid_input","message":message.into()})
}

fn api_error(message: impl Into<String>) -> Value {
    json!({"code":"api_error","message":message.into()})
}

fn iso(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn to_iso(value: &Value) -> Value {
    if let Some(text) = value.as_str() {
        return DateTime::parse_from_rfc3339(text)
            .map(|date| json!(iso(date.to_utc())))
            .unwrap_or(Value::Null);
    }
    if let Some(milliseconds) = value.as_i64() {
        return Utc
            .timestamp_millis_opt(milliseconds)
            .single()
            .map(iso)
            .map(Value::String)
            .unwrap_or(Value::Null);
    }
    Value::Null
}

fn number(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Number(value)) => Value::Number(value.clone()),
        Some(Value::String(value)) => value
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .unwrap_or_else(|| json!(0)),
        _ => json!(0),
    }
}

fn object(value: &Value) -> Result<&Map<String, Value>, Value> {
    value
        .as_object()
        .ok_or_else(|| api_error("Invalid API response"))
}

fn array(value: &Value) -> Result<&Vec<Value>, Value> {
    value
        .as_array()
        .ok_or_else(|| api_error("Invalid API response"))
}

fn append(query: &mut Vec<(String, String)>, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        query.push((key.to_owned(), value.to_owned()));
    }
}

fn append_filters(
    input: &Input<'_>,
    query: &mut Vec<(String, String)>,
    include_event: bool,
) -> Result<(), Value> {
    let Some(value) = input.args.get("filters") else {
        return Ok(());
    };
    let filters = value
        .as_object()
        .ok_or_else(|| invalid("filters must be an object"))?;
    for (key, value) in filters {
        if !FILTERS.contains(&key.as_str()) || (!include_event && key == "event") {
            return Err(invalid(format!("Unknown filter: {key}")));
        }
        let value = value
            .as_str()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| invalid(format!("filters.{key} must be a non-empty string")))?;
        match key.as_str() {
            "segment" | "cohort" => {
                Uuid::parse_str(value)
                    .map_err(|_| invalid(format!("filters.{key} must be a UUID")))?;
            }
            "match" if !matches!(value, "all" | "any") => {
                return Err(invalid("filters.match must be all or any"));
            }
            _ => {}
        }
        query.push((key.clone(), value.to_owned()));
    }
    Ok(())
}

async fn get(
    api: &Router,
    headers: &HeaderMap,
    path: String,
    query: Vec<(String, String)>,
) -> Result<Value, Value> {
    let authorization = headers.get(header::AUTHORIZATION).cloned().ok_or_else(
        || json!({"code":"access_denied","status":401,"message":"Missing bearer token"}),
    )?;
    let uri = if query.is_empty() {
        path
    } else {
        let mut serializer = form_urlencoded::Serializer::new(String::new());
        serializer.extend_pairs(query);
        format!("{path}?{}", serializer.finish())
    };
    if uri.len() > MAX_URI_LENGTH {
        return Err(invalid("Tool arguments are too large"));
    }
    let mut request = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .header(header::AUTHORIZATION, authorization);
    for name in ["x-umami-share-token", "x-umami-share-context"] {
        if let Some(value) = headers.get(name) {
            request = request.header(name, value);
        }
    }
    let response = api
        .clone()
        .oneshot(
            request
                .body(Body::empty())
                .map_err(|_| api_error("Unable to build API request"))?,
        )
        .await
        .map_err(|_| api_error("API request failed"))?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), MAX_RESPONSE_BODY)
        .await
        .map_err(|_| api_error("API response was too large"))?;
    let body = serde_json::from_slice::<Value>(&bytes).ok();
    if !status.is_success() {
        let code = match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => "access_denied",
            StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => "invalid_input",
            StatusCode::NOT_FOUND => "not_found",
            _ => "api_error",
        };
        let message = body
            .as_ref()
            .and_then(|body| body.get("message").or_else(|| body.get("error")))
            .and_then(Value::as_str)
            .unwrap_or("API request failed");
        return Err(json!({"code":code,"status":status.as_u16(),"message":message}));
    }
    body.ok_or_else(|| api_error("Invalid API response"))
}

fn page_info(result: &Map<String, Value>) -> Result<Map<String, Value>, Value> {
    let page = result.get("page").and_then(Value::as_i64).unwrap_or(1);
    let page_size = result
        .get("pageSize")
        .and_then(Value::as_i64)
        .unwrap_or(DEFAULT_PAGE_SIZE);
    let count = result.get("count").and_then(Value::as_i64).unwrap_or(0);
    if page < 1 || page_size < 1 || count < 0 {
        return Err(api_error("Invalid paginated API response"));
    }
    Ok(props(&[
        ("page", json!(page)),
        ("pageSize", json!(page_size)),
        ("count", json!(count)),
        ("hasMore", json!(page.saturating_mul(page_size) < count)),
    ]))
}

fn format_session(row: &Map<String, Value>) -> Value {
    json!({
        "id":row.get("id").cloned().unwrap_or(Value::Null),
        "firstAt":to_iso(row.get("firstAt").unwrap_or(&Value::Null)),
        "lastAt":to_iso(row.get("lastAt").unwrap_or(&Value::Null)),
        "visits":number(row.get("visits")),
        "views":number(row.get("views")),
        "events":number(row.get("events")),
        "hostname":row.get("hostname").cloned().unwrap_or(Value::Null),
        "browser":row.get("browser").cloned().unwrap_or(Value::Null),
        "os":row.get("os").cloned().unwrap_or(Value::Null),
        "device":row.get("device").cloned().unwrap_or(Value::Null),
        "screen":row.get("screen").cloned().unwrap_or(Value::Null),
        "language":row.get("language").cloned().unwrap_or(Value::Null),
        "country":row.get("country").cloned().unwrap_or(Value::Null),
        "region":row.get("region").cloned().unwrap_or(Value::Null),
        "city":row.get("city").cloned().unwrap_or(Value::Null),
        "distinctId":row.get("distinctId").cloned().unwrap_or(Value::Null)
    })
}

pub(super) async fn invoke(
    api: &Router,
    headers: &HeaderMap,
    name: &str,
    args: &Map<String, Value>,
) -> Result<Value, Value> {
    match name {
        "get_website_daterange" => get_website_daterange(api, headers, args).await,
        "get_website_traffic" => get_website_traffic(api, headers, args).await,
        "get_website_metrics" => get_website_metrics(api, headers, args).await,
        "get_realtime" => get_realtime(api, headers, args).await,
        "get_events" => get_events(api, headers, args).await,
        "get_event_stats" => get_event_stats(api, headers, args).await,
        "get_event_series" => get_event_series(api, headers, args).await,
        "get_event_properties" => get_event_properties(api, headers, args).await,
        "get_sessions" => get_sessions(api, headers, args).await,
        "get_session_stats" => get_session_stats(api, headers, args).await,
        "get_session" => get_session(api, headers, args).await,
        "get_annotations" => get_annotations(api, headers, args).await,
        "list_segments" => list_segments(api, headers, args).await,
        "list_funnels" => list_funnels(api, headers, args).await,
        "run_funnel" => run_funnel(api, headers, args).await,
        "get_goals" => get_goals(api, headers, args).await,
        "run_journey" => run_journey(api, headers, args).await,
        "run_retention" => run_retention(api, headers, args).await,
        "run_attribution" => run_attribution(api, headers, args).await,
        "get_revenue" => get_revenue(api, headers, args).await,
        "get_performance" => get_performance(api, headers, args).await,
        _ => Err(json!({"code":"unknown_tool","message":format!("Unknown tool: {name}")})),
    }
}

fn common_range<'a>(
    args: &'a Map<String, Value>,
    allowed: &[&str],
) -> Result<(Input<'a>, Uuid, Range, Vec<(String, String)>), Value> {
    let input = Input::new(args, allowed)?;
    let website_id = input.uuid("websiteId")?;
    let range = input.range(true)?.expect("required range");
    let mut query = Vec::new();
    range.query(&mut query);
    Ok((input, website_id, range, query))
}

async fn get_website_daterange(
    api: &Router,
    headers: &HeaderMap,
    args: &Map<String, Value>,
) -> Result<Value, Value> {
    let input = Input::new(args, &["websiteId"])?;
    let website_id = input.uuid("websiteId")?;
    let result = get(
        api,
        headers,
        format!("/api/websites/{website_id}/daterange"),
        vec![],
    )
    .await?;
    let result = object(&result)?;
    let start = result.get("startDate").cloned().unwrap_or(Value::Null);
    let end = result.get("endDate").cloned().unwrap_or(Value::Null);
    Ok(json!({
        "websiteId":website_id,
        "startAt":to_iso(&start),
        "endAt":to_iso(&end),
        "hasData":!start.is_null() && !end.is_null()
    }))
}

fn series(value: Option<&Value>) -> Result<Vec<Value>, Value> {
    let Some(value) = value else {
        return Ok(vec![]);
    };
    let Some(rows) = value.as_array() else {
        return Ok(vec![]);
    };
    rows.iter()
        .map(|row| {
            let row = object(row)?;
            Ok(json!({
                "date":row.get("x").cloned().unwrap_or(Value::Null),
                "value":number(row.get("y"))
            }))
        })
        .collect()
}

async fn get_website_traffic(
    api: &Router,
    headers: &HeaderMap,
    args: &Map<String, Value>,
) -> Result<Value, Value> {
    let (input, website_id, range, mut query) = common_range(
        args,
        &[
            "websiteId",
            "startAt",
            "endAt",
            "unit",
            "timezone",
            "compare",
            "filters",
        ],
    )?;
    let unit = input.choice("unit", &["minute", "hour", "day", "month", "year"])?;
    let timezone = input.optional_string("timezone", false)?;
    let compare = input.choice("compare", &["prev", "yoy"])?;
    append(&mut query, "unit", unit);
    append(&mut query, "timezone", timezone);
    append(&mut query, "compare", compare);
    append_filters(&input, &mut query, false)?;
    let result = get(
        api,
        headers,
        format!("/api/websites/{website_id}/pageviews"),
        query,
    )
    .await?;
    let result = object(&result)?;
    let mut output = json!({
        "websiteId":website_id,
        "range":range.json(),
        "unit":unit.unwrap_or("auto"),
        "pageviews":series(result.get("pageviews"))?,
        "visits":series(result.get("sessions"))?
    });
    if let Some(previous) = result.get("compare") {
        let previous = object(previous)?;
        output["previous"] = json!({
            "startAt":previous.get("startDate").cloned().unwrap_or(Value::Null),
            "endAt":previous.get("endDate").cloned().unwrap_or(Value::Null),
            "pageviews":series(previous.get("pageviews"))?,
            "visits":series(previous.get("sessions"))?
        });
    }
    Ok(output)
}

async fn get_website_metrics(
    api: &Router,
    headers: &HeaderMap,
    args: &Map<String, Value>,
) -> Result<Value, Value> {
    let (input, website_id, range, mut query) = common_range(
        args,
        &[
            "websiteId",
            "startAt",
            "endAt",
            "type",
            "limit",
            "offset",
            "search",
            "timezone",
            "filters",
        ],
    )?;
    let metric = input
        .choice("type", METRIC_TYPES)?
        .ok_or_else(|| invalid("type is required"))?;
    if input
        .args
        .get("filters")
        .and_then(Value::as_object)
        .is_some_and(|filters| filters.contains_key("event"))
        && metric != "event"
    {
        return Err(invalid(
            "filters.event only applies when type is event; use the event tools instead",
        ));
    }
    let limit = input.integer("limit", DEFAULT_METRIC_LIMIT, 1, Some(MAX_METRIC_LIMIT))?;
    let offset = input.integer("offset", 0, 0, None)?;
    query.extend([
        ("type".into(), metric.into()),
        ("limit".into(), limit.to_string()),
        ("offset".into(), offset.to_string()),
    ]);
    append(
        &mut query,
        "search",
        input.optional_string("search", false)?,
    );
    append(
        &mut query,
        "timezone",
        input.optional_string("timezone", false)?,
    );
    append_filters(&input, &mut query, true)?;
    let result = get(
        api,
        headers,
        format!("/api/websites/{website_id}/metrics"),
        query,
    )
    .await?;
    let data = array(&result)?
        .iter()
        .map(|row| {
            let row = object(row)?;
            Ok(json!({
                "name":row.get("x").cloned().filter(|value| !value.is_null()).unwrap_or_else(|| json!("(none)")),
                "value":number(row.get("y"))
            }))
        })
        .collect::<Result<Vec<_>, Value>>()?;
    Ok(json!({
        "websiteId":website_id,
        "type":metric,
        "measure":if VISITOR_METRICS.contains(&metric) {"visitors"} else {"views"},
        "range":range.json(),
        "limit":limit,
        "offset":offset,
        "data":data
    }))
}

async fn get_realtime(
    api: &Router,
    headers: &HeaderMap,
    args: &Map<String, Value>,
) -> Result<Value, Value> {
    let input = Input::new(args, &["websiteId"])?;
    let website_id = input.uuid("websiteId")?;
    let result = get(
        api,
        headers,
        format!("/api/websites/{website_id}/active"),
        vec![],
    )
    .await?;
    let result = object(&result)?;
    Ok(json!({
        "websiteId":website_id,
        "activeVisitors":number(result.get("visitors")),
        "asOf":iso(Utc::now())
    }))
}

async fn get_events(
    api: &Router,
    headers: &HeaderMap,
    args: &Map<String, Value>,
) -> Result<Value, Value> {
    let (input, website_id, range, mut query) = common_range(
        args,
        &[
            "websiteId",
            "startAt",
            "endAt",
            "event",
            "search",
            "page",
            "pageSize",
            "filters",
        ],
    )?;
    let page = input.integer("page", 1, 1, None)?;
    let page_size = input.integer("pageSize", DEFAULT_PAGE_SIZE, 1, Some(MAX_PAGE_SIZE))?;
    query.extend([
        ("page".into(), page.to_string()),
        ("pageSize".into(), page_size.to_string()),
    ]);
    append(
        &mut query,
        "search",
        input.optional_string("search", false)?,
    );
    append_filters(&input, &mut query, true)?;
    if let Some(event) = input.optional_string("event", false)? {
        query.retain(|(key, _)| key != "event");
        query.push(("event".into(), event.into()));
    }
    let result = get(
        api,
        headers,
        format!("/api/websites/{website_id}/events"),
        query,
    )
    .await?;
    let result = object(&result)?;
    let rows = result
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| api_error("Invalid events response"))?;
    let events = rows
        .iter()
        .map(|row| {
            let row = object(row)?;
            let event_type = match row.get("eventType").and_then(Value::as_i64) {
                Some(2) => json!("event"),
                Some(1) => json!("pageview"),
                Some(value) => json!(value),
                None => Value::Null,
            };
            Ok(json!({
                "id":row.get("id").cloned().unwrap_or(Value::Null),
                "sessionId":row.get("sessionId").cloned().unwrap_or(Value::Null),
                "createdAt":to_iso(row.get("createdAt").unwrap_or(&Value::Null)),
                "type":event_type,
                "name":row.get("eventName").cloned().unwrap_or(Value::Null),
                "path":row.get("urlPath").cloned().unwrap_or(Value::Null),
                "title":row.get("pageTitle").cloned().unwrap_or(Value::Null),
                "hostname":row.get("hostname").cloned().unwrap_or(Value::Null),
                "referrer":row.get("referrerDomain").cloned().unwrap_or(Value::Null),
                "browser":row.get("browser").cloned().unwrap_or(Value::Null),
                "os":row.get("os").cloned().unwrap_or(Value::Null),
                "device":row.get("device").cloned().unwrap_or(Value::Null),
                "country":row.get("country").cloned().unwrap_or(Value::Null),
                "distinctId":row.get("distinctId").cloned().unwrap_or(Value::Null)
            }))
        })
        .collect::<Result<Vec<_>, Value>>()?;
    let mut output = props(&[
        ("websiteId", json!(website_id)),
        ("range", range.json()),
        ("events", json!(events)),
    ]);
    output.extend(page_info(result)?);
    Ok(Value::Object(output))
}

fn event_totals(value: Option<&Value>) -> Result<Value, Value> {
    let value = value
        .and_then(Value::as_object)
        .ok_or_else(|| api_error("Invalid event stats response"))?;
    Ok(json!({
        "events":number(value.get("events")),
        "visitors":number(value.get("visitors")),
        "visits":number(value.get("visits")),
        "uniqueEvents":number(value.get("uniqueEvents"))
    }))
}

async fn get_event_stats(
    api: &Router,
    headers: &HeaderMap,
    args: &Map<String, Value>,
) -> Result<Value, Value> {
    let (input, website_id, range, mut query) = common_range(
        args,
        &["websiteId", "startAt", "endAt", "compare", "filters"],
    )?;
    let compare = input.choice("compare", &["prev", "yoy"])?.unwrap_or("prev");
    query.push(("compare".into(), compare.into()));
    append_filters(&input, &mut query, true)?;
    let result = get(
        api,
        headers,
        format!("/api/websites/{website_id}/events/stats"),
        query,
    )
    .await?;
    let result = object(&result)?;
    let data = result
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| api_error("Invalid event stats response"))?;
    Ok(json!({
        "websiteId":website_id,
        "range":range.json(),
        "current":event_totals(Some(&Value::Object(data.clone())))?,
        "previous":event_totals(data.get("comparison"))?,
        "compare":compare
    }))
}

fn transform_event_series(rows: &[Value]) -> Result<Vec<Value>, Value> {
    let mut positions = HashMap::<String, usize>::new();
    let mut events = Vec::<(String, Vec<Value>, f64)>::new();
    for row in rows {
        let row = object(row)?;
        let name = row
            .get("x")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let point = json!({
            "date":row.get("t").cloned().unwrap_or(Value::Null),
            "value":number(row.get("y"))
        });
        let value = number(row.get("y")).as_f64().unwrap_or(0.0);
        let position = *positions.entry(name.clone()).or_insert_with(|| {
            events.push((name, Vec::new(), 0.0));
            events.len() - 1
        });
        events[position].1.push(point);
        events[position].2 += value;
    }
    Ok(events
        .into_iter()
        .map(|(name, series, total)| json!({"name":name,"total":total,"series":series}))
        .collect())
}

async fn get_event_series(
    api: &Router,
    headers: &HeaderMap,
    args: &Map<String, Value>,
) -> Result<Value, Value> {
    let (input, website_id, range, mut query) = common_range(
        args,
        &[
            "websiteId",
            "startAt",
            "endAt",
            "unit",
            "timezone",
            "limit",
            "filters",
        ],
    )?;
    let unit = input
        .choice("unit", &["minute", "hour", "day", "month", "year"])?
        .unwrap_or("day");
    let timezone = input.optional_string("timezone", false)?.unwrap_or("UTC");
    let limit = input.integer("limit", DEFAULT_METRIC_LIMIT, 1, Some(MAX_METRIC_LIMIT))?;
    query.extend([
        ("unit".into(), unit.into()),
        ("timezone".into(), timezone.into()),
        ("limit".into(), limit.to_string()),
    ]);
    append_filters(&input, &mut query, true)?;
    let result = get(
        api,
        headers,
        format!("/api/websites/{website_id}/events/series"),
        query,
    )
    .await?;
    Ok(json!({
        "websiteId":website_id,
        "range":range.json(),
        "unit":unit,
        "events":transform_event_series(array(&result)?)?
    }))
}

fn data_type(value: Option<&str>) -> Option<&'static str> {
    match value {
        Some("string") => Some("1"),
        Some("number") => Some("2"),
        Some("boolean") => Some("3"),
        Some("date") => Some("4"),
        Some("array") => Some("5"),
        _ => None,
    }
}

fn data_type_name(value: Option<i64>) -> Value {
    match value {
        Some(1) => json!("string"),
        Some(2) => json!("number"),
        Some(3) => json!("boolean"),
        Some(4) => json!("date"),
        Some(5) => json!("array"),
        Some(value) => json!(value.to_string()),
        None => json!("unknown"),
    }
}

async fn get_event_properties(
    api: &Router,
    headers: &HeaderMap,
    args: &Map<String, Value>,
) -> Result<Value, Value> {
    let (input, website_id, range, mut query) = common_range(
        args,
        &[
            "websiteId",
            "startAt",
            "endAt",
            "propertyName",
            "eventName",
            "dataType",
            "filters",
        ],
    )?;
    let property_name = input.optional_string("propertyName", true)?;
    let event_name = input.optional_string("eventName", true)?;
    let wanted = input.choice(
        "dataType",
        &["string", "number", "boolean", "date", "array"],
    )?;
    append_filters(&input, &mut query, true)?;
    if let Some(property_name) = property_name {
        query.push(("propertyName".into(), property_name.into()));
        append(&mut query, "eventName", event_name);
        append(&mut query, "dataType", data_type(wanted));
        let result = get(
            api,
            headers,
            format!("/api/websites/{website_id}/event-data/values"),
            query,
        )
        .await?;
        let values = array(&result)?
            .iter()
            .map(|row| {
                let row = object(row)?;
                Ok(json!({
                    "value":row.get("value").cloned().unwrap_or(Value::Null),
                    "count":number(row.get("total"))
                }))
            })
            .collect::<Result<Vec<_>, Value>>()?;
        return Ok(json!({
            "websiteId":website_id,
            "range":range.json(),
            "propertyName":property_name,
            "eventName":event_name,
            "values":values
        }));
    }
    if let Some(event_name) = event_name {
        query.push(("event".into(), event_name.into()));
    }
    let properties_result = get(
        api,
        headers,
        format!("/api/websites/{website_id}/event-data/properties"),
        query.clone(),
    )
    .await?;
    let stats_result = get(
        api,
        headers,
        format!("/api/websites/{website_id}/event-data/stats"),
        query,
    )
    .await?;
    let wanted_code = data_type(wanted).and_then(|value| value.parse::<i64>().ok());
    let properties = array(&properties_result)?
        .iter()
        .filter_map(|row| {
            let row = row.as_object()?;
            if wanted_code.is_some() && row.get("dataType").and_then(Value::as_i64) != wanted_code {
                return None;
            }
            Some(json!({
                "eventName":row.get("eventName").cloned().unwrap_or(Value::Null),
                "propertyName":row.get("propertyName").cloned().unwrap_or(Value::Null),
                "dataType":data_type_name(row.get("dataType").and_then(Value::as_i64)),
                "count":number(row.get("total"))
            }))
        })
        .collect::<Vec<_>>();
    let stats = object(&stats_result)?;
    Ok(json!({
        "websiteId":website_id,
        "range":range.json(),
        "eventName":event_name,
        "totals":{
            "events":number(stats.get("events")),
            "properties":number(stats.get("properties")),
            "records":number(stats.get("records"))
        },
        "properties":properties
    }))
}

async fn get_sessions(
    api: &Router,
    headers: &HeaderMap,
    args: &Map<String, Value>,
) -> Result<Value, Value> {
    let (input, website_id, range, mut query) = common_range(
        args,
        &[
            "websiteId",
            "startAt",
            "endAt",
            "search",
            "page",
            "pageSize",
            "filters",
        ],
    )?;
    let page = input.integer("page", 1, 1, None)?;
    let page_size = input.integer("pageSize", DEFAULT_PAGE_SIZE, 1, Some(MAX_PAGE_SIZE))?;
    query.extend([
        ("page".into(), page.to_string()),
        ("pageSize".into(), page_size.to_string()),
    ]);
    append(
        &mut query,
        "search",
        input.optional_string("search", false)?,
    );
    append_filters(&input, &mut query, true)?;
    let result = get(
        api,
        headers,
        format!("/api/websites/{website_id}/sessions"),
        query,
    )
    .await?;
    let result = object(&result)?;
    let sessions = result
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| api_error("Invalid sessions response"))?
        .iter()
        .map(|row| object(row).map(format_session))
        .collect::<Result<Vec<_>, Value>>()?;
    let mut output = props(&[
        ("websiteId", json!(website_id)),
        ("range", range.json()),
        ("sessions", json!(sessions)),
    ]);
    output.extend(page_info(result)?);
    Ok(Value::Object(output))
}

async fn get_session_stats(
    api: &Router,
    headers: &HeaderMap,
    args: &Map<String, Value>,
) -> Result<Value, Value> {
    let (input, website_id, range, mut query) =
        common_range(args, &["websiteId", "startAt", "endAt", "filters"])?;
    append_filters(&input, &mut query, true)?;
    let result = get(
        api,
        headers,
        format!("/api/websites/{website_id}/sessions/stats"),
        query,
    )
    .await?;
    let result = object(&result)?;
    let unwrap = |name: &str| {
        result
            .get(name)
            .and_then(|value| value.get("value").or(Some(value)))
            .map(|value| number(Some(value)))
            .unwrap_or_else(|| json!(0))
    };
    Ok(json!({
        "websiteId":website_id,
        "range":range.json(),
        "visitors":unwrap("visitors"),
        "visits":unwrap("visits"),
        "pageviews":unwrap("pageviews"),
        "events":unwrap("events"),
        "countries":unwrap("countries")
    }))
}

fn property_value(row: &Map<String, Value>) -> Value {
    for name in ["stringValue", "numberValue", "dateValue"] {
        if let Some(value) = row.get(name).filter(|value| !value.is_null()) {
            return value.clone();
        }
    }
    Value::Null
}

async fn get_session(
    api: &Router,
    headers: &HeaderMap,
    args: &Map<String, Value>,
) -> Result<Value, Value> {
    let input = Input::new(args, &["websiteId", "sessionId", "includeActivity"])?;
    let website_id = input.uuid("websiteId")?;
    let session_id = input.uuid("sessionId")?;
    let include_activity = input.optional_bool("includeActivity")?.unwrap_or(true);
    let session_result = get(
        api,
        headers,
        format!("/api/websites/{website_id}/sessions/{session_id}"),
        vec![],
    )
    .await?;
    let session = object(&session_result)?;
    let now = Utc::now();
    let first = session
        .get("firstAt")
        .and_then(Value::as_str)
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.to_utc())
        .unwrap_or(now - Duration::days(30));
    let last = session
        .get("lastAt")
        .and_then(Value::as_str)
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.to_utc())
        .unwrap_or(now);
    let properties_result = get(
        api,
        headers,
        format!("/api/websites/{website_id}/sessions/{session_id}/properties"),
        vec![],
    )
    .await?;
    let mut properties = Map::new();
    for row in array(&properties_result)? {
        let row = object(row)?;
        if let Some(key) = row.get("dataKey").and_then(Value::as_str) {
            properties.insert(key.into(), property_value(row));
        }
    }
    let rows = if include_activity {
        let mut query = Vec::new();
        Range {
            start: first - Duration::hours(1),
            end: last + Duration::hours(1),
        }
        .query(&mut query);
        let activity = get(
            api,
            headers,
            format!("/api/websites/{website_id}/sessions/{session_id}/activity"),
            query,
        )
        .await?;
        array(&activity)?.clone()
    } else {
        vec![]
    };
    let activity = rows
        .iter()
        .take(MAX_SESSION_ACTIVITY)
        .map(|row| {
            let row = object(row)?;
            Ok(json!({
                "at":to_iso(row.get("createdAt").unwrap_or(&Value::Null)),
                "type":if row.get("eventType").and_then(Value::as_i64) == Some(2) {"event"} else {"pageview"},
                "name":row.get("eventName").cloned().unwrap_or(Value::Null),
                "path":row.get("urlPath").cloned().unwrap_or(Value::Null),
                "query":row.get("urlQuery").cloned().unwrap_or(Value::Null),
                "referrer":row.get("referrerDomain").cloned().unwrap_or(Value::Null),
                "visitId":row.get("visitId").cloned().unwrap_or(Value::Null),
                "eventId":row.get("eventId").cloned().unwrap_or(Value::Null)
            }))
        })
        .collect::<Result<Vec<_>, Value>>()?;
    Ok(json!({
        "websiteId":website_id,
        "session":format_session(session),
        "properties":properties,
        "activity":activity,
        "activityTruncated":rows.len() > MAX_SESSION_ACTIVITY
    }))
}

async fn get_annotations(
    api: &Router,
    headers: &HeaderMap,
    args: &Map<String, Value>,
) -> Result<Value, Value> {
    let input = Input::new(
        args,
        &[
            "websiteId",
            "startAt",
            "endAt",
            "search",
            "page",
            "pageSize",
        ],
    )?;
    let website_id = input.uuid("websiteId")?;
    let range = input.range(false)?;
    if range.is_none() && input.args.contains_key("endAt") {
        return Err(invalid("endAt requires startAt"));
    }
    let page = input.integer("page", 1, 1, None)?;
    let page_size = input.integer("pageSize", DEFAULT_PAGE_SIZE, 1, Some(MAX_PAGE_SIZE))?;
    let mut query = vec![
        ("page".into(), page.to_string()),
        ("pageSize".into(), page_size.to_string()),
    ];
    if let Some(range) = range {
        range.query(&mut query);
    }
    append(
        &mut query,
        "search",
        input.optional_string("search", false)?,
    );
    let result = get(
        api,
        headers,
        format!("/api/websites/{website_id}/annotations"),
        query,
    )
    .await?;
    let result = object(&result)?;
    let annotations = result
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| api_error("Invalid annotations response"))?
        .iter()
        .map(|row| {
            let row = object(row)?;
            Ok(json!({
                "id":row.get("id").cloned().unwrap_or(Value::Null),
                "date":to_iso(row.get("date").unwrap_or(&Value::Null)),
                "allDay":row.get("allDay").and_then(Value::as_bool).unwrap_or(true),
                "note":row.get("note").and_then(Value::as_str).unwrap_or(""),
                "createdAt":to_iso(row.get("createdAt").unwrap_or(&Value::Null))
            }))
        })
        .collect::<Result<Vec<_>, Value>>()?;
    let mut output = props(&[
        ("websiteId", json!(website_id)),
        ("annotations", json!(annotations)),
    ]);
    if let Some(range) = range {
        output.insert("range".into(), range.json());
    }
    output.extend(page_info(result)?);
    Ok(Value::Object(output))
}

async fn list_segments(
    api: &Router,
    headers: &HeaderMap,
    args: &Map<String, Value>,
) -> Result<Value, Value> {
    let input = Input::new(args, &["websiteId", "type", "search"])?;
    let website_id = input.uuid("websiteId")?;
    let kind = input.choice("type", &["segment", "cohort"])?;
    let search = input.optional_string("search", false)?;
    let kinds = kind.map_or_else(|| vec!["segment", "cohort"], |kind| vec![kind]);
    let mut segments = Vec::new();
    for kind in kinds {
        let mut query = vec![("type".into(), kind.into())];
        append(&mut query, "search", search);
        let result = get(
            api,
            headers,
            format!("/api/websites/{website_id}/segments"),
            query,
        )
        .await?;
        let result = object(&result)?;
        let rows = result
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| api_error("Invalid segments response"))?;
        for row in rows {
            let row = object(row)?;
            segments.push(json!({
                "id":row.get("id").cloned().unwrap_or(Value::Null),
                "name":row.get("name").cloned().unwrap_or(Value::Null),
                "type":row.get("type").cloned().filter(|value| !value.is_null()).unwrap_or_else(|| json!(kind)),
                "definition":row.get("parameters").cloned().unwrap_or(Value::Null),
                "createdAt":to_iso(row.get("createdAt").unwrap_or(&Value::Null))
            }));
        }
    }
    Ok(json!({"websiteId":website_id,"segments":segments}))
}

async fn list_funnels(
    api: &Router,
    headers: &HeaderMap,
    args: &Map<String, Value>,
) -> Result<Value, Value> {
    let input = Input::new(args, &["websiteId", "search", "page", "pageSize"])?;
    let website_id = input.uuid("websiteId")?;
    let page = input.integer("page", 1, 1, None)?;
    let page_size = input.integer("pageSize", DEFAULT_PAGE_SIZE, 1, Some(MAX_PAGE_SIZE))?;
    let mut query = vec![
        ("page".into(), page.to_string()),
        ("pageSize".into(), page_size.to_string()),
    ];
    append(
        &mut query,
        "search",
        input.optional_string("search", false)?,
    );
    let result = get(
        api,
        headers,
        format!("/api/websites/{website_id}/funnels"),
        query,
    )
    .await?;
    let result = object(&result)?;
    let funnels = result
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| api_error("Invalid funnels response"))?
        .iter()
        .map(|row| {
            let row = object(row)?;
            let parameters = row.get("parameters").and_then(Value::as_object);
            let steps = parameters
                .and_then(|parameters| parameters.get("steps"))
                .and_then(Value::as_array)
                .map(|steps| {
                    steps
                        .iter()
                        .enumerate()
                        .map(|(index, step)| {
                            let step = step.as_object();
                            json!({
                                "step":index + 1,
                                "type":step.and_then(|step| step.get("type")).cloned().unwrap_or(Value::Null),
                                "value":step.and_then(|step| step.get("value")).cloned().unwrap_or(Value::Null)
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            Ok(json!({
                "id":row.get("id").cloned().unwrap_or(Value::Null),
                "name":row.get("name").cloned().unwrap_or(Value::Null),
                "description":row.get("description").cloned().filter(|value| value.as_str().is_some_and(|value| !value.is_empty())).unwrap_or(Value::Null),
                "windowMinutes":parameters.and_then(|parameters| parameters.get("window")).cloned().unwrap_or(Value::Null),
                "steps":steps,
                "createdAt":to_iso(row.get("createdAt").unwrap_or(&Value::Null))
            }))
        })
        .collect::<Result<Vec<_>, Value>>()?;
    let mut output = props(&[
        ("websiteId", json!(website_id)),
        ("funnels", json!(funnels)),
    ]);
    output.extend(page_info(result)?);
    Ok(Value::Object(output))
}

fn validate_steps(input: &Input<'_>) -> Result<Option<String>, Value> {
    let Some(steps) = input.args.get("steps") else {
        return Ok(None);
    };
    let steps = steps
        .as_array()
        .filter(|steps| (2..=8).contains(&steps.len()))
        .ok_or_else(|| invalid("steps must contain 2 to 8 items"))?;
    for step in steps {
        let step = step
            .as_object()
            .ok_or_else(|| invalid("Each step must be an object"))?;
        if step
            .keys()
            .any(|key| !matches!(key.as_str(), "type" | "value"))
        {
            return Err(invalid("Unknown funnel step argument"));
        }
        let kind = step
            .get("type")
            .and_then(Value::as_str)
            .filter(|kind| matches!(*kind, "path" | "event"))
            .ok_or_else(|| invalid("Invalid funnel step type"))?;
        let value = step
            .get("value")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| invalid("Funnel step value must not be empty"))?;
        let _ = (kind, value);
    }
    serde_json::to_string(steps)
        .map(Some)
        .map_err(|_| invalid("Invalid funnel steps"))
}

async fn run_funnel(
    api: &Router,
    headers: &HeaderMap,
    args: &Map<String, Value>,
) -> Result<Value, Value> {
    let (input, website_id, range, mut query) = common_range(
        args,
        &[
            "websiteId",
            "startAt",
            "endAt",
            "funnelId",
            "steps",
            "windowMinutes",
            "filters",
        ],
    )?;
    let funnel_id = input.optional_uuid("funnelId")?;
    let steps = validate_steps(&input)?;
    if funnel_id.is_some() == steps.is_some() {
        return Err(invalid("Provide either funnelId or steps, not both"));
    }
    append_filters(&input, &mut query, true)?;
    let window = input.integer("windowMinutes", 60, 1, Some(43_200))?;
    let path = if let Some(funnel_id) = funnel_id {
        format!("/api/websites/{website_id}/funnels/{funnel_id}/stats")
    } else {
        query.push(("window".into(), window.to_string()));
        query.push(("steps".into(), steps.expect("validated steps")));
        format!("/api/websites/{website_id}/funnels/stats")
    };
    let result = get(api, headers, path, query).await?;
    let steps = array(&result)?
        .iter()
        .enumerate()
        .map(|(index, step)| {
            let step = object(step)?;
            Ok(json!({
                "step":index + 1,
                "type":step.get("type").cloned().unwrap_or(Value::Null),
                "value":step.get("value").cloned().unwrap_or(Value::Null),
                "visitors":number(step.get("visitors")),
                "previous":number(step.get("previous")),
                "dropped":number(step.get("dropped")),
                "dropoffRate":number(step.get("dropoff")),
                "remainingRate":number(step.get("remaining"))
            }))
        })
        .collect::<Result<Vec<_>, Value>>()?;
    let mut output = props(&[
        ("websiteId", json!(website_id)),
        ("range", range.json()),
        ("steps", json!(steps)),
    ]);
    if let Some(funnel_id) = funnel_id {
        output.insert("funnelId".into(), json!(funnel_id));
    } else {
        output.insert("windowMinutes".into(), json!(window));
    }
    Ok(Value::Object(output))
}

async fn get_goals(
    api: &Router,
    headers: &HeaderMap,
    args: &Map<String, Value>,
) -> Result<Value, Value> {
    let input = Input::new(
        args,
        &[
            "websiteId",
            "goalId",
            "startAt",
            "endAt",
            "search",
            "page",
            "pageSize",
            "filters",
        ],
    )?;
    let website_id = input.uuid("websiteId")?;
    let goal_id = input.optional_uuid("goalId")?;
    let range = input.range(false)?;
    if range.is_none() && input.args.contains_key("endAt") {
        return Err(invalid("endAt requires startAt"));
    }
    let page = input.integer("page", 1, 1, None)?;
    let page_size = input.integer("pageSize", DEFAULT_PAGE_SIZE, 1, Some(MAX_PAGE_SIZE))?;
    let mut query = vec![
        ("page".into(), page.to_string()),
        ("pageSize".into(), page_size.to_string()),
    ];
    append(
        &mut query,
        "search",
        input.optional_string("search", false)?,
    );
    let result = get(
        api,
        headers,
        format!("/api/websites/{website_id}/goals"),
        query,
    )
    .await?;
    let result = object(&result)?;
    let rows = result
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| api_error("Invalid goals response"))?;
    let mut goals = Vec::new();
    for row in rows {
        let row = object(row)?;
        let row_id = row
            .get("id")
            .and_then(Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok());
        if goal_id.is_some() && row_id != goal_id {
            continue;
        }
        let parameters = row.get("parameters").and_then(Value::as_object);
        let mut goal = props(&[
            ("id", row.get("id").cloned().unwrap_or(Value::Null)),
            ("name", row.get("name").cloned().unwrap_or(Value::Null)),
            (
                "description",
                row.get("description")
                    .cloned()
                    .filter(|value| value.as_str().is_some_and(|value| !value.is_empty()))
                    .unwrap_or(Value::Null),
            ),
            (
                "type",
                parameters
                    .and_then(|parameters| parameters.get("type"))
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
            (
                "value",
                parameters
                    .and_then(|parameters| parameters.get("value"))
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
            (
                "createdAt",
                to_iso(row.get("createdAt").unwrap_or(&Value::Null)),
            ),
        ]);
        if let (Some(range), Some(row_id)) = (range, row_id) {
            let mut stats_query = Vec::new();
            range.query(&mut stats_query);
            append_filters(&input, &mut stats_query, true)?;
            let stats = get(
                api,
                headers,
                format!("/api/websites/{website_id}/goals/{row_id}/stats"),
                stats_query,
            )
            .await?;
            let stats = object(&stats)?;
            let conversions = number(stats.get("num")).as_f64().unwrap_or(0.0);
            let visitors = number(stats.get("total")).as_f64().unwrap_or(0.0);
            goal.insert(
                "results".into(),
                json!({
                    "conversions":conversions,
                    "visitors":visitors,
                    "conversionRate":if visitors == 0.0 {0.0} else {(conversions / visitors * 1000.0).round() / 10.0}
                }),
            );
        }
        goals.push(Value::Object(goal));
    }
    let mut output = props(&[("websiteId", json!(website_id)), ("goals", json!(goals))]);
    if let Some(range) = range {
        output.insert("range".into(), range.json());
    }
    output.extend(page_info(result)?);
    Ok(Value::Object(output))
}

async fn run_journey(
    api: &Router,
    headers: &HeaderMap,
    args: &Map<String, Value>,
) -> Result<Value, Value> {
    let (input, website_id, range, mut query) = common_range(
        args,
        &[
            "websiteId",
            "startAt",
            "endAt",
            "steps",
            "startStep",
            "endStep",
            "filters",
        ],
    )?;
    let steps = input.integer("steps", 3, 2, Some(7))?;
    query.push(("steps".into(), steps.to_string()));
    append(
        &mut query,
        "startStep",
        input.optional_string("startStep", false)?,
    );
    append(
        &mut query,
        "endStep",
        input.optional_string("endStep", false)?,
    );
    append_filters(&input, &mut query, true)?;
    let result = get(
        api,
        headers,
        format!("/api/websites/{website_id}/journeys"),
        query,
    )
    .await?;
    let journeys = array(&result)?
        .iter()
        .map(|row| {
            let row = object(row)?;
            Ok(json!({
                "steps":row.get("items").cloned().unwrap_or_else(|| json!([])),
                "visitors":number(row.get("count").or_else(|| row.get("visitors")))
            }))
        })
        .collect::<Result<Vec<_>, Value>>()?;
    Ok(json!({"websiteId":website_id,"range":range.json(),"journeys":journeys}))
}

async fn run_retention(
    api: &Router,
    headers: &HeaderMap,
    args: &Map<String, Value>,
) -> Result<Value, Value> {
    let (input, website_id, range, mut query) = common_range(
        args,
        &["websiteId", "startAt", "endAt", "timezone", "filters"],
    )?;
    append(
        &mut query,
        "timezone",
        input.optional_string("timezone", false)?,
    );
    append_filters(&input, &mut query, true)?;
    let result = get(
        api,
        headers,
        format!("/api/websites/{website_id}/retention"),
        query,
    )
    .await?;
    let cohorts = array(&result)?
        .iter()
        .map(|row| {
            let row = object(row)?;
            Ok(json!({
                "cohortDate":row.get("date").cloned().unwrap_or(Value::Null),
                "day":number(row.get("day")),
                "visitors":number(row.get("visitors")),
                "returningVisitors":number(row.get("returnVisitors")),
                "retentionRate":number(row.get("percentage"))
            }))
        })
        .collect::<Result<Vec<_>, Value>>()?;
    Ok(json!({"websiteId":website_id,"range":range.json(),"cohorts":cohorts}))
}

fn attribution_list(value: Option<&Value>) -> Result<Vec<Value>, Value> {
    let Some(value) = value else {
        return Ok(vec![]);
    };
    array(value)?
        .iter()
        .map(|row| {
            let row = object(row)?;
            Ok(json!({
                "name":row.get("name").cloned().filter(|value| !value.is_null()).unwrap_or_else(|| json!("(none)")),
                "value":number(row.get("value"))
            }))
        })
        .collect()
}

async fn run_attribution(
    api: &Router,
    headers: &HeaderMap,
    args: &Map<String, Value>,
) -> Result<Value, Value> {
    let (input, website_id, range, mut query) = common_range(
        args,
        &[
            "websiteId",
            "startAt",
            "endAt",
            "model",
            "conversionType",
            "conversion",
            "filters",
        ],
    )?;
    let model = input
        .choice("model", &["first-click", "last-click"])?
        .unwrap_or("last-click");
    let conversion_type = input
        .choice("conversionType", &["path", "event"])?
        .ok_or_else(|| invalid("conversionType is required"))?;
    let conversion = input.required_string("conversion", true)?;
    query.extend([
        ("model".into(), model.into()),
        ("type".into(), conversion_type.into()),
        ("step".into(), conversion.into()),
    ]);
    append_filters(&input, &mut query, true)?;
    let result = get(
        api,
        headers,
        format!("/api/websites/{website_id}/attribution"),
        query,
    )
    .await?;
    let result = object(&result)?;
    Ok(json!({
        "websiteId":website_id,
        "range":range.json(),
        "model":model,
        "conversion":{"type":conversion_type,"value":conversion},
        "total":result.get("total").cloned().unwrap_or(Value::Null),
        "referrers":attribution_list(result.get("referrer"))?,
        "paidAds":attribution_list(result.get("paidAds"))?,
        "utmSources":attribution_list(result.get("utm_source"))?,
        "utmMediums":attribution_list(result.get("utm_medium"))?,
        "utmCampaigns":attribution_list(result.get("utm_campaign"))?,
        "utmContent":attribution_list(result.get("utm_content"))?,
        "utmTerms":attribution_list(result.get("utm_term"))?
    }))
}

async fn get_revenue(
    api: &Router,
    headers: &HeaderMap,
    args: &Map<String, Value>,
) -> Result<Value, Value> {
    let (input, website_id, range, mut query) = common_range(
        args,
        &[
            "websiteId",
            "startAt",
            "endAt",
            "currency",
            "unit",
            "timezone",
            "filters",
        ],
    )?;
    let currency = input
        .optional_string("currency", false)?
        .unwrap_or("USD")
        .to_uppercase();
    if currency.chars().count() != 3 {
        return Err(invalid("currency must contain exactly 3 characters"));
    }
    let unit = input.choice("unit", &["minute", "hour", "day", "month", "year"])?;
    let timezone = input.optional_string("timezone", false)?;
    query.push(("currency".into(), currency.clone()));
    append(&mut query, "unit", unit);
    append(&mut query, "timezone", timezone);
    append_filters(&input, &mut query, true)?;
    let total = get(
        api,
        headers,
        format!("/api/websites/{website_id}/revenue/stats"),
        query.clone(),
    )
    .await?;
    let chart = get(
        api,
        headers,
        format!("/api/websites/{website_id}/revenue/chart"),
        query.clone(),
    )
    .await?;
    let metric = |kind: &str| {
        let mut query = query.clone();
        query.push(("type".into(), kind.into()));
        query
    };
    let country = get(
        api,
        headers,
        format!("/api/websites/{website_id}/revenue/metrics"),
        metric("country"),
    )
    .await?;
    let region = get(
        api,
        headers,
        format!("/api/websites/{website_id}/revenue/metrics"),
        metric("region"),
    )
    .await?;
    let referrer = get(
        api,
        headers,
        format!("/api/websites/{website_id}/revenue/metrics"),
        metric("referrer"),
    )
    .await?;
    let channel = get(
        api,
        headers,
        format!("/api/websites/{website_id}/revenue/metrics"),
        metric("channel"),
    )
    .await?;
    let chart_rows = chart.get("chart").cloned().unwrap_or_else(|| json!([]));
    if !chart_rows.is_array() {
        return Err(api_error("Invalid revenue chart response"));
    }
    for value in [&country, &region, &referrer, &channel] {
        array(value)?;
    }
    Ok(json!({
        "websiteId":website_id,
        "range":range.json(),
        "currency":currency,
        "total":total,
        "chart":chart_rows,
        "byCountry":country,
        "byRegion":region,
        "byReferrer":referrer,
        "byChannel":channel
    }))
}

fn percentiles(value: Option<&Value>) -> Result<Value, Value> {
    let value = value
        .and_then(Value::as_object)
        .ok_or_else(|| api_error("Invalid performance response"))?;
    Ok(json!({
        "p50":number(value.get("p50")),
        "p75":number(value.get("p75")),
        "p95":number(value.get("p95"))
    }))
}

async fn get_performance(
    api: &Router,
    headers: &HeaderMap,
    args: &Map<String, Value>,
) -> Result<Value, Value> {
    let (input, website_id, range, mut query) = common_range(
        args,
        &[
            "websiteId",
            "startAt",
            "endAt",
            "metric",
            "breakdown",
            "limit",
            "unit",
            "timezone",
            "filters",
        ],
    )?;
    let requested_metric = input.choice("metric", &["lcp", "inp", "cls", "fcp", "ttfb"])?;
    let breakdown = input.choice("breakdown", &["path", "title", "device", "browser"])?;
    let metric = requested_metric.or(if breakdown.is_some() {
        Some("lcp")
    } else {
        None
    });
    let unit = input
        .choice("unit", &["minute", "hour", "day", "month", "year"])?
        .unwrap_or("day");
    let timezone = input.optional_string("timezone", false)?.unwrap_or("UTC");
    let limit = input.integer("limit", DEFAULT_METRIC_LIMIT, 1, Some(MAX_METRIC_LIMIT))?;
    query.extend([
        ("unit".into(), unit.into()),
        ("timezone".into(), timezone.into()),
    ]);
    append_filters(&input, &mut query, false)?;
    let summary = get(
        api,
        headers,
        format!("/api/websites/{website_id}/performance/stats"),
        query.clone(),
    )
    .await?;
    let summary = object(&summary)?;
    let mut vitals = Map::new();
    for name in ["lcp", "inp", "cls", "fcp", "ttfb"] {
        vitals.insert(name.into(), percentiles(summary.get(name))?);
    }
    let mut output = props(&[
        ("websiteId", json!(website_id)),
        ("range", range.json()),
        ("measurements", number(summary.get("count"))),
        ("vitals", Value::Object(vitals)),
    ]);
    if let Some(metric) = metric {
        let mut chart_query = query.clone();
        chart_query.push(("metric".into(), metric.into()));
        let chart = get(
            api,
            headers,
            format!("/api/websites/{website_id}/performance/chart"),
            chart_query,
        )
        .await?;
        let chart = object(&chart)?;
        let points = chart
            .get("chart")
            .and_then(Value::as_array)
            .ok_or_else(|| api_error("Invalid performance chart response"))?
            .iter()
            .map(|row| {
                let row = object(row)?;
                let mut point = props(&[("date", row.get("t").cloned().unwrap_or(Value::Null))]);
                point.extend(object(&percentiles(Some(&Value::Object(row.clone())))?)?.clone());
                Ok(Value::Object(point))
            })
            .collect::<Result<Vec<_>, Value>>()?;
        output.insert(
            "series".into(),
            json!({"metric":metric,"unit":unit,"points":points}),
        );
    }
    if let Some(breakdown) = breakdown {
        let metric = metric.expect("breakdown selects metric");
        let mut metrics_query = query;
        metrics_query.extend([
            ("metric".into(), metric.into()),
            ("type".into(), breakdown.into()),
            ("limit".into(), limit.to_string()),
        ]);
        let rows = get(
            api,
            headers,
            format!("/api/websites/{website_id}/performance/metrics"),
            metrics_query,
        )
        .await?;
        let rows = array(&rows)?
            .iter()
            .map(|row| {
                let row = object(row)?;
                let mut result = props(&[
                    (
                        "name",
                        row.get("name").cloned().unwrap_or_else(|| json!("")),
                    ),
                    ("measurements", number(row.get("count"))),
                ]);
                result.extend(object(&percentiles(Some(&Value::Object(row.clone())))?)?.clone());
                Ok(Value::Object(result))
            })
            .collect::<Result<Vec<_>, Value>>()?;
        output.insert(
            "breakdown".into(),
            json!({"metric":metric,"type":breakdown,"rows":rows}),
        );
    }
    Ok(Value::Object(output))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn catalog_contains_only_additional_read_only_tools() {
        let catalog = catalog();
        let names = catalog
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect::<HashSet<_>>();
        assert_eq!(catalog.len(), 21);
        assert_eq!(names.len(), catalog.len());
        assert!(!names.contains("list_websites"));
        assert!(!names.contains("get_website_stats"));
        assert!(catalog.iter().all(|tool| {
            tool["annotations"]["readOnlyHint"] == true
                && tool["annotations"]["destructiveHint"] == false
                && tool["inputSchema"]["additionalProperties"] == false
        }));
    }

    #[test]
    fn event_series_groups_and_totals_rows() {
        let rows = vec![
            json!({"x":"signup","t":"2026-01-01","y":2}),
            json!({"x":"purchase","t":"2026-01-01","y":1}),
            json!({"x":"signup","t":"2026-01-02","y":3}),
        ];
        let result = transform_event_series(&rows).unwrap();
        assert_eq!(result[0]["name"], "signup");
        assert_eq!(result[0]["total"], 5.0);
        assert_eq!(result[0]["series"].as_array().unwrap().len(), 2);
        assert_eq!(result[1]["name"], "purchase");
    }

    #[test]
    fn date_ranges_require_increasing_iso_dates() {
        let args = Map::from_iter([
            ("startAt".into(), json!("2026-11-19")),
            ("endAt".into(), json!("2026-11-18")),
        ]);
        assert!(Input::new(&args, &["startAt", "endAt"])
            .unwrap()
            .range(true)
            .is_err());
    }

    #[test]
    fn stats_filters_reuse_pageview_validation() {
        let args = Map::from_iter([("filters".into(), json!({"path":"/pricing","country":"US"}))]);
        assert_eq!(
            stats_filters(&args).unwrap(),
            vec![
                ("country".into(), "US".into()),
                ("path".into(), "/pricing".into())
            ]
        );
        let args = Map::from_iter([("filters".into(), json!({"event":"signup"}))]);
        assert!(stats_filters(&args).is_err());
        assert!(stats_filters_schema()["properties"].get("event").is_none());
    }
}
