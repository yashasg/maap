use crate::{
    access::{website_section_pool, ShareDataVisibility, ShareSection},
    error_response, json_response, AppState,
};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::get,
    Router,
};
use chrono::{DateTime, TimeZone, Utc};
use regex::Regex;
use rust_decimal::Decimal;
use serde_json::{json, Value};
use sqlx::{PgPool, Postgres, QueryBuilder, Row};
use std::{collections::HashMap, str::FromStr};
use uuid::Uuid;

const FILTER_NAMES: &[&str] = &[
    "path",
    "referrer",
    "title",
    "query",
    "os",
    "browser",
    "device",
    "country",
    "region",
    "city",
    "tag",
    "hostname",
    "distinctId",
    "language",
    "event",
    "utmSource",
    "utmMedium",
    "utmCampaign",
    "utmContent",
    "utmTerm",
    "eventType",
];

#[derive(Clone, Copy)]
struct DateRange {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Source {
    Event,
    Session,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Operation {
    Pivot,
    ArraySeries,
    DateSeries,
    NumericSeries,
    NumericStats,
    PropertySeries,
    ActivityStats,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Operator {
    Eq,
    Neq,
    Starts,
    NotStarts,
    Contains,
    NotContains,
    Regex,
    NotRegex,
    Greater,
    Less,
    GreaterEqual,
    LessEqual,
    Before,
    After,
}

#[derive(Clone)]
struct Filter {
    name: &'static str,
    column: &'static str,
    operator: Operator,
    values: Vec<String>,
}

struct Filters {
    values: Vec<Filter>,
    match_any: bool,
    needs_session: bool,
}

#[derive(Clone)]
struct PropertyFilter {
    property_name: String,
    data_type: i32,
    operator: Operator,
    value: String,
}

struct Context {
    pool: PgPool,
    range: DateRange,
    timezone: String,
    unit: &'static str,
    filters: Filters,
    audience: crate::audience::Audience,
    property_filters: Vec<PropertyFilter>,
}

#[derive(Debug)]
struct QueryIssue {
    status: StatusCode,
    message: String,
}

impl QueryIssue {
    fn bad(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn response(self) -> Response {
        error_response(self.status, self.message)
    }
}

pub(super) fn register_routes(router: Router<AppState>) -> Router<AppState> {
    router
        .route(
            "/api/websites/{website_id}/event-data-pivot",
            get(event_pivot),
        )
        .route(
            "/api/websites/{website_id}/event-data-pivot/array-series",
            get(event_array_series),
        )
        .route(
            "/api/websites/{website_id}/event-data-pivot/date-series",
            get(event_date_series),
        )
        .route(
            "/api/websites/{website_id}/event-data-pivot/numeric-series",
            get(event_numeric_series),
        )
        .route(
            "/api/websites/{website_id}/event-data-pivot/numeric-stats",
            get(event_numeric_stats),
        )
        .route(
            "/api/websites/{website_id}/event-data-pivot/property-series",
            get(event_property_series),
        )
        .route(
            "/api/websites/{website_id}/session-data-pivot",
            get(session_pivot),
        )
        .route(
            "/api/websites/{website_id}/session-data/array-series",
            get(session_array_series),
        )
        .route(
            "/api/websites/{website_id}/session-data/date-series",
            get(session_date_series),
        )
        .route(
            "/api/websites/{website_id}/session-data/numeric-series",
            get(session_numeric_series),
        )
        .route(
            "/api/websites/{website_id}/session-data/numeric-stats",
            get(session_numeric_stats),
        )
        .route(
            "/api/websites/{website_id}/session-data/property-series",
            get(session_property_series),
        )
        .route(
            "/api/websites/{website_id}/session-data/stats",
            get(session_activity_stats),
        )
}

macro_rules! handler {
    ($name:ident, $source:expr, $operation:expr) => {
        async fn $name(
            State(state): State<AppState>,
            Path(website_id): Path<Uuid>,
            Query(params): Query<HashMap<String, String>>,
            headers: HeaderMap,
        ) -> Response {
            handle(&state, &headers, website_id, params, $source, $operation).await
        }
    };
}

handler!(event_pivot, Source::Event, Operation::Pivot);
handler!(event_array_series, Source::Event, Operation::ArraySeries);
handler!(event_date_series, Source::Event, Operation::DateSeries);
handler!(
    event_numeric_series,
    Source::Event,
    Operation::NumericSeries
);
handler!(event_numeric_stats, Source::Event, Operation::NumericStats);
handler!(
    event_property_series,
    Source::Event,
    Operation::PropertySeries
);
handler!(session_pivot, Source::Session, Operation::Pivot);
handler!(
    session_array_series,
    Source::Session,
    Operation::ArraySeries
);
handler!(session_date_series, Source::Session, Operation::DateSeries);
handler!(
    session_numeric_series,
    Source::Session,
    Operation::NumericSeries
);
handler!(
    session_numeric_stats,
    Source::Session,
    Operation::NumericStats
);
handler!(
    session_property_series,
    Source::Session,
    Operation::PropertySeries
);
handler!(
    session_activity_stats,
    Source::Session,
    Operation::ActivityStats
);

async fn handle(
    state: &AppState,
    headers: &HeaderMap,
    website_id: Uuid,
    mut params: HashMap<String, String>,
    source: Source,
    operation: Operation,
) -> Response {
    let context = match context(state, headers, website_id, &mut params, source, operation).await {
        Ok(context) => context,
        Err(response) => return response,
    };
    let event_name = params.get("eventName").cloned();
    let property_name = params.get("propertyName").cloned();
    let metric = match params.get("metric").map(String::as_str).unwrap_or("sum") {
        "sum" => "sum",
        "avg" => "avg",
        "count" => "count",
        _ => return QueryIssue::bad("Invalid metric").response(),
    };
    let (page, page_size, offset, max_results) = match pagination(&params) {
        Ok(value) => value,
        Err(issue) => return issue.response(),
    };
    let result = match (source, operation) {
        (Source::Event, Operation::Pivot) => {
            event_pivot_query(
                &context,
                website_id,
                event_name.as_deref().expect("validated eventName"),
                page,
                page_size,
                offset,
                max_results,
            )
            .await
        }
        (Source::Event, Operation::ArraySeries) => {
            event_array_query(
                &context,
                website_id,
                event_name.as_deref().expect("validated eventName"),
                property_name.as_deref().expect("validated propertyName"),
            )
            .await
        }
        (Source::Event, Operation::DateSeries) => {
            event_date_query(
                &context,
                website_id,
                event_name.as_deref().expect("validated eventName"),
                property_name.as_deref().expect("validated propertyName"),
            )
            .await
        }
        (Source::Event, Operation::NumericSeries) => {
            event_numeric_series_query(
                &context,
                website_id,
                event_name.as_deref().expect("validated eventName"),
                property_name.as_deref().expect("validated propertyName"),
                metric,
            )
            .await
        }
        (Source::Event, Operation::NumericStats) => {
            event_numeric_stats_query(
                &context,
                website_id,
                event_name.as_deref().expect("validated eventName"),
                property_name.as_deref().expect("validated propertyName"),
            )
            .await
        }
        (Source::Event, Operation::PropertySeries) => {
            event_property_query(
                &context,
                website_id,
                event_name.as_deref().expect("validated eventName"),
                property_name.as_deref().expect("validated propertyName"),
            )
            .await
        }
        (Source::Session, Operation::Pivot) => {
            session_pivot_query(
                &context,
                website_id,
                property_name.as_deref().expect("validated propertyName"),
                page,
                page_size,
                offset,
                max_results,
            )
            .await
        }
        (Source::Session, Operation::ArraySeries) => {
            session_array_query(
                &context,
                website_id,
                property_name.as_deref().expect("validated propertyName"),
            )
            .await
        }
        (Source::Session, Operation::DateSeries) => {
            session_date_query(
                &context,
                website_id,
                property_name.as_deref().expect("validated propertyName"),
            )
            .await
        }
        (Source::Session, Operation::NumericSeries) => {
            session_numeric_series_query(
                &context,
                website_id,
                property_name.as_deref().expect("validated propertyName"),
                metric,
            )
            .await
        }
        (Source::Session, Operation::NumericStats) => {
            session_numeric_stats_query(
                &context,
                website_id,
                property_name.as_deref().expect("validated propertyName"),
            )
            .await
        }
        (Source::Session, Operation::PropertySeries) => {
            session_property_query(
                &context,
                website_id,
                property_name.as_deref().expect("validated propertyName"),
            )
            .await
        }
        (Source::Session, Operation::ActivityStats) => {
            session_activity_query(
                &context,
                website_id,
                property_name.as_deref().expect("validated propertyName"),
            )
            .await
        }
        _ => unreachable!(),
    };

    match result {
        Ok(value) => json_response(value),
        Err(error) => database_error(error),
    }
}

fn controls(source: Source, operation: Operation) -> &'static [&'static str] {
    match (source, operation) {
        (Source::Event, Operation::Pivot) => &[
            "startAt",
            "endAt",
            "period",
            "eventName",
            "timezone",
            "unit",
            "page",
            "pageSize",
            "maxResults",
        ],
        (Source::Session, Operation::Pivot) => &[
            "startAt",
            "endAt",
            "period",
            "propertyName",
            "timezone",
            "unit",
            "page",
            "pageSize",
            "maxResults",
        ],
        (Source::Event, Operation::NumericSeries) => &[
            "startAt",
            "endAt",
            "period",
            "eventName",
            "propertyName",
            "metric",
            "timezone",
            "unit",
        ],
        (Source::Session, Operation::NumericSeries) => &[
            "startAt",
            "endAt",
            "period",
            "propertyName",
            "metric",
            "timezone",
            "unit",
        ],
        (Source::Event, Operation::NumericStats) => {
            &["startAt", "endAt", "period", "eventName", "propertyName"]
        }
        (Source::Session, Operation::NumericStats) => {
            &["startAt", "endAt", "period", "propertyName", "timezone"]
        }
        (Source::Event, Operation::DateSeries) => &[
            "startAt",
            "endAt",
            "period",
            "eventName",
            "propertyName",
            "timezone",
        ],
        (Source::Session, Operation::DateSeries) => {
            &["startAt", "endAt", "period", "propertyName", "timezone"]
        }
        (Source::Event, Operation::ArraySeries | Operation::PropertySeries) => &[
            "startAt",
            "endAt",
            "period",
            "eventName",
            "propertyName",
            "timezone",
            "unit",
        ],
        (
            Source::Session,
            Operation::ArraySeries | Operation::PropertySeries | Operation::ActivityStats,
        ) => &[
            "startAt",
            "endAt",
            "period",
            "propertyName",
            "timezone",
            "unit",
        ],
        (Source::Event, Operation::ActivityStats) => &[],
    }
}

fn validate_params(
    params: &HashMap<String, String>,
    source: Source,
    operation: Operation,
) -> Result<(), QueryIssue> {
    if source == Source::Event && params.get("eventName").is_none_or(|value| value.is_empty()) {
        return Err(QueryIssue::bad("eventName is required"));
    }
    if operation != Operation::Pivot || source == Source::Session {
        if params
            .get("propertyName")
            .is_none_or(|value| value.is_empty())
        {
            return Err(QueryIssue::bad("propertyName is required"));
        }
    }
    if let Some(metric) = params.get("metric") {
        if !matches!(metric.as_str(), "sum" | "avg" | "count") {
            return Err(QueryIssue::bad("Invalid metric"));
        }
    }
    if let Some(unit) = params.get("unit") {
        if !matches!(unit.as_str(), "minute" | "hour" | "day" | "month" | "year") {
            return Err(QueryIssue::bad("Invalid unit"));
        }
    }
    if let Some(value) = params.get("match") {
        if !matches!(value.as_str(), "all" | "any") {
            return Err(QueryIssue::bad("Invalid match"));
        }
    }
    if let Some(value) = params.get("testMode") {
        if !matches!(value.as_str(), "true" | "false") {
            return Err(QueryIssue::bad("Invalid testMode"));
        }
    }

    let controls = controls(source, operation);
    for key in params.keys() {
        let base = key.trim_end_matches(|character: char| character.is_ascii_digit());
        if controls.contains(&key.as_str())
            || FILTER_NAMES.contains(&base)
            || key.starts_with("pf_")
            || matches!(key.as_str(), "match" | "testMode" | "excludeBounce")
        {
            continue;
        }
        return Err(QueryIssue::bad(format!(
            "Unsupported query parameter: {key}"
        )));
    }
    parse_property_filters(params)?;
    parse_filters(params)?;
    pagination(params)?;
    validate_date_params(params)?;
    Ok(())
}

fn parse_integer(params: &HashMap<String, String>, key: &str) -> Result<Option<i64>, QueryIssue> {
    params
        .get(key)
        .map(|value| {
            value
                .parse::<i64>()
                .map_err(|_| QueryIssue::bad(format!("{key} must be an integer")))
        })
        .transpose()
}

fn pagination(
    params: &HashMap<String, String>,
) -> Result<(i64, i64, i64, Option<i64>), QueryIssue> {
    let page = parse_integer(params, "page")?.unwrap_or(1);
    let page_size = parse_integer(params, "pageSize")?.unwrap_or(20);
    let max_results = parse_integer(params, "maxResults")?;
    if page < 1 || page_size < 1 || max_results.is_some_and(|value| value < 1) {
        return Err(QueryIssue::bad(
            "page, pageSize and maxResults must be positive integers",
        ));
    }
    let offset = page
        .checked_sub(1)
        .and_then(|value| value.checked_mul(page_size))
        .ok_or_else(|| QueryIssue::bad("Pagination is too large"))?;
    Ok((page, page_size, offset, max_results))
}

fn validate_date_params(params: &HashMap<String, String>) -> Result<(), QueryIssue> {
    let has_start = params.contains_key("startAt");
    let has_end = params.contains_key("endAt");
    let has_period = params.contains_key("period");
    if has_start != has_end {
        return Err(QueryIssue::bad(
            "startAt and endAt must be provided together",
        ));
    }
    if !has_period && !has_start {
        return Err(QueryIssue::bad(
            "Either startAt+endAt or period must be provided",
        ));
    }
    if has_period && has_start {
        return Err(QueryIssue::bad(
            "period cannot be combined with an explicit date range",
        ));
    }
    if has_start {
        let start = parse_integer(params, "startAt")?.expect("present startAt");
        let end = parse_integer(params, "endAt")?.expect("present endAt");
        let start = Utc
            .timestamp_millis_opt(start)
            .single()
            .ok_or_else(|| QueryIssue::bad("Invalid startAt"))?;
        let end = Utc
            .timestamp_millis_opt(end)
            .single()
            .ok_or_else(|| QueryIssue::bad("Invalid endAt"))?;
        if start > end {
            return Err(QueryIssue::bad("Invalid date range"));
        }
    }
    Ok(())
}

fn normalize_timezone(timezone: &str) -> &str {
    match timezone {
        "America/Buenos_Aires" => "America/Argentina/Buenos_Aires",
        "America/Catamarca" => "America/Argentina/Catamarca",
        "America/Cordoba" | "America/Rosario" => "America/Argentina/Cordoba",
        "America/Jujuy" => "America/Argentina/Jujuy",
        "America/Mendoza" => "America/Argentina/Mendoza",
        "Asia/Batavia" => "Asia/Jakarta",
        "Asia/Calcutta" => "Asia/Kolkata",
        "Asia/Chongqing" | "Asia/Harbin" => "Asia/Shanghai",
        "Asia/Katmandu" => "Asia/Kathmandu",
        "Asia/Macao" => "Asia/Macau",
        "Asia/Rangoon" => "Asia/Yangon",
        "Asia/Saigon" => "Asia/Ho_Chi_Minh",
        "Europe/Kiev" | "Europe/Zaporozhye" => "Europe/Kyiv",
        "Etc/UTC" => "UTC",
        "US/Arizona" => "America/Phoenix",
        "US/Central" => "America/Chicago",
        "US/Eastern" => "America/New_York",
        "US/Mountain" => "America/Denver",
        "US/Pacific" => "America/Los_Angeles",
        "US/Samoa" => "Pacific/Pago_Pago",
        _ => timezone,
    }
}

async fn context(
    state: &AppState,
    headers: &HeaderMap,
    website_id: Uuid,
    params: &mut HashMap<String, String>,
    source: Source,
    operation: Operation,
) -> Result<Context, Response> {
    let (section, visibility) = share_access(source, operation);
    let pool = website_section_pool(state, headers, website_id, &[section], visibility).await?;
    let audience = crate::audience::Audience::resolve(&pool, website_id, params).await?;
    validate_params(params, source, operation).map_err(QueryIssue::response)?;
    let timezone =
        normalize_timezone(params.get("timezone").map(String::as_str).unwrap_or("UTC")).to_string();
    let valid_timezone = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM pg_timezone_names WHERE name = $1)",
    )
    .bind(&timezone)
    .fetch_one(&pool)
    .await
    .map_err(database_error)?;
    if !valid_timezone {
        return Err(error_response(StatusCode::BAD_REQUEST, "Invalid timezone"));
    }
    let mut range = date_range(&pool, params, &timezone).await?;
    if let Some(reset_at) = sqlx::query_scalar::<_, Option<DateTime<Utc>>>(
        "SELECT reset_at FROM website WHERE website_id = $1",
    )
    .bind(website_id)
    .fetch_optional(&pool)
    .await
    .map_err(database_error)?
    .flatten()
    {
        range.start = range.start.max(reset_at);
    }
    Ok(Context {
        pool,
        audience,
        range,
        timezone,
        unit: match params.get("unit").map(String::as_str).unwrap_or("day") {
            "minute" => "minute",
            "hour" => "hour",
            "month" => "month",
            "year" => "year",
            _ => "day",
        },
        filters: parse_filters(params).map_err(QueryIssue::response)?,
        property_filters: parse_property_filters(params).map_err(QueryIssue::response)?,
    })
}

fn share_access(source: Source, operation: Operation) -> (ShareSection, ShareDataVisibility) {
    match (source, operation) {
        (Source::Event, Operation::Pivot) => (ShareSection::Events, ShareDataVisibility::Rows),
        (Source::Event, _) => (ShareSection::Events, ShareDataVisibility::Aggregate),
        (Source::Session, _) => (ShareSection::Sessions, ShareDataVisibility::Aggregate),
    }
}

async fn date_range(
    pool: &PgPool,
    params: &HashMap<String, String>,
    timezone: &str,
) -> Result<DateRange, Response> {
    if let Some(period) = params.get("period") {
        let sql = match period.as_str() {
            "24h" => "SELECT CURRENT_TIMESTAMP - INTERVAL '24 hours', CURRENT_TIMESTAMP WHERE $1::text IS NOT NULL",
            "7d" => "SELECT CURRENT_TIMESTAMP - INTERVAL '7 days', CURRENT_TIMESTAMP WHERE $1::text IS NOT NULL",
            "30d" => "SELECT CURRENT_TIMESTAMP - INTERVAL '30 days', CURRENT_TIMESTAMP WHERE $1::text IS NOT NULL",
            "today" | "0day" => "SELECT date_trunc('day', CURRENT_TIMESTAMP AT TIME ZONE $1) AT TIME ZONE $1, (date_trunc('day', CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 day' - INTERVAL '1 millisecond') AT TIME ZONE $1",
            "24hour" => "SELECT (date_trunc('hour', CURRENT_TIMESTAMP AT TIME ZONE $1) - INTERVAL '24 hours') AT TIME ZONE $1, (date_trunc('hour', CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 hour' - INTERVAL '1 millisecond') AT TIME ZONE $1",
            "0week" => "SELECT (date_trunc('week', (CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 day') - INTERVAL '1 day') AT TIME ZONE $1, (date_trunc('week', (CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 day') + INTERVAL '6 days 23 hours 59 minutes 59.999 seconds') AT TIME ZONE $1",
            "7day" => "SELECT (date_trunc('day', CURRENT_TIMESTAMP AT TIME ZONE $1) - INTERVAL '7 days') AT TIME ZONE $1, (date_trunc('day', CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 day' - INTERVAL '1 millisecond') AT TIME ZONE $1",
            "0month" => "SELECT date_trunc('month', CURRENT_TIMESTAMP AT TIME ZONE $1) AT TIME ZONE $1, (date_trunc('month', CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 month' - INTERVAL '1 millisecond') AT TIME ZONE $1",
            "30day" => "SELECT (date_trunc('day', CURRENT_TIMESTAMP AT TIME ZONE $1) - INTERVAL '30 days') AT TIME ZONE $1, (date_trunc('day', CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 day' - INTERVAL '1 millisecond') AT TIME ZONE $1",
            "90day" => "SELECT (date_trunc('day', CURRENT_TIMESTAMP AT TIME ZONE $1) - INTERVAL '90 days') AT TIME ZONE $1, (date_trunc('day', CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 day' - INTERVAL '1 millisecond') AT TIME ZONE $1",
            "0year" => "SELECT date_trunc('year', CURRENT_TIMESTAMP AT TIME ZONE $1) AT TIME ZONE $1, (date_trunc('year', CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 year' - INTERVAL '1 millisecond') AT TIME ZONE $1",
            "6month" => "SELECT (date_trunc('month', CURRENT_TIMESTAMP AT TIME ZONE $1) - INTERVAL '6 months') AT TIME ZONE $1, (date_trunc('month', CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 month' - INTERVAL '1 millisecond') AT TIME ZONE $1",
            "12month" => "SELECT (date_trunc('month', CURRENT_TIMESTAMP AT TIME ZONE $1) - INTERVAL '12 months') AT TIME ZONE $1, (date_trunc('month', CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 month' - INTERVAL '1 millisecond') AT TIME ZONE $1",
            _ => return Err(error_response(StatusCode::BAD_REQUEST, "Invalid period")),
        };
        let row = sqlx::query(sql)
            .bind(timezone)
            .fetch_one(pool)
            .await
            .map_err(database_error)?;
        return Ok(DateRange {
            start: row.get(0),
            end: row.get(1),
        });
    }
    let start = params["startAt"]
        .parse::<i64>()
        .ok()
        .and_then(|value| Utc.timestamp_millis_opt(value).single())
        .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, "Invalid startAt"))?;
    let end = params["endAt"]
        .parse::<i64>()
        .ok()
        .and_then(|value| Utc.timestamp_millis_opt(value).single())
        .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, "Invalid endAt"))?;
    Ok(DateRange { start, end })
}

fn operator(value: &str) -> Option<Operator> {
    Some(match value {
        "eq" => Operator::Eq,
        "neq" => Operator::Neq,
        "s" => Operator::Starts,
        "ns" => Operator::NotStarts,
        "c" => Operator::Contains,
        "dnc" => Operator::NotContains,
        "re" => Operator::Regex,
        "nre" => Operator::NotRegex,
        "gt" => Operator::Greater,
        "lt" => Operator::Less,
        "gte" => Operator::GreaterEqual,
        "lte" => Operator::LessEqual,
        "bf" => Operator::Before,
        "af" => Operator::After,
        _ => return None,
    })
}

fn filter_column(name: &str) -> Option<(&'static str, &'static str, bool)> {
    Some(match name {
        "path" => ("path", "e.url_path", false),
        "referrer" => ("referrer", "e.referrer_domain", false),
        "title" => ("title", "e.page_title", false),
        "query" => ("query", "e.url_query", false),
        "os" => ("os", "s.os", true),
        "browser" => ("browser", "s.browser", true),
        "device" => ("device", "s.device", true),
        "country" => ("country", "s.country", true),
        "region" => ("region", "s.region", true),
        "city" => ("city", "s.city", true),
        "tag" => ("tag", "e.tag", false),
        "hostname" => ("hostname", "e.hostname", false),
        "distinctId" => ("distinctId", "s.distinct_id", true),
        "language" => ("language", "s.language", true),
        "event" => ("event", "e.event_name", false),
        "utmSource" => ("utmSource", "e.utm_source", false),
        "utmMedium" => ("utmMedium", "e.utm_medium", false),
        "utmCampaign" => ("utmCampaign", "e.utm_campaign", false),
        "utmContent" => ("utmContent", "e.utm_content", false),
        "utmTerm" => ("utmTerm", "e.utm_term", false),
        "eventType" => ("eventType", "e.event_type", false),
        _ => return None,
    })
}

fn parse_filters(params: &HashMap<String, String>) -> Result<Filters, QueryIssue> {
    let mut values = Vec::new();
    let mut has_tag = false;
    let mut needs_session = false;
    for (key, value) in params {
        let base = key.trim_end_matches(|character: char| character.is_ascii_digit());
        let Some((name, column, session)) = filter_column(base) else {
            continue;
        };
        let (parsed_operator, raw) = value
            .split_once('.')
            .and_then(|(candidate, raw)| operator(candidate).map(|operator| (operator, raw)))
            .unwrap_or((Operator::Eq, value.as_str()));
        if !matches!(
            parsed_operator,
            Operator::Eq
                | Operator::Neq
                | Operator::Starts
                | Operator::NotStarts
                | Operator::Contains
                | Operator::NotContains
                | Operator::Regex
                | Operator::NotRegex
        ) && name != "eventType"
        {
            return Err(QueryIssue::bad(format!("Invalid operator for {name}")));
        }
        let items = if matches!(parsed_operator, Operator::Eq | Operator::Neq) {
            raw.split(',').map(str::to_string).collect::<Vec<_>>()
        } else {
            vec![raw.to_string()]
        };
        if items.iter().any(String::is_empty) {
            return Err(QueryIssue::bad(format!("Invalid filter value for {name}")));
        }
        if matches!(parsed_operator, Operator::Regex | Operator::NotRegex)
            && Regex::new(&items[0]).is_err()
        {
            return Err(QueryIssue::bad(format!(
                "Invalid regular expression for {name}"
            )));
        }
        if name == "eventType"
            && (parsed_operator != Operator::Eq
                || items.len() != 1
                || items[0]
                    .parse::<i32>()
                    .ok()
                    .filter(|value| *value > 0)
                    .is_none())
        {
            return Err(QueryIssue::bad("Invalid eventType"));
        }
        has_tag |= name == "tag";
        needs_session |= session;
        values.push(Filter {
            name,
            column,
            operator: parsed_operator,
            values: items,
        });
    }
    if !has_tag {
        values.push(Filter {
            name: "tag",
            column: "e.tag",
            operator: if params.get("testMode").map(String::as_str) == Some("true") {
                Operator::Eq
            } else {
                Operator::Neq
            },
            values: vec!["test".to_string()],
        });
    }
    Ok(Filters {
        values,
        match_any: params.get("match").map(String::as_str) == Some("any"),
        needs_session,
    })
}

fn parse_property_filters(
    params: &HashMap<String, String>,
) -> Result<Vec<PropertyFilter>, QueryIssue> {
    let mut filters = Vec::new();
    for (key, value) in params {
        let Some(name) = key.strip_prefix("pf_") else {
            continue;
        };
        let property_name = name.trim_end_matches(|character: char| character.is_ascii_digit());
        if property_name.is_empty() || property_name.len() > 500 {
            return Err(QueryIssue::bad("Invalid property filter name"));
        }
        let mut parts = value.splitn(3, '.');
        let data_type = parts
            .next()
            .and_then(|value| value.parse::<i32>().ok())
            .filter(|value| (1..=5).contains(value))
            .ok_or_else(|| QueryIssue::bad(format!("Invalid property filter: {key}")))?;
        let parsed_operator = parts
            .next()
            .and_then(operator)
            .ok_or_else(|| QueryIssue::bad(format!("Invalid property filter: {key}")))?;
        let raw = parts
            .next()
            .ok_or_else(|| QueryIssue::bad(format!("Invalid property filter: {key}")))?;
        let valid_operator = match data_type {
            1 | 3 => matches!(
                parsed_operator,
                Operator::Eq
                    | Operator::Neq
                    | Operator::Contains
                    | Operator::NotContains
                    | Operator::Regex
                    | Operator::NotRegex
            ),
            2 => matches!(
                parsed_operator,
                Operator::Eq
                    | Operator::Neq
                    | Operator::Greater
                    | Operator::Less
                    | Operator::GreaterEqual
                    | Operator::LessEqual
            ),
            4 => matches!(
                parsed_operator,
                Operator::Eq | Operator::Before | Operator::After
            ),
            5 => matches!(parsed_operator, Operator::Contains | Operator::NotContains),
            _ => false,
        };
        if !valid_operator || raw.is_empty() {
            return Err(QueryIssue::bad(format!("Invalid property filter: {key}")));
        }
        match data_type {
            2 if Decimal::from_str(raw).is_err() => {
                return Err(QueryIssue::bad(format!("Invalid numeric filter: {key}")));
            }
            3 if !matches!(raw, "true" | "false") => {
                return Err(QueryIssue::bad(format!("Invalid boolean filter: {key}")));
            }
            4 if chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d").is_err() => {
                return Err(QueryIssue::bad(format!("Invalid date filter: {key}")));
            }
            1 | 3
                if matches!(parsed_operator, Operator::Regex | Operator::NotRegex)
                    && Regex::new(raw).is_err() =>
            {
                return Err(QueryIssue::bad(format!(
                    "Invalid regular expression: {key}"
                )));
            }
            _ => {}
        }
        filters.push(PropertyFilter {
            property_name: property_name.to_string(),
            data_type,
            operator: parsed_operator,
            value: raw.to_string(),
        });
    }
    Ok(filters)
}

fn push_filter(builder: &mut QueryBuilder<'_, Postgres>, filter: &Filter) {
    if filter.name == "eventType" {
        builder.push(filter.column).push(" = ").push_bind(
            filter.values[0]
                .parse::<i32>()
                .expect("validated eventType"),
        );
        return;
    }
    match filter.operator {
        Operator::Eq => {
            builder
                .push(filter.column)
                .push(" = ANY(")
                .push_bind(filter.values.clone())
                .push(")");
        }
        Operator::Neq => {
            builder
                .push("COALESCE(")
                .push(filter.column)
                .push(", '') != ALL(")
                .push_bind(filter.values.clone())
                .push(")");
        }
        Operator::Starts => {
            builder
                .push(filter.column)
                .push(" ILIKE ")
                .push_bind(format!("{}%", filter.values[0]));
        }
        Operator::NotStarts => {
            builder
                .push("COALESCE(")
                .push(filter.column)
                .push(", '') NOT ILIKE ")
                .push_bind(format!("{}%", filter.values[0]));
        }
        Operator::Contains => {
            builder
                .push(filter.column)
                .push(" ILIKE ")
                .push_bind(format!("%{}%", filter.values[0]));
        }
        Operator::NotContains => {
            builder
                .push("COALESCE(")
                .push(filter.column)
                .push(", '') NOT ILIKE ")
                .push_bind(format!("%{}%", filter.values[0]));
        }
        Operator::Regex => {
            builder
                .push(filter.column)
                .push(" ~* ")
                .push_bind(filter.values[0].clone());
        }
        Operator::NotRegex => {
            builder
                .push("COALESCE(")
                .push(filter.column)
                .push(", '') !~* ")
                .push_bind(filter.values[0].clone());
        }
        _ => unreachable!("common filter operator validated"),
    }
}

fn push_filters(builder: &mut QueryBuilder<'_, Postgres>, filters: &Filters) {
    let (always, matched): (Vec<_>, Vec<_>) = filters
        .values
        .iter()
        .partition(|filter| filter.name == "eventType" || !filters.match_any);
    if !matched.is_empty() {
        builder.push(" AND (");
        for (index, filter) in matched.into_iter().enumerate() {
            if index > 0 {
                builder.push(" OR ");
            }
            push_filter(builder, filter);
        }
        builder.push(")");
    }
    for filter in always {
        builder.push(" AND ");
        push_filter(builder, filter);
    }
    if filters
        .values
        .iter()
        .any(|filter| filter.name == "referrer")
    {
        builder.push(
            " AND (e.referrer_domain != regexp_replace(e.hostname, '^www.', '') OR e.referrer_domain IS NULL)",
        );
    }
}

fn push_property_filters(
    builder: &mut QueryBuilder<'_, Postgres>,
    source: Source,
    range: DateRange,
    timezone: &str,
    filters: &[PropertyFilter],
) {
    for filter in filters {
        builder.push(" AND EXISTS (SELECT 1 FROM ");
        match source {
            Source::Event => builder.push(
                "event_data pf WHERE pf.website_id = e.website_id AND pf.website_event_id = e.event_id",
            ),
            Source::Session => builder.push(
                "session_data pf WHERE pf.website_id = e.website_id AND pf.session_id = e.session_id",
            ),
        };
        if source == Source::Event {
            builder
                .push(" AND pf.created_at BETWEEN ")
                .push_bind(range.start)
                .push(" AND ")
                .push_bind(range.end);
        }
        builder
            .push(" AND pf.data_key = ")
            .push_bind(filter.property_name.clone())
            .push(" AND pf.data_type = ")
            .push_bind(filter.data_type)
            .push(" AND ");
        match filter.data_type {
            2 => {
                builder.push("pf.number_value ");
                builder.push(match filter.operator {
                    Operator::Eq => "= ",
                    Operator::Neq => "!= ",
                    Operator::Greater => "> ",
                    Operator::Less => "< ",
                    Operator::GreaterEqual => ">= ",
                    Operator::LessEqual => "<= ",
                    _ => unreachable!(),
                });
                builder.push_bind(Decimal::from_str(&filter.value).expect("validated decimal"));
            }
            4 => {
                builder
                    .push("(pf.date_value AT TIME ZONE ")
                    .push_bind(timezone.to_string())
                    .push(")::date ");
                builder.push(match filter.operator {
                    Operator::Before => "< ",
                    Operator::After => "> ",
                    _ => "= ",
                });
                builder.push_bind(filter.value.clone()).push("::date");
            }
            5 => {
                if filter.operator == Operator::NotContains {
                    builder.push("NOT ");
                }
                builder
                    .push("EXISTS (SELECT 1 FROM jsonb_array_elements_text(CASE WHEN jsonb_typeof(COALESCE(pf.string_value, '[]')::jsonb) = 'array' THEN COALESCE(pf.string_value, '[]')::jsonb ELSE '[]'::jsonb END) item(value) WHERE item.value = ")
                    .push_bind(filter.value.clone())
                    .push(")");
            }
            _ => match filter.operator {
                Operator::Eq | Operator::Neq => {
                    builder.push("pf.string_value ");
                    builder.push(if filter.operator == Operator::Eq {
                        "= ANY("
                    } else {
                        "!= ALL("
                    });
                    builder
                        .push_bind(
                            filter
                                .value
                                .split(',')
                                .map(str::to_string)
                                .collect::<Vec<_>>(),
                        )
                        .push(")");
                }
                Operator::Contains | Operator::NotContains => {
                    if filter.operator == Operator::NotContains {
                        builder.push("COALESCE(pf.string_value, '') NOT ILIKE ");
                    } else {
                        builder.push("pf.string_value ILIKE ");
                    }
                    builder.push_bind(format!("%{}%", filter.value));
                }
                Operator::Regex | Operator::NotRegex => {
                    if filter.operator == Operator::NotRegex {
                        builder.push("COALESCE(pf.string_value, '') !~* ");
                    } else {
                        builder.push("pf.string_value ~* ");
                    }
                    builder.push_bind(filter.value.clone());
                }
                _ => unreachable!(),
            },
        }
        builder.push(")");
    }
}

fn push_event_scope(
    builder: &mut QueryBuilder<'_, Postgres>,
    context: &Context,
    website_id: Uuid,
    event_name: &str,
) {
    builder.push("SELECT e.event_id FROM website_event e ");
    if context.filters.needs_session {
        builder
            .push("JOIN session s ON s.session_id = e.session_id AND s.website_id = e.website_id ");
    }
    builder
        .push("WHERE e.website_id = ")
        .push_bind(website_id)
        .push(" AND e.created_at BETWEEN ")
        .push_bind(context.range.start)
        .push(" AND ")
        .push_bind(context.range.end)
        .push(" AND e.event_type = 2 AND e.event_name = ")
        .push_bind(event_name.to_string());
    push_filters(builder, &context.filters);
    context
        .audience
        .push(builder, "e", context.range.start, context.range.end);
    push_property_filters(
        builder,
        Source::Event,
        context.range,
        &context.timezone,
        &context.property_filters,
    );
}

fn push_session_scope(
    builder: &mut QueryBuilder<'_, Postgres>,
    context: &Context,
    website_id: Uuid,
) {
    builder.push("SELECT DISTINCT e.session_id FROM website_event e ");
    if context.filters.needs_session {
        builder
            .push("JOIN session s ON s.session_id = e.session_id AND s.website_id = e.website_id ");
    }
    builder
        .push("WHERE e.website_id = ")
        .push_bind(website_id)
        .push(" AND e.created_at BETWEEN ")
        .push_bind(context.range.start)
        .push(" AND ")
        .push_bind(context.range.end)
        .push(" AND e.event_type != 5");
    push_filters(builder, &context.filters);
    context
        .audience
        .push(builder, "e", context.range.start, context.range.end);
    push_property_filters(
        builder,
        Source::Session,
        context.range,
        &context.timezone,
        &context.property_filters,
    );
}

fn bucket_format(unit: &str) -> &'static str {
    match unit {
        "minute" => "YYYY-MM-DD\"T\"HH24:MI:00\"Z\"",
        "hour" | "day" => "YYYY-MM-DD\"T\"HH24:00:00\"Z\"",
        "month" => "YYYY-MM-01\"T\"HH24:00:00\"Z\"",
        "year" => "YYYY-01-01\"T\"HH24:00:00\"Z\"",
        _ => unreachable!("validated unit"),
    }
}

fn push_bucket(builder: &mut QueryBuilder<'_, Postgres>, field: &str, unit: &str, timezone: &str) {
    builder
        .push("to_char(date_trunc('")
        .push(unit)
        .push("', ")
        .push(field)
        .push(" AT TIME ZONE ")
        .push_bind(timezone.to_string())
        .push("), '")
        .push(bucket_format(unit))
        .push("')");
}

fn database_error(error: sqlx::Error) -> Response {
    error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}

async fn event_pivot_query(
    context: &Context,
    website_id: Uuid,
    event_name: &str,
    page: i64,
    page_size: i64,
    offset: i64,
    max_results: Option<i64>,
) -> Result<Value, sqlx::Error> {
    let mut count = QueryBuilder::<Postgres>::new("WITH filtered_events AS (");
    push_event_scope(&mut count, context, website_id, event_name);
    count.push(
        " AND EXISTS (SELECT 1 FROM event_data d WHERE d.website_id = e.website_id AND d.website_event_id = e.event_id AND d.created_at BETWEEN ",
    )
    .push_bind(context.range.start)
    .push(" AND ")
    .push_bind(context.range.end)
    .push(")) SELECT count(*)::bigint FROM filtered_events");
    let actual_count = count
        .build_query_scalar::<i64>()
        .fetch_one(&context.pool)
        .await?;
    let result_count = max_results
        .map(|limit| actual_count.min(limit))
        .unwrap_or(actual_count);

    let mut query = QueryBuilder::<Postgres>::new("WITH filtered_events AS (");
    push_event_scope(&mut query, context, website_id, event_name);
    query.push(
        " AND EXISTS (SELECT 1 FROM event_data d WHERE d.website_id = e.website_id AND d.website_event_id = e.event_id AND d.created_at BETWEEN ",
    )
    .push_bind(context.range.start)
    .push(" AND ")
    .push_bind(context.range.end)
    .push(
        ")), paged_events AS (SELECT e.event_id, e.session_id, e.event_name, e.url_path, e.created_at FROM website_event e JOIN filtered_events f ON f.event_id = e.event_id ORDER BY e.created_at DESC LIMIT ",
    )
    .push_bind(page_size)
    .push(" OFFSET ")
    .push_bind(offset)
    .push(
        ") SELECT p.event_id, p.session_id, p.event_name, p.url_path, p.created_at, array_agg(d.data_key ORDER BY d.data_key), array_agg(COALESCE(CASE WHEN d.data_type IN (1, 3, 5) THEN d.string_value END, CASE WHEN d.data_type = 2 THEN d.number_value::varchar END, CASE WHEN d.data_type = 4 THEN to_char(d.date_value AT TIME ZONE ",
    )
    .push_bind(context.timezone.clone())
    .push(
        ", 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') END, '') ORDER BY d.data_key) FROM paged_events p JOIN event_data d ON d.website_event_id = p.event_id AND d.website_id = ",
    )
    .push_bind(website_id)
    .push(" AND d.created_at BETWEEN ")
    .push_bind(context.range.start)
    .push(" AND ")
    .push_bind(context.range.end)
    .push(" GROUP BY p.event_id, p.session_id, p.event_name, p.url_path, p.created_at ORDER BY p.created_at DESC");
    let rows = query.build().fetch_all(&context.pool).await?;
    let data = rows
        .into_iter()
        .map(|row| {
            json!({
                "eventId": row.get::<Uuid, _>(0),
                "sessionId": row.get::<Uuid, _>(1),
                "eventName": row.get::<String, _>(2),
                "urlPath": row.get::<String, _>(3),
                "createdAt": row.get::<DateTime<Utc>, _>(4),
                "propertyKeys": row.get::<Vec<String>, _>(5),
                "propertyValues": row.get::<Vec<String>, _>(6),
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "data": data,
        "count": result_count,
        "page": page,
        "pageSize": page_size,
        "isCapped": max_results.is_some_and(|limit| actual_count >= limit),
    }))
}

fn event_series_prefix(
    context: &Context,
    website_id: Uuid,
    event_name: &str,
) -> QueryBuilder<'static, Postgres> {
    let mut query = QueryBuilder::<Postgres>::new("WITH filtered_events AS (");
    push_event_scope(&mut query, context, website_id, event_name);
    query.push(") ");
    query
}

async fn event_array_query(
    context: &Context,
    website_id: Uuid,
    event_name: &str,
    property_name: &str,
) -> Result<Value, sqlx::Error> {
    let mut query = event_series_prefix(context, website_id, event_name);
    query.push("SELECT item.value AS x, ");
    push_bucket(&mut query, "d.created_at", context.unit, &context.timezone);
    query
        .push(
            " AS t, count(*)::bigint AS y FROM event_data d JOIN filtered_events f ON f.event_id = d.website_event_id CROSS JOIN LATERAL jsonb_array_elements_text(CASE WHEN jsonb_typeof(COALESCE(d.string_value, '[]')::jsonb) = 'array' THEN COALESCE(d.string_value, '[]')::jsonb ELSE '[]'::jsonb END) item(value) WHERE d.website_id = ",
        )
        .push_bind(website_id)
        .push(" AND d.created_at BETWEEN ")
        .push_bind(context.range.start)
        .push(" AND ")
        .push_bind(context.range.end)
        .push(" AND d.data_key = ")
        .push_bind(property_name.to_string())
        .push(" AND d.data_type = 5 GROUP BY 1, 2 ORDER BY 2");
    let rows = query.build().fetch_all(&context.pool).await?;
    Ok(Value::Array(
        rows.into_iter()
            .map(|row| {
                json!({
                    "x": row.get::<String, _>("x"),
                    "t": row.get::<String, _>("t"),
                    "y": row.get::<i64, _>("y"),
                })
            })
            .collect(),
    ))
}

async fn event_date_query(
    context: &Context,
    website_id: Uuid,
    event_name: &str,
    property_name: &str,
) -> Result<Value, sqlx::Error> {
    let mut query = event_series_prefix(context, website_id, event_name);
    query
        .push("SELECT to_char(d.date_value AT TIME ZONE ")
        .push_bind(context.timezone.clone())
        .push(
            ", 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS t, count(*)::bigint AS y FROM event_data d JOIN filtered_events f ON f.event_id = d.website_event_id WHERE d.website_id = ",
        )
        .push_bind(website_id)
        .push(" AND d.created_at BETWEEN ")
        .push_bind(context.range.start)
        .push(" AND ")
        .push_bind(context.range.end)
        .push(" AND d.data_key = ")
        .push_bind(property_name.to_string())
        .push(" AND d.data_type = 4 GROUP BY 1 ORDER BY 1");
    let rows = query.build().fetch_all(&context.pool).await?;
    Ok(Value::Array(
        rows.into_iter()
            .map(|row| {
                json!({
                    "t": row.get::<String, _>("t"),
                    "y": row.get::<i64, _>("y"),
                })
            })
            .collect(),
    ))
}

async fn event_numeric_series_query(
    context: &Context,
    website_id: Uuid,
    event_name: &str,
    property_name: &str,
    metric: &str,
) -> Result<Value, sqlx::Error> {
    let mut query = event_series_prefix(context, website_id, event_name);
    query.push("SELECT ");
    push_bucket(&mut query, "d.created_at", context.unit, &context.timezone);
    query.push(" AS t, ");
    query.push(match metric {
        "avg" => "COALESCE(avg(d.number_value), 0)::float8",
        "count" => "count(*)::float8",
        _ => "COALESCE(sum(d.number_value), 0)::float8",
    });
    query
        .push(
            " AS y FROM event_data d JOIN filtered_events f ON f.event_id = d.website_event_id WHERE d.website_id = ",
        )
        .push_bind(website_id)
        .push(" AND d.created_at BETWEEN ")
        .push_bind(context.range.start)
        .push(" AND ")
        .push_bind(context.range.end)
        .push(" AND d.data_key = ")
        .push_bind(property_name.to_string())
        .push(" AND d.data_type = 2 GROUP BY 1 ORDER BY 1");
    let rows = query.build().fetch_all(&context.pool).await?;
    Ok(Value::Array(
        rows.into_iter()
            .map(|row| {
                json!({
                    "t": row.get::<String, _>("t"),
                    "y": row.get::<f64, _>("y"),
                })
            })
            .collect(),
    ))
}

async fn event_numeric_stats_query(
    context: &Context,
    website_id: Uuid,
    event_name: &str,
    property_name: &str,
) -> Result<Value, sqlx::Error> {
    let mut query = event_series_prefix(context, website_id, event_name);
    query
        .push(
            "SELECT COALESCE(sum(d.number_value), 0)::float8 AS total, COALESCE(avg(d.number_value), 0)::float8 AS average, COALESCE(percentile_cont(0.5) WITHIN GROUP (ORDER BY d.number_value), 0)::float8 AS median, COALESCE(max(d.number_value), 0)::float8 AS max, COALESCE(min(d.number_value), 0)::float8 AS min FROM event_data d JOIN filtered_events f ON f.event_id = d.website_event_id WHERE d.website_id = ",
        )
        .push_bind(website_id)
        .push(" AND d.created_at BETWEEN ")
        .push_bind(context.range.start)
        .push(" AND ")
        .push_bind(context.range.end)
        .push(" AND d.data_key = ")
        .push_bind(property_name.to_string())
        .push(" AND d.data_type = 2");
    let row = query.build().fetch_one(&context.pool).await?;
    Ok(json!({
        "total": row.get::<f64, _>("total"),
        "average": row.get::<f64, _>("average"),
        "median": row.get::<f64, _>("median"),
        "max": row.get::<f64, _>("max"),
        "min": row.get::<f64, _>("min"),
    }))
}

async fn event_property_query(
    context: &Context,
    website_id: Uuid,
    event_name: &str,
    property_name: &str,
) -> Result<Value, sqlx::Error> {
    let mut query = event_series_prefix(context, website_id, event_name);
    query.push("SELECT d.string_value AS x, ");
    push_bucket(&mut query, "d.created_at", context.unit, &context.timezone);
    query
        .push(
            " AS t, count(*)::bigint AS y FROM event_data d JOIN filtered_events f ON f.event_id = d.website_event_id WHERE d.website_id = ",
        )
        .push_bind(website_id)
        .push(" AND d.created_at BETWEEN ")
        .push_bind(context.range.start)
        .push(" AND ")
        .push_bind(context.range.end)
        .push(" AND d.data_key = ")
        .push_bind(property_name.to_string())
        .push(" AND d.data_type IN (1, 3) GROUP BY 1, 2 ORDER BY 2");
    let rows = query.build().fetch_all(&context.pool).await?;
    Ok(Value::Array(
        rows.into_iter()
            .map(|row| {
                json!({
                    "x": row.get::<String, _>("x"),
                    "t": row.get::<String, _>("t"),
                    "y": row.get::<i64, _>("y"),
                })
            })
            .collect(),
    ))
}

fn session_series_prefix(context: &Context, website_id: Uuid) -> QueryBuilder<'static, Postgres> {
    let mut query = QueryBuilder::<Postgres>::new("WITH filtered_sessions AS (");
    push_session_scope(&mut query, context, website_id);
    query.push(") ");
    query
}

async fn session_pivot_query(
    context: &Context,
    website_id: Uuid,
    property_name: &str,
    page: i64,
    page_size: i64,
    offset: i64,
    max_results: Option<i64>,
) -> Result<Value, sqlx::Error> {
    let mut count = session_series_prefix(context, website_id);
    count
        .push(
            "SELECT count(*)::bigint FROM (SELECT DISTINCT d.session_id FROM session_data d JOIN filtered_sessions f ON f.session_id = d.session_id WHERE d.website_id = ",
        )
        .push_bind(website_id)
        .push(" AND d.data_key = ")
        .push_bind(property_name.to_string())
        .push(") candidates");
    let actual_count = count
        .build_query_scalar::<i64>()
        .fetch_one(&context.pool)
        .await?;
    let result_count = max_results
        .map(|limit| actual_count.min(limit))
        .unwrap_or(actual_count);

    let mut query = session_series_prefix(context, website_id);
    query
        .push(
            ", latest_properties AS (SELECT ranked.session_id, ranked.distinct_id, ranked.created_at, ranked.data_key, ranked.data_type, ranked.string_value, ranked.number_value, ranked.date_value FROM (SELECT d.*, row_number() OVER (PARTITION BY d.session_id, d.data_key ORDER BY d.created_at DESC, d.session_data_id DESC) AS row_num FROM session_data d JOIN filtered_sessions f ON f.session_id = d.session_id WHERE d.website_id = ",
        )
        .push_bind(website_id)
        .push(
            ") ranked WHERE ranked.row_num = 1), paged_sessions AS (SELECT session_id, created_at AS sort_created_at FROM latest_properties WHERE data_key = ",
        )
        .push_bind(property_name.to_string())
        .push(" ORDER BY created_at DESC LIMIT ")
        .push_bind(page_size)
        .push(" OFFSET ")
        .push_bind(offset)
        .push(
            ") SELECT d.session_id, COALESCE(max(d.distinct_id), '') AS distinct_id, max(d.created_at) AS created_at, array_agg(d.data_key ORDER BY d.data_key), array_agg(COALESCE(CASE WHEN d.data_type IN (1, 3, 5) THEN d.string_value END, CASE WHEN d.data_type = 2 THEN d.number_value::varchar END, CASE WHEN d.data_type = 4 THEN to_char(d.date_value AT TIME ZONE ",
        )
        .push_bind(context.timezone.clone())
        .push(
            ", 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') END, '') ORDER BY d.data_key) FROM latest_properties d JOIN paged_sessions p ON p.session_id = d.session_id GROUP BY d.session_id, p.sort_created_at ORDER BY p.sort_created_at DESC",
        );
    let rows = query.build().fetch_all(&context.pool).await?;
    let data = rows
        .into_iter()
        .map(|row| {
            json!({
                "sessionId": row.get::<Uuid, _>(0),
                "distinctId": row.get::<String, _>(1),
                "createdAt": row.get::<DateTime<Utc>, _>(2),
                "propertyKeys": row.get::<Vec<String>, _>(3),
                "propertyValues": row.get::<Vec<String>, _>(4),
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "data": data,
        "count": result_count,
        "page": page,
        "pageSize": page_size,
        "isCapped": max_results.is_some_and(|limit| actual_count >= limit),
    }))
}

async fn session_array_query(
    context: &Context,
    website_id: Uuid,
    property_name: &str,
) -> Result<Value, sqlx::Error> {
    let mut query = session_series_prefix(context, website_id);
    query.push("SELECT item.value AS x, ");
    push_bucket(&mut query, "d.created_at", context.unit, &context.timezone);
    query
        .push(
            " AS t, count(DISTINCT d.session_id)::bigint AS y FROM session_data d JOIN filtered_sessions f ON f.session_id = d.session_id CROSS JOIN LATERAL jsonb_array_elements_text(CASE WHEN jsonb_typeof(COALESCE(d.string_value, '[]')::jsonb) = 'array' THEN COALESCE(d.string_value, '[]')::jsonb ELSE '[]'::jsonb END) item(value) WHERE d.website_id = ",
        )
        .push_bind(website_id)
        .push(" AND d.data_key = ")
        .push_bind(property_name.to_string())
        .push(" AND d.data_type = 5 GROUP BY 1, 2 ORDER BY 2");
    let rows = query.build().fetch_all(&context.pool).await?;
    Ok(Value::Array(
        rows.into_iter()
            .map(|row| {
                json!({
                    "x": row.get::<String, _>("x"),
                    "t": row.get::<String, _>("t"),
                    "y": row.get::<i64, _>("y"),
                })
            })
            .collect(),
    ))
}

async fn session_date_query(
    context: &Context,
    website_id: Uuid,
    property_name: &str,
) -> Result<Value, sqlx::Error> {
    let mut query = session_series_prefix(context, website_id);
    query
        .push("SELECT to_char(d.date_value AT TIME ZONE ")
        .push_bind(context.timezone.clone())
        .push(
            ", 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS t, count(DISTINCT d.session_id)::bigint AS y FROM session_data d JOIN filtered_sessions f ON f.session_id = d.session_id WHERE d.website_id = ",
        )
        .push_bind(website_id)
        .push(" AND d.data_key = ")
        .push_bind(property_name.to_string())
        .push(" AND d.data_type = 4 GROUP BY 1 ORDER BY 1");
    let rows = query.build().fetch_all(&context.pool).await?;
    Ok(Value::Array(
        rows.into_iter()
            .map(|row| {
                json!({
                    "t": row.get::<String, _>("t"),
                    "y": row.get::<i64, _>("y"),
                })
            })
            .collect(),
    ))
}

async fn session_numeric_series_query(
    context: &Context,
    website_id: Uuid,
    property_name: &str,
    metric: &str,
) -> Result<Value, sqlx::Error> {
    let mut query = session_series_prefix(context, website_id);
    query.push("SELECT ");
    push_bucket(&mut query, "d.created_at", context.unit, &context.timezone);
    query.push(" AS t, ");
    query.push(match metric {
        "avg" => "COALESCE(avg(d.number_value), 0)::float8",
        "count" => "count(DISTINCT d.session_id)::float8",
        _ => "COALESCE(sum(d.number_value), 0)::float8",
    });
    query
        .push(
            " AS y FROM session_data d JOIN filtered_sessions f ON f.session_id = d.session_id WHERE d.website_id = ",
        )
        .push_bind(website_id)
        .push(" AND d.data_key = ")
        .push_bind(property_name.to_string())
        .push(" AND d.data_type = 2 GROUP BY 1 ORDER BY 1");
    let rows = query.build().fetch_all(&context.pool).await?;
    Ok(Value::Array(
        rows.into_iter()
            .map(|row| {
                json!({
                    "t": row.get::<String, _>("t"),
                    "y": row.get::<f64, _>("y"),
                })
            })
            .collect(),
    ))
}

async fn session_numeric_stats_query(
    context: &Context,
    website_id: Uuid,
    property_name: &str,
) -> Result<Value, sqlx::Error> {
    let mut query = session_series_prefix(context, website_id);
    query
        .push(
            "SELECT COALESCE(sum(d.number_value), 0)::float8 AS total, COALESCE(avg(d.number_value), 0)::float8 AS average, COALESCE(percentile_cont(0.5) WITHIN GROUP (ORDER BY d.number_value), 0)::float8 AS median, COALESCE(max(d.number_value), 0)::float8 AS max, COALESCE(min(d.number_value), 0)::float8 AS min FROM session_data d JOIN filtered_sessions f ON f.session_id = d.session_id WHERE d.website_id = ",
        )
        .push_bind(website_id)
        .push(" AND d.data_key = ")
        .push_bind(property_name.to_string())
        .push(" AND d.data_type = 2");
    let row = query.build().fetch_one(&context.pool).await?;
    Ok(json!({
        "total": row.get::<f64, _>("total"),
        "average": row.get::<f64, _>("average"),
        "median": row.get::<f64, _>("median"),
        "max": row.get::<f64, _>("max"),
        "min": row.get::<f64, _>("min"),
    }))
}

async fn session_property_query(
    context: &Context,
    website_id: Uuid,
    property_name: &str,
) -> Result<Value, sqlx::Error> {
    let mut query = session_series_prefix(context, website_id);
    query.push("SELECT d.string_value AS x, ");
    push_bucket(&mut query, "d.created_at", context.unit, &context.timezone);
    query
        .push(
            " AS t, count(DISTINCT d.session_id)::bigint AS y FROM session_data d JOIN filtered_sessions f ON f.session_id = d.session_id WHERE d.website_id = ",
        )
        .push_bind(website_id)
        .push(" AND d.data_key = ")
        .push_bind(property_name.to_string())
        .push(" AND d.data_type IN (1, 3) GROUP BY 1, 2 ORDER BY 2");
    let rows = query.build().fetch_all(&context.pool).await?;
    Ok(Value::Array(
        rows.into_iter()
            .map(|row| {
                json!({
                    "x": row.get::<String, _>("x"),
                    "t": row.get::<String, _>("t"),
                    "y": row.get::<i64, _>("y"),
                })
            })
            .collect(),
    ))
}

async fn session_activity_query(
    context: &Context,
    website_id: Uuid,
    property_name: &str,
) -> Result<Value, sqlx::Error> {
    let mut query = session_series_prefix(context, website_id);
    query
        .push(
            ", session_rollup AS (SELECT e.session_id, e.visit_id, count(*)::bigint AS activity, count(*) FILTER (WHERE e.event_type = 1)::bigint AS views, count(*) FILTER (WHERE e.event_type = 2)::bigint AS events FROM website_event e JOIN filtered_sessions f ON f.session_id = e.session_id WHERE e.website_id = ",
        )
        .push_bind(website_id)
        .push(" AND e.created_at BETWEEN ")
        .push_bind(context.range.start)
        .push(" AND ")
        .push_bind(context.range.end)
        .push(
            " AND e.event_type != 5 GROUP BY e.session_id, e.visit_id), session_stats AS (SELECT session_id, count(*)::bigint AS visits, sum(activity)::bigint AS activity, sum(views)::bigint AS views, sum(events)::bigint AS events FROM session_rollup GROUP BY session_id), property_values AS (SELECT d.session_id, d.string_value AS value FROM session_data d JOIN filtered_sessions f ON f.session_id = d.session_id WHERE d.website_id = ",
        )
        .push_bind(website_id)
        .push(" AND d.data_key = ")
        .push_bind(property_name.to_string())
        .push(
            " AND d.data_type = 1 AND COALESCE(d.string_value, '') != '') SELECT p.value AS label, COALESCE(sum(s.activity), 0)::bigint AS activity, count(DISTINCT p.session_id)::bigint AS sessions, COALESCE(sum(s.visits), 0)::bigint AS visits, COALESCE(sum(s.views), 0)::bigint AS views, COALESCE(sum(s.events), 0)::bigint AS events FROM property_values p JOIN session_stats s ON s.session_id = p.session_id GROUP BY p.value ORDER BY activity DESC, p.value LIMIT 100",
        );
    let rows = query.build().fetch_all(&context.pool).await?;
    Ok(Value::Array(
        rows.into_iter()
            .map(|row| {
                json!({
                    "label": row.get::<String, _>("label"),
                    "activity": row.get::<i64, _>("activity"),
                    "sessions": row.get::<i64, _>("sessions"),
                    "visits": row.get::<i64, _>("visits"),
                    "views": row.get::<i64, _>("views"),
                    "events": row.get::<i64, _>("events"),
                })
            })
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;
    use sqlx::Execute;

    fn params(values: &[(&str, &str)]) -> HashMap<String, String> {
        values
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    #[test]
    fn validates_declared_controls_and_required_values() {
        let valid = params(&[
            ("startAt", "1000"),
            ("endAt", "2000"),
            ("eventName", "purchase"),
            ("propertyName", "revenue"),
            ("metric", "avg"),
            ("unit", "day"),
            ("timezone", "UTC"),
        ]);
        assert!(validate_params(&valid, Source::Event, Operation::NumericSeries).is_ok());

        let mut invalid = valid.clone();
        invalid.insert("metric".to_string(), "max".to_string());
        assert_eq!(
            validate_params(&invalid, Source::Event, Operation::NumericSeries)
                .unwrap_err()
                .status,
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn share_permissions_match_node_handlers() {
        assert_eq!(
            share_access(Source::Event, Operation::Pivot),
            (ShareSection::Events, ShareDataVisibility::Rows)
        );
        assert_eq!(
            share_access(Source::Event, Operation::NumericStats),
            (ShareSection::Events, ShareDataVisibility::Aggregate)
        );
        assert_eq!(
            share_access(Source::Session, Operation::Pivot),
            (ShareSection::Sessions, ShareDataVisibility::Aggregate)
        );
    }

    #[test]
    fn validates_typed_property_filters() {
        let numeric = params(&[("pf_seats", "2.gt.5")]);
        let parsed = parse_property_filters(&numeric).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].data_type, 2);
        assert_eq!(parsed[0].operator, Operator::Greater);

        for invalid in [
            ("pf_seats", "2.gt.nope"),
            ("pf_active", "3.eq.maybe"),
            ("pf_date", "4.bf.not-a-date"),
            ("pf_tags", "5.eq.value"),
        ] {
            assert!(parse_property_filters(&params(&[invalid])).is_err());
        }
    }

    #[test]
    fn property_values_are_bound_not_interpolated() {
        let filters = parse_property_filters(&params(&[(
            "pf_plan",
            "1.eq.pro'); drop table session_data; --",
        )]))
        .unwrap();
        let mut builder = QueryBuilder::<Postgres>::new("SELECT 1 FROM website_event e WHERE true");
        push_property_filters(
            &mut builder,
            Source::Session,
            DateRange {
                start: Utc.timestamp_millis_opt(0).single().unwrap(),
                end: Utc.timestamp_millis_opt(1).single().unwrap(),
            },
            "UTC",
            &filters,
        );
        let query = builder.build();
        assert!(!query.sql().contains("drop table"));
        assert!(query.sql().contains('$'));
    }

    #[test]
    fn pagination_and_dates_reject_invalid_ranges() {
        assert!(pagination(&params(&[("page", "0")])).is_err());
        assert!(pagination(&params(&[("page", "2"), ("pageSize", "3")]))
            .is_ok_and(|(_, _, offset, _)| offset == 3));
        assert!(validate_date_params(&params(&[("startAt", "2000"), ("endAt", "1000")])).is_err());
        assert!(validate_date_params(&params(&[("period", "7day")])).is_ok());
    }

    #[tokio::test]
    async fn postgres_property_analyses_match_contract_shapes() {
        let Ok(database_url) = std::env::var("MAAP_PROPERTIES_TEST_DATABASE_URL") else {
            return;
        };
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await
            .unwrap();
        for statement in [
            "CREATE TABLE website_event (event_id uuid PRIMARY KEY, website_id uuid NOT NULL, session_id uuid NOT NULL, visit_id uuid NOT NULL, created_at timestamptz NOT NULL, url_path varchar(500) NOT NULL, event_type integer NOT NULL, event_name varchar(50), tag varchar(50))",
            "CREATE TABLE event_data (event_data_id uuid PRIMARY KEY, website_id uuid NOT NULL, website_event_id uuid NOT NULL, data_key varchar(500) NOT NULL, string_value varchar(500), number_value numeric(19,4), date_value timestamptz, data_type integer NOT NULL, created_at timestamptz NOT NULL)",
            "CREATE TABLE session_data (session_data_id uuid PRIMARY KEY, website_id uuid NOT NULL, session_id uuid NOT NULL, data_key varchar(500) NOT NULL, string_value varchar(500), number_value numeric(19,4), date_value timestamptz, data_type integer NOT NULL, distinct_id varchar(50), created_at timestamptz NOT NULL)",
        ] {
            sqlx::query(statement).execute(&pool).await.unwrap();
        }
        let website_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let visit_id = Uuid::new_v4();
        let event_id = Uuid::new_v4();
        let created_at = Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        sqlx::query("INSERT INTO website_event VALUES ($1,$2,$3,$4,$5,'/',2,'purchase',NULL)")
            .bind(event_id)
            .bind(website_id)
            .bind(session_id)
            .bind(visit_id)
            .bind(created_at)
            .execute(&pool)
            .await
            .unwrap();
        for (key, string, number, date, data_type) in [
            ("revenue", None, Some(Decimal::new(595, 1)), None, 2),
            ("premium", Some("true"), None, None, 3),
            ("items", Some("[\"sku-1\",\"sku-2\"]"), None, None, 5),
            ("orderedAt", None, None, Some(created_at), 4),
        ] {
            sqlx::query("INSERT INTO event_data VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)")
                .bind(Uuid::new_v4())
                .bind(website_id)
                .bind(event_id)
                .bind(key)
                .bind(string)
                .bind(number)
                .bind(date)
                .bind(data_type)
                .bind(created_at)
                .execute(&pool)
                .await
                .unwrap();
        }
        for (key, string, number, date, data_type) in [
            ("plan", Some("pro"), None, None, 1),
            ("seats", None, Some(Decimal::new(6, 0)), None, 2),
            (
                "tags",
                Some("[\"early-adopter\",\"newsletter\"]"),
                None,
                None,
                5,
            ),
            ("signedUpAt", None, None, Some(created_at), 4),
        ] {
            sqlx::query("INSERT INTO session_data VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'person-1',$9)")
                .bind(Uuid::new_v4())
                .bind(website_id)
                .bind(session_id)
                .bind(key)
                .bind(string)
                .bind(number)
                .bind(date)
                .bind(data_type)
                .bind(created_at)
                .execute(&pool)
                .await
                .unwrap();
        }

        let base_context = || Context {
            pool: pool.clone(),
            range: DateRange {
                start: created_at - chrono::Duration::hours(1),
                end: created_at + chrono::Duration::hours(1),
            },
            timezone: "UTC".to_string(),
            unit: "day",
            filters: parse_filters(&HashMap::new()).unwrap(),
            audience: Default::default(),
            property_filters: vec![],
        };

        let event_stats =
            event_numeric_stats_query(&base_context(), website_id, "purchase", "revenue")
                .await
                .unwrap();
        assert_eq!(event_stats["total"], 59.5);
        let event_array = event_array_query(&base_context(), website_id, "purchase", "items")
            .await
            .unwrap();
        assert_eq!(event_array.as_array().unwrap().len(), 2);
        let event_date = event_date_query(&base_context(), website_id, "purchase", "orderedAt")
            .await
            .unwrap();
        assert_eq!(event_date[0]["y"], 1);
        let mut filtered = base_context();
        filtered.property_filters =
            parse_property_filters(&params(&[("pf_premium", "3.eq.true")])).unwrap();
        let event_page = event_pivot_query(&filtered, website_id, "purchase", 1, 20, 0, None)
            .await
            .unwrap();
        assert_eq!(event_page["count"], 1);

        let session_page =
            session_pivot_query(&base_context(), website_id, "seats", 1, 20, 0, None)
                .await
                .unwrap();
        assert_eq!(session_page["count"], 1);
        assert_eq!(
            session_array_query(&base_context(), website_id, "tags")
                .await
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            session_date_query(&base_context(), website_id, "signedUpAt")
                .await
                .unwrap()[0]["y"],
            1
        );
        assert_eq!(
            session_numeric_stats_query(&base_context(), website_id, "seats")
                .await
                .unwrap()["average"],
            6.0
        );
        assert_eq!(
            session_property_query(&base_context(), website_id, "plan")
                .await
                .unwrap()[0]["x"],
            "pro"
        );
        assert_eq!(
            session_activity_query(&base_context(), website_id, "plan")
                .await
                .unwrap()[0]["sessions"],
            1
        );
    }
}
