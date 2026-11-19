use crate::{
    access::{website_section_pool, ShareDataVisibility, ShareSection},
    audience::Audience,
    error_response, json_response, AppState,
};
use axum::{
    body::to_bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Duration, Months, NaiveDate, NaiveDateTime, TimeZone, Utc};
use serde_json::{json, Map, Value};
use sqlx::{PgPool, Postgres, QueryBuilder, Row};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq)]
struct DateRange {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
}

#[derive(Clone)]
struct Filter {
    name: &'static str,
    column: &'static str,
    operator: &'static str,
    values: Vec<String>,
}

struct Filters {
    values: Vec<Filter>,
    match_any: bool,
    needs_session: bool,
    exclude_bounce: bool,
}

pub(crate) struct ScalarFilterPredicate(Filters);

impl ScalarFilterPredicate {
    pub(crate) fn needs_session(&self) -> bool {
        self.0.needs_session
    }

    pub(crate) fn push(&self, builder: &mut QueryBuilder<'_, Postgres>, include_event_type: bool) {
        push_filters_matching(builder, &self.0, include_event_type);
    }

    pub(crate) fn push_bounce(
        &self,
        builder: &mut QueryBuilder<'_, Postgres>,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) {
        push_bounce_filter(builder, &self.0, DateRange { start, end });
    }
}

pub(crate) fn scalar_filter_predicate(
    params: &HashMap<String, String>,
) -> Result<ScalarFilterPredicate, Response> {
    parse_filters(params)
        .map(ScalarFilterPredicate)
        .map_err(|message| error_response(StatusCode::BAD_REQUEST, message))
}

pub(super) fn register_routes(router: Router<AppState>) -> Router<AppState> {
    router
        .route(
            "/api/websites/{website_id}/revenue/chart",
            get(revenue_chart),
        )
        .route(
            "/api/websites/{website_id}/revenue/metrics",
            get(revenue_metrics),
        )
        .route(
            "/api/websites/{website_id}/revenue/sessions",
            get(revenue_sessions),
        )
        .route(
            "/api/websites/{website_id}/revenue/stats",
            get(revenue_stats),
        )
        .route("/api/websites/{website_id}/retention", get(retention))
        .route("/api/websites/{website_id}/journeys", get(journeys))
        .route("/api/websites/{website_id}/attribution", get(attribution))
        .route(
            "/api/websites/{website_id}/performance/chart",
            get(performance_chart),
        )
        .route(
            "/api/websites/{website_id}/performance/metrics",
            get(performance_metrics),
        )
        .route(
            "/api/websites/{website_id}/performance/stats",
            get(performance_stats),
        )
        .route("/api/websites/{website_id}/utm/metrics", get(utm_metrics))
        .route("/api/websites/{website_id}/breakdown", get(breakdown))
        .route("/api/reports/attribution", post(report_attribution))
        .route("/api/reports/breakdown", post(report_breakdown))
        .route("/api/reports/journey", post(report_journey))
        .route("/api/reports/retention", post(report_retention))
        .route("/api/reports/revenue", post(report_revenue))
        .route("/api/reports/performance", post(report_performance))
        .route("/api/reports/utm", post(report_utm))
}

fn parse_date(value: &str) -> Option<DateTime<Utc>> {
    if let Ok(milliseconds) = value.parse::<f64>() {
        if milliseconds.is_finite() {
            return Utc.timestamp_millis_opt(milliseconds as i64).single();
        }
    }
    if let Ok(date) = DateTime::parse_from_rfc3339(value) {
        return Some(date.with_timezone(&Utc));
    }
    if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        return Some(DateTime::from_naive_utc_and_offset(
            date.and_hms_opt(0, 0, 0)?,
            Utc,
        ));
    }
    for format in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f"] {
        if let Ok(date) = NaiveDateTime::parse_from_str(value, format) {
            return Some(DateTime::from_naive_utc_and_offset(date, Utc));
        }
    }
    None
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

async fn period_range(pool: &PgPool, period: &str, timezone: &str) -> Result<DateRange, Response> {
    let timezone = normalize_timezone(timezone);
    let sql = match period {
        "24h" => {
            "SELECT CURRENT_TIMESTAMP - INTERVAL '24 hours', CURRENT_TIMESTAMP, $1::text"
        }
        "7d" => "SELECT CURRENT_TIMESTAMP - INTERVAL '7 days', CURRENT_TIMESTAMP, $1::text",
        "30d" => "SELECT CURRENT_TIMESTAMP - INTERVAL '30 days', CURRENT_TIMESTAMP, $1::text",
        "today" | "0day" => {
            "SELECT date_trunc('day', CURRENT_TIMESTAMP AT TIME ZONE $1) AT TIME ZONE $1, (date_trunc('day', CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 day' - INTERVAL '1 millisecond') AT TIME ZONE $1"
        }
        "24hour" => {
            "SELECT (date_trunc('hour', CURRENT_TIMESTAMP AT TIME ZONE $1) - INTERVAL '24 hours') AT TIME ZONE $1, (date_trunc('hour', CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 hour' - INTERVAL '1 millisecond') AT TIME ZONE $1"
        }
        "0week" => {
            "SELECT (date_trunc('week', (CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 day') - INTERVAL '1 day') AT TIME ZONE $1, (date_trunc('week', (CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 day') + INTERVAL '6 days 23 hours 59 minutes 59.999 seconds') AT TIME ZONE $1"
        }
        "7day" => {
            "SELECT (date_trunc('day', CURRENT_TIMESTAMP AT TIME ZONE $1) - INTERVAL '7 days') AT TIME ZONE $1, (date_trunc('day', CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 day' - INTERVAL '1 millisecond') AT TIME ZONE $1"
        }
        "0month" => {
            "SELECT date_trunc('month', CURRENT_TIMESTAMP AT TIME ZONE $1) AT TIME ZONE $1, (date_trunc('month', CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 month' - INTERVAL '1 millisecond') AT TIME ZONE $1"
        }
        "30day" => {
            "SELECT (date_trunc('day', CURRENT_TIMESTAMP AT TIME ZONE $1) - INTERVAL '30 days') AT TIME ZONE $1, (date_trunc('day', CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 day' - INTERVAL '1 millisecond') AT TIME ZONE $1"
        }
        "90day" => {
            "SELECT (date_trunc('day', CURRENT_TIMESTAMP AT TIME ZONE $1) - INTERVAL '90 days') AT TIME ZONE $1, (date_trunc('day', CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 day' - INTERVAL '1 millisecond') AT TIME ZONE $1"
        }
        "0year" => {
            "SELECT date_trunc('year', CURRENT_TIMESTAMP AT TIME ZONE $1) AT TIME ZONE $1, (date_trunc('year', CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 year' - INTERVAL '1 millisecond') AT TIME ZONE $1"
        }
        "6month" => {
            "SELECT (date_trunc('month', CURRENT_TIMESTAMP AT TIME ZONE $1) - INTERVAL '6 months') AT TIME ZONE $1, (date_trunc('month', CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 month' - INTERVAL '1 millisecond') AT TIME ZONE $1"
        }
        "12month" => {
            "SELECT (date_trunc('month', CURRENT_TIMESTAMP AT TIME ZONE $1) - INTERVAL '12 months') AT TIME ZONE $1, (date_trunc('month', CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 month' - INTERVAL '1 millisecond') AT TIME ZONE $1"
        }
        _ => return Err(error_response(StatusCode::BAD_REQUEST, "Invalid period")),
    };
    let row = sqlx::query(sql)
        .bind(timezone)
        .fetch_one(pool)
        .await
        .map_err(|error| error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    Ok(DateRange {
        start: row.get(0),
        end: row.get(1),
    })
}

async fn date_range(
    pool: &PgPool,
    website_id: Uuid,
    params: &HashMap<String, String>,
    allow_period: bool,
) -> Result<DateRange, Response> {
    let timezone = normalize_timezone(params.get("timezone").map(String::as_str).unwrap_or("UTC"));
    let valid_timezone = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM pg_timezone_names WHERE name = $1)",
    )
    .bind(timezone)
    .fetch_one(pool)
    .await
    .map_err(|error| error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    if !valid_timezone {
        return Err(error_response(StatusCode::BAD_REQUEST, "Invalid timezone"));
    }

    let has_explicit = ["startAt", "endAt", "startDate", "endDate"]
        .iter()
        .any(|key| params.contains_key(*key));
    let mut range = if let Some(period) = params.get("period") {
        if !allow_period || has_explicit {
            return Err(error_response(
                StatusCode::BAD_REQUEST,
                "Invalid date range",
            ));
        }
        period_range(pool, period, timezone).await?
    } else {
        let values = if params.contains_key("startAt") || params.contains_key("endAt") {
            (params.get("startAt"), params.get("endAt"))
        } else {
            (params.get("startDate"), params.get("endDate"))
        };
        let (Some(start), Some(end)) = values else {
            return Err(error_response(
                StatusCode::BAD_REQUEST,
                "A complete date range is required",
            ));
        };
        let (Some(start), Some(end)) = (parse_date(start), parse_date(end)) else {
            return Err(error_response(
                StatusCode::BAD_REQUEST,
                "Invalid date range",
            ));
        };
        DateRange { start, end }
    };
    if range.start > range.end {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "Invalid date range",
        ));
    }
    if let Some(reset_at) = sqlx::query_scalar::<_, Option<DateTime<Utc>>>(
        "SELECT reset_at FROM website WHERE website_id = $1",
    )
    .bind(website_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
    .flatten()
    {
        range.start = range.start.max(reset_at);
    }
    Ok(range)
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

fn parse_filters(params: &HashMap<String, String>) -> Result<Filters, String> {
    if params.keys().any(|key| {
        key == "segment"
            || key == "cohort"
            || ["epf", "spf"].iter().any(|prefix| {
                key.strip_prefix(prefix).is_some_and(|index| {
                    !index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit())
                })
            })
    }) {
        return Err("Audience filters must be resolved before parsing scalar filters".to_string());
    }

    let mut values = Vec::new();
    let mut has_tag = false;
    let mut needs_session = false;
    for (key, value) in params {
        let base = key.trim_end_matches(|c: char| c.is_ascii_digit());
        let Some((name, column, session)) = filter_column(base) else {
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
        let items = if matches!(operator, "eq" | "neq") {
            raw.split(',').map(str::to_string).collect()
        } else {
            vec![raw.to_string()]
        };
        if name == "eventType"
            && (operator != "eq"
                || items.len() != 1
                || items[0]
                    .parse::<i32>()
                    .ok()
                    .filter(|value| *value > 0)
                    .is_none())
        {
            return Err("Invalid event type filter".to_string());
        }
        has_tag |= name == "tag";
        needs_session |= session;
        values.push(Filter {
            name,
            column,
            operator,
            values: items,
        });
    }
    if !has_tag {
        values.push(Filter {
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
        values,
        match_any: params.get("match").map(String::as_str) == Some("any"),
        needs_session,
        exclude_bounce: params
            .get("excludeBounce")
            .is_some_and(|value| !value.is_empty()),
    })
}

fn validate_params(params: &HashMap<String, String>, controls: &[&str]) -> Result<(), Response> {
    for key in params.keys() {
        let base = key.trim_end_matches(|c: char| c.is_ascii_digit());
        if filter_column(base).is_some()
            || controls.contains(&key.as_str())
            || key == "testMode"
            || key == "match"
            || key == "excludeBounce"
        {
            continue;
        }
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            format!("Unsupported query parameter: {key}"),
        ));
    }
    Ok(())
}

fn push_filter(builder: &mut QueryBuilder<'_, Postgres>, filter: &Filter) {
    if filter.name == "eventType" {
        builder.push(filter.column).push(" = ").push_bind(
            filter.values[0]
                .parse::<i32>()
                .expect("validated event type"),
        );
        return;
    }
    match filter.operator {
        "eq" => {
            builder
                .push(filter.column)
                .push(" = ANY(")
                .push_bind(filter.values.clone())
                .push(")");
        }
        "neq" => {
            builder
                .push("COALESCE(")
                .push(filter.column)
                .push(", '') != ALL(")
                .push_bind(filter.values.clone())
                .push(")");
        }
        "s" => {
            builder
                .push(filter.column)
                .push(" ILIKE ")
                .push_bind(format!("{}%", filter.values[0]));
        }
        "ns" => {
            builder
                .push("COALESCE(")
                .push(filter.column)
                .push(", '') NOT ILIKE ")
                .push_bind(format!("{}%", filter.values[0]));
        }
        "c" => {
            builder
                .push(filter.column)
                .push(" ILIKE ")
                .push_bind(format!("%{}%", filter.values[0]));
        }
        "dnc" => {
            builder
                .push("COALESCE(")
                .push(filter.column)
                .push(", '') NOT ILIKE ")
                .push_bind(format!("%{}%", filter.values[0]));
        }
        "re" => {
            builder
                .push(filter.column)
                .push(" ~* ")
                .push_bind(filter.values[0].clone());
        }
        "nre" => {
            builder
                .push("COALESCE(")
                .push(filter.column)
                .push(", '') !~* ")
                .push_bind(filter.values[0].clone());
        }
        _ => unreachable!(),
    }
}

fn push_filters_matching(
    builder: &mut QueryBuilder<'_, Postgres>,
    filters: &Filters,
    include_event_type: bool,
) {
    let selected = filters
        .values
        .iter()
        .filter(|filter| include_event_type || filter.name != "eventType")
        .collect::<Vec<_>>();
    let (always, matched): (Vec<_>, Vec<_>) = selected
        .iter()
        .copied()
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
    if selected.iter().any(|filter| filter.name == "referrer") {
        builder.push(
            " AND (e.referrer_domain != regexp_replace(e.hostname, '^www.', '') OR e.referrer_domain IS NULL)",
        );
    }
}

fn push_filters(builder: &mut QueryBuilder<'_, Postgres>, filters: &Filters) {
    push_filters_matching(builder, filters, true);
}

fn push_bounce_filter(
    builder: &mut QueryBuilder<'_, Postgres>,
    filters: &Filters,
    range: DateRange,
) {
    if filters.exclude_bounce {
        builder
            .push(" AND EXISTS (SELECT 1 FROM website_event bx WHERE bx.website_id = e.website_id AND bx.session_id = e.session_id AND bx.visit_id = e.visit_id AND bx.created_at BETWEEN ")
            .push_bind(range.start)
            .push(" AND ")
            .push_bind(range.end)
            .push(" AND bx.event_type != 5 GROUP BY bx.session_id, bx.visit_id HAVING count(*) FILTER (WHERE bx.event_type NOT IN (2, 5)) > 1 OR (count(*) FILTER (WHERE bx.event_type NOT IN (2, 5)) = 1 AND count(*) FILTER (WHERE bx.event_type = 2) > 0))");
    }
}

fn push_insight_filters(
    builder: &mut QueryBuilder<'_, Postgres>,
    filters: &Filters,
    audience: &Audience,
    range: DateRange,
) {
    push_filters(builder, filters);
    audience.push(builder, "e", range.start, range.end);
    push_bounce_filter(builder, filters, range);
}

fn push_filtered_sessions(
    builder: &mut QueryBuilder<'_, Postgres>,
    website_id: Uuid,
    range: DateRange,
    filters: &Filters,
    audience: &Audience,
) {
    builder.push("filtered_sessions AS (SELECT DISTINCT e.session_id FROM website_event e ");
    if filters.needs_session {
        builder
            .push("JOIN session s ON s.session_id = e.session_id AND s.website_id = e.website_id ");
    }
    builder
        .push("WHERE e.website_id = ")
        .push_bind(website_id)
        .push(" AND e.created_at BETWEEN ")
        .push_bind(range.start)
        .push(" AND ")
        .push_bind(range.end)
        .push(" AND e.event_type != 5");
    push_insight_filters(builder, filters, audience, range);
    builder.push(")");
}

async fn context(
    state: &AppState,
    headers: &HeaderMap,
    website_id: Uuid,
    params: &HashMap<String, String>,
    controls: &[&str],
    allow_period: bool,
    section: ShareSection,
    visibility: ShareDataVisibility,
) -> Result<(PgPool, DateRange, Filters, Audience), Response> {
    let pool = website_section_pool(state, headers, website_id, &[section], visibility).await?;
    let mut params = params.clone();
    let audience = Audience::resolve(&pool, website_id, &mut params).await?;
    params.retain(|key, _| !key.starts_with("pf_") && key != "epf" && key != "spf");
    validate_params(&params, controls)?;
    if params.get("unit").is_some_and(|value| {
        !matches!(value.as_str(), "minute" | "hour" | "day" | "month" | "year")
    }) {
        return Err(error_response(StatusCode::BAD_REQUEST, "Invalid unit"));
    }
    if params
        .get("compare")
        .is_some_and(|value| !matches!(value.as_str(), "prev" | "yoy"))
    {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "Invalid comparison",
        ));
    }
    if params
        .get("match")
        .is_some_and(|value| !matches!(value.as_str(), "all" | "any"))
    {
        return Err(error_response(StatusCode::BAD_REQUEST, "Invalid match"));
    }
    let filters = parse_filters(&params)
        .map_err(|message| error_response(StatusCode::BAD_REQUEST, message))?;
    let range = date_range(&pool, website_id, &params, allow_period).await?;
    Ok((pool, range, filters, audience))
}

fn database_error(error: sqlx::Error) -> Response {
    error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}

fn filter_scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

pub(crate) fn report_filter_params(
    value: Option<&Value>,
) -> Result<HashMap<String, String>, Response> {
    let object = value
        .and_then(Value::as_object)
        .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, "filters must be an object"))?;
    Ok(object
        .iter()
        .filter_map(|(key, value)| filter_scalar(value).map(|value| (key.clone(), value)))
        .collect())
}

async fn revenue_chart(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let Some(currency) = params.get("currency").filter(|value| !value.is_empty()) else {
        return error_response(StatusCode::BAD_REQUEST, "currency is required");
    };
    let unit = match params.get("unit").map(String::as_str).unwrap_or("day") {
        "minute" => "minute",
        "hour" => "hour",
        "day" => "day",
        "month" => "month",
        "year" => "year",
        _ => return error_response(StatusCode::BAD_REQUEST, "Invalid unit"),
    };
    let (pool, range, filters, audience) = match context(
        &state,
        &headers,
        website_id,
        &params,
        &[
            "startAt",
            "endAt",
            "startDate",
            "endDate",
            "period",
            "timezone",
            "unit",
            "compare",
            "currency",
        ],
        true,
        ShareSection::Revenue,
        ShareDataVisibility::Aggregate,
    )
    .await
    {
        Ok(context) => context,
        Err(response) => return response,
    };
    let timezone = normalize_timezone(params.get("timezone").map(String::as_str).unwrap_or("UTC"));
    let mut builder = QueryBuilder::<Postgres>::new("WITH ");
    push_filtered_sessions(&mut builder, website_id, range, &filters, &audience);
    builder
        .push(
            ", filtered_revenue AS (SELECT r.event_name, r.created_at, r.revenue FROM revenue r JOIN filtered_sessions f ON f.session_id = r.session_id WHERE r.website_id = ",
        )
        .push_bind(website_id)
        .push(" AND r.created_at BETWEEN ")
        .push_bind(range.start)
        .push(" AND ")
        .push_bind(range.end)
        .push(" AND upper(r.currency) = upper(")
        .push_bind(currency.to_string())
        .push(")) SELECT event_name AS x, to_char(date_trunc('")
        .push(unit)
        .push("', created_at AT TIME ZONE ")
        .push_bind(timezone)
        .push("), 'YYYY-MM-DD HH24:MI:SS') AS t, COALESCE(sum(revenue), 0)::float8 AS y, count(*) FILTER (WHERE revenue > 0)::bigint AS count FROM filtered_revenue GROUP BY 1, 2 ORDER BY 2");
    match builder.build().fetch_all(&pool).await {
        Ok(rows) => json_response(json!({
            "chart": rows.into_iter().map(|row| json!({
                "x": row.get::<String, _>("x"),
                "t": row.get::<String, _>("t"),
                "y": row.get::<f64, _>("y"),
                "count": row.get::<i64, _>("count"),
            })).collect::<Vec<_>>()
        })),
        Err(error) => database_error(error),
    }
}

fn push_revenue_ctes(
    builder: &mut QueryBuilder<'_, Postgres>,
    website_id: Uuid,
    range: DateRange,
    currency: &str,
    filters: &Filters,
    audience: &Audience,
) {
    push_filtered_sessions(builder, website_id, range, filters, audience);
    builder
        .push(", filtered_revenue AS (SELECT r.website_id, r.session_id, r.revenue FROM revenue r JOIN filtered_sessions f ON f.session_id = r.session_id WHERE r.website_id = ")
        .push_bind(website_id)
        .push(" AND r.created_at BETWEEN ")
        .push_bind(range.start)
        .push(" AND ")
        .push_bind(range.end)
        .push(" AND upper(r.currency) = upper(")
        .push_bind(currency.to_string())
        .push("))");
}

fn channel_case() -> &'static str {
    r#"
    CASE
      WHEN COALESCE(referrer_domain, '') = '' AND COALESCE(url_query, '') = '' THEN 'direct'
      WHEN lower(COALESCE(url_query, '')) ~ '(ad_id=|aid=|dclid=|epik=|gclid=|li_fat_id=|msclkid=|ob_click_id=|pc_id=|rdt_cid=|scid=|ttclid=|twclid=|utm_medium=cpc|utm_medium=paid|utm_medium=paid_social|utm_source=google)' THEN 'paidAds'
      WHEN lower(COALESCE(utm_medium, '')) ~ '(referral|app|link)' THEN 'referral'
      WHEN lower(COALESCE(utm_medium, '')) LIKE '%affiliate%' THEN 'affiliate'
      WHEN lower(COALESCE(utm_medium, '')) LIKE '%sms%' OR lower(COALESCE(utm_source, '')) LIKE '%sms%' THEN 'sms'
      WHEN lower(COALESCE(referrer_domain, '')) ~ '(chatgpt\.com|claude\.ai|copilot\.microsoft\.com|gemini\.google\.com|meta\.ai|perplexity\.ai)' THEN 'llm'
      WHEN lower(COALESCE(referrer_domain, '')) ~ '(baidu\.com|bing\.com|duckduckgo\.com|ecosia\.org|google\.|msn\.com|search\.brave\.com|yandex\.)' OR lower(COALESCE(utm_medium, '')) LIKE '%organic%' THEN prefix || 'Search'
      WHEN lower(COALESCE(referrer_domain, '')) ~ '(bsky\.app|facebook\.com|fb\.com|ig\.com|instagram\.com|linkedin\.|news\.ycombinator\.com|pinterest\.|reddit\.|snapchat\.|t\.co|threads\.net|tiktok\.|twitter\.com|x\.com)' THEN prefix || 'Social'
      WHEN lower(COALESCE(referrer_domain, '')) ~ '(gmail\.|hotmail\.|mail\.yahoo\.|outlook\.|proton\.me|protonmail\.)' OR lower(COALESCE(utm_medium, '')) LIKE '%mail%' THEN 'email'
      WHEN lower(COALESCE(referrer_domain, '')) ~ '(alibaba\.com|aliexpress\.com|amazon\.|bestbuy\.com|ebay\.com|etsy\.com|newegg\.com|target\.com|walmart\.com)' OR lower(COALESCE(utm_medium, '')) LIKE '%shop%' THEN prefix || 'Shopping'
      WHEN lower(COALESCE(referrer_domain, '')) ~ '(twitch\.|youtube\.)' OR lower(COALESCE(utm_medium, '')) LIKE '%video%' THEN prefix || 'Video'
      WHEN referrer_domain != regexp_replace(hostname, '^www.', '') AND referrer_domain != '' THEN 'referral'
      ELSE 'unknown'
    END
    "#
}

async fn revenue_metrics(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let Some(currency) = params.get("currency").filter(|value| !value.is_empty()) else {
        return error_response(StatusCode::BAD_REQUEST, "currency is required");
    };
    let kind = match params.get("type").map(String::as_str) {
        Some(value @ ("country" | "region" | "referrer" | "channel")) => value,
        _ => return error_response(StatusCode::BAD_REQUEST, "Invalid metric type"),
    };
    let (pool, range, filters, audience) = match context(
        &state,
        &headers,
        website_id,
        &params,
        &[
            "startAt",
            "endAt",
            "startDate",
            "endDate",
            "period",
            "timezone",
            "unit",
            "compare",
            "currency",
            "type",
        ],
        true,
        ShareSection::Revenue,
        ShareDataVisibility::Aggregate,
    )
    .await
    {
        Ok(context) => context,
        Err(response) => return response,
    };
    let mut builder = QueryBuilder::<Postgres>::new("WITH ");
    push_revenue_ctes(
        &mut builder,
        website_id,
        range,
        currency,
        &filters,
        &audience,
    );
    match kind {
        "country" => builder.push(
            " SELECT s.country AS name, COALESCE(sum(r.revenue), 0)::float8 AS value FROM filtered_revenue r JOIN session s ON s.website_id = r.website_id AND s.session_id = r.session_id GROUP BY 1 ORDER BY 2 DESC",
        ),
        "region" => builder.push(
            " SELECT s.country, s.region AS name, COALESCE(sum(r.revenue), 0)::float8 AS value FROM filtered_revenue r JOIN session s ON s.website_id = r.website_id AND s.session_id = r.session_id GROUP BY 1, 2 ORDER BY 3 DESC",
        ),
        "referrer" => builder.push(
            ", session_revenue AS (SELECT session_id, sum(revenue) AS value FROM filtered_revenue GROUP BY 1), first_event AS (SELECT DISTINCT ON (e.session_id) e.session_id, e.referrer_domain FROM website_event e JOIN session_revenue r ON r.session_id = e.session_id WHERE e.website_id = "
        ).push_bind(website_id).push(" AND e.created_at BETWEEN ").push_bind(range.start).push(" AND ").push_bind(range.end).push(
            " ORDER BY e.session_id, e.created_at, e.event_id) SELECT f.referrer_domain AS name, sum(r.value)::float8 AS value FROM session_revenue r LEFT JOIN first_event f ON f.session_id = r.session_id GROUP BY 1 ORDER BY 2 DESC"
        ),
        "channel" => builder.push(
            ", session_revenue AS (SELECT session_id, sum(revenue) AS value FROM filtered_revenue GROUP BY 1), first_event AS (SELECT DISTINCT ON (e.session_id) e.session_id, e.referrer_domain, e.url_query, e.utm_medium, e.utm_source, e.hostname FROM website_event e JOIN session_revenue r ON r.session_id = e.session_id WHERE e.website_id = "
        ).push_bind(website_id).push(" AND e.created_at BETWEEN ").push_bind(range.start).push(" AND ").push_bind(range.end).push(
            " ORDER BY e.session_id, e.created_at, e.event_id), classified AS (SELECT r.value, CASE WHEN lower(COALESCE(f.utm_medium, '')) ~ '(cp|ppc|retargeting|paid)' THEN 'paid' ELSE 'organic' END AS prefix, f.* FROM session_revenue r LEFT JOIN first_event f ON f.session_id = r.session_id) SELECT "
        ).push(channel_case()).push(
            " AS name, sum(value)::float8 AS value FROM classified GROUP BY 1 ORDER BY 2 DESC"
        ),
        _ => unreachable!(),
    };
    match builder.build().fetch_all(&pool).await {
        Ok(rows) => json_response(Value::Array(
            rows.into_iter()
                .map(|row| {
                    let mut value = json!({
                        "name": row.get::<Option<String>, _>("name"),
                        "value": row.get::<f64, _>("value"),
                    });
                    if kind == "region" {
                        value["country"] = json!(row.get::<Option<String>, _>("country"));
                    }
                    value
                })
                .collect(),
        )),
        Err(error) => database_error(error),
    }
}

fn positive_param(
    params: &HashMap<String, String>,
    name: &str,
    default: i64,
) -> Result<i64, Response> {
    params
        .get(name)
        .map(|value| value.parse::<i64>())
        .transpose()
        .ok()
        .flatten()
        .filter(|value| *value > 0)
        .or_else(|| (!params.contains_key(name)).then_some(default))
        .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, format!("Invalid {name}")))
}

async fn revenue_sessions(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let Some(currency) = params.get("currency").filter(|value| !value.is_empty()) else {
        return error_response(StatusCode::BAD_REQUEST, "currency is required");
    };
    let page = match positive_param(&params, "page", 1) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let page_size = match positive_param(&params, "pageSize", 20) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let max_results = match positive_param(&params, "maxResults", i64::MAX) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let (pool, range, filters, audience) = match context(
        &state,
        &headers,
        website_id,
        &params,
        &[
            "startAt",
            "endAt",
            "startDate",
            "endDate",
            "period",
            "timezone",
            "unit",
            "compare",
            "currency",
            "page",
            "pageSize",
            "maxResults",
            "search",
        ],
        true,
        ShareSection::Revenue,
        ShareDataVisibility::Rows,
    )
    .await
    {
        Ok(context) => context,
        Err(response) => return response,
    };
    let offset = match (page - 1).checked_mul(page_size) {
        Some(value) => value,
        None => return error_response(StatusCode::BAD_REQUEST, "Invalid page"),
    };
    let limit = page_size.min(max_results.saturating_sub(offset).max(0));
    let mut builder = QueryBuilder::<Postgres>::new("WITH ");
    push_filtered_sessions(&mut builder, website_id, range, &filters, &audience);
    builder
        .push(", revenue_sessions AS (SELECT DISTINCT session_id FROM revenue WHERE website_id = ")
        .push_bind(website_id)
        .push(" AND created_at BETWEEN ")
        .push_bind(range.start)
        .push(" AND ")
        .push_bind(range.end)
        .push(" AND upper(currency) = upper(")
        .push_bind(currency)
        .push(")), grouped AS (SELECT s.session_id AS id, s.website_id, max(e.hostname) AS hostname, s.browser, s.os, s.device, s.screen, s.language, s.country, s.region, s.city, (array_agg(e.referrer_domain ORDER BY e.created_at, e.event_id))[1] AS referrer_domain, min(e.created_at) AS first_at, max(e.created_at) AS last_at, count(DISTINCT e.visit_id)::bigint AS visits, count(*) FILTER (WHERE e.event_type = 1)::bigint AS views, count(*) FILTER (WHERE e.event_type = 2)::bigint AS events FROM website_event e JOIN session s ON s.session_id = e.session_id AND s.website_id = e.website_id JOIN filtered_sessions f ON f.session_id = e.session_id JOIN revenue_sessions r ON r.session_id = e.session_id WHERE e.website_id = ")
        .push_bind(website_id)
        .push(" AND e.created_at BETWEEN ")
        .push_bind(range.start)
        .push(" AND ")
        .push_bind(range.end)
        .push(" AND e.event_type != 5");
    push_insight_filters(&mut builder, &filters, &audience, range);
    if let Some(search) = params.get("search") {
        builder
            .push(" AND (s.browser ILIKE ")
            .push_bind(format!("%{search}%"))
            .push(" OR s.os ILIKE ")
            .push_bind(format!("%{search}%"))
            .push(" OR s.device ILIKE ")
            .push_bind(format!("%{search}%"))
            .push(" OR s.city ILIKE ")
            .push_bind(format!("%{search}%"))
            .push(")");
    }
    builder.push(" GROUP BY s.session_id, s.website_id, s.browser, s.os, s.device, s.screen, s.language, s.country, s.region, s.city), counted AS (SELECT count(*)::bigint AS total_count FROM grouped) SELECT p.*, c.total_count FROM counted c LEFT JOIN LATERAL (SELECT * FROM grouped ORDER BY last_at DESC LIMIT ")
        .push_bind(limit)
        .push(" OFFSET ")
        .push_bind(offset)
        .push(") p ON true");
    match builder.build().fetch_all(&pool).await {
        Ok(rows) => {
            let count = rows
                .first()
                .map(|row| row.get::<i64, _>("total_count").min(max_results))
                .unwrap_or(0);
            let data = rows
                .into_iter()
                .filter_map(|row| {
                    let id = row.get::<Option<Uuid>, _>("id")?;
                    Some(json!({
                        "id": id,
                        "websiteId": row.get::<Uuid, _>("website_id"),
                        "hostname": row.get::<Option<String>, _>("hostname"),
                        "browser": row.get::<Option<String>, _>("browser"),
                        "os": row.get::<Option<String>, _>("os"),
                        "device": row.get::<Option<String>, _>("device"),
                        "screen": row.get::<Option<String>, _>("screen"),
                        "language": row.get::<Option<String>, _>("language"),
                        "country": row.get::<Option<String>, _>("country"),
                        "region": row.get::<Option<String>, _>("region"),
                        "city": row.get::<Option<String>, _>("city"),
                        "referrerDomain": row.get::<Option<String>, _>("referrer_domain"),
                        "firstAt": row.get::<Option<DateTime<Utc>>, _>("first_at"),
                        "lastAt": row.get::<Option<DateTime<Utc>>, _>("last_at"),
                        "visits": row.get::<i64, _>("visits"),
                        "views": row.get::<i64, _>("views"),
                        "events": row.get::<i64, _>("events"),
                        "createdAt": row.get::<Option<DateTime<Utc>>, _>("last_at"),
                    }))
                })
                .collect::<Vec<_>>();
            json_response(json!({
                "data": data,
                "count": count,
                "page": page,
                "pageSize": page_size,
            }))
        }
        Err(error) => database_error(error),
    }
}

fn comparison_range(range: DateRange, compare: &str, now: DateTime<Utc>) -> DateRange {
    let length = range.end - range.start + Duration::milliseconds(1);
    let compared = if now < range.end {
        Duration::hours(((now - range.start).num_hours()).max(0))
    } else {
        length
    };
    if compare == "yoy" {
        let start = range
            .start
            .checked_sub_months(Months::new(12))
            .unwrap_or(range.start);
        let end = (range.start + compared - Duration::milliseconds(1))
            .checked_sub_months(Months::new(12))
            .unwrap_or(range.start + compared - Duration::milliseconds(1));
        DateRange { start, end }
    } else {
        let start = range.start - length;
        DateRange {
            start,
            end: start + compared - Duration::milliseconds(1),
        }
    }
}

async fn revenue_total(
    pool: &PgPool,
    website_id: Uuid,
    range: DateRange,
    currency: &str,
    filters: &Filters,
    audience: &Audience,
) -> Result<Value, sqlx::Error> {
    let mut builder = QueryBuilder::<Postgres>::new("WITH ");
    push_filtered_sessions(&mut builder, website_id, range, filters, audience);
    builder
        .push(", filtered_revenue AS (SELECT r.session_id, r.revenue FROM revenue r JOIN filtered_sessions f ON f.session_id = r.session_id WHERE r.website_id = ")
        .push_bind(website_id)
        .push(" AND r.created_at BETWEEN ")
        .push_bind(range.start)
        .push(" AND ")
        .push_bind(range.end)
        .push(" AND upper(r.currency) = upper(")
        .push_bind(currency)
        .push(")) SELECT COALESCE(sum(revenue), 0)::float8 AS sum, count(*) FILTER (WHERE revenue > 0)::bigint AS count, count(DISTINCT session_id)::bigint AS unique_count, (SELECT count(*)::bigint FROM filtered_sessions) AS total_sessions FROM filtered_revenue");
    let row = builder.build().fetch_one(pool).await?;
    let sum = row.get::<f64, _>("sum");
    let count = row.get::<i64, _>("count");
    let unique_count = row.get::<i64, _>("unique_count");
    let total_sessions = row.get::<i64, _>("total_sessions");
    Ok(json!({
        "sum": sum,
        "count": count,
        "average": if count > 0 { sum / count as f64 } else { 0.0 },
        "unique_count": unique_count,
        "total_sessions": total_sessions,
        "arpu": if total_sessions > 0 { sum / total_sessions as f64 } else { 0.0 },
    }))
}

async fn revenue_stats(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let Some(currency) = params.get("currency").filter(|value| !value.is_empty()) else {
        return error_response(StatusCode::BAD_REQUEST, "currency is required");
    };
    let compare = params.get("compare").map(String::as_str).unwrap_or("prev");
    if !matches!(compare, "prev" | "yoy") {
        return error_response(StatusCode::BAD_REQUEST, "Invalid comparison");
    }
    let (pool, range, filters, audience) = match context(
        &state,
        &headers,
        website_id,
        &params,
        &[
            "startAt",
            "endAt",
            "startDate",
            "endDate",
            "period",
            "timezone",
            "unit",
            "compare",
            "currency",
        ],
        true,
        ShareSection::Revenue,
        ShareDataVisibility::Aggregate,
    )
    .await
    {
        Ok(context) => context,
        Err(response) => return response,
    };
    let current = match revenue_total(&pool, website_id, range, currency, &filters, &audience).await
    {
        Ok(value) => value,
        Err(error) => return database_error(error),
    };
    let comparison = match revenue_total(
        &pool,
        website_id,
        comparison_range(range, compare, Utc::now()),
        currency,
        &filters,
        &audience,
    )
    .await
    {
        Ok(value) => value,
        Err(error) => return database_error(error),
    };
    let mut output = current.as_object().cloned().unwrap_or_default();
    output.insert("comparison".to_string(), comparison);
    json_response(Value::Object(output))
}

async fn retention(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let (pool, range, filters, audience) = match context(
        &state,
        &headers,
        website_id,
        &params,
        &["startAt", "endAt", "timezone", "unit"],
        false,
        ShareSection::Retention,
        ShareDataVisibility::Aggregate,
    )
    .await
    {
        Ok(context) => context,
        Err(response) => return response,
    };
    let timezone = normalize_timezone(params.get("timezone").map(String::as_str).unwrap_or("UTC"));
    let mut builder = QueryBuilder::<Postgres>::new(
        "WITH cohort_items AS (SELECT min(date_trunc('day', e.created_at AT TIME ZONE ",
    );
    builder
        .push_bind(timezone)
        .push("))::date AS cohort_date, e.session_id FROM website_event e ");
    if filters.needs_session {
        builder
            .push("JOIN session s ON s.session_id = e.session_id AND s.website_id = e.website_id ");
    }
    builder
        .push("WHERE e.website_id = ")
        .push_bind(website_id)
        .push(" AND e.created_at BETWEEN ")
        .push_bind(range.start)
        .push(" AND ")
        .push_bind(range.end);
    push_insight_filters(&mut builder, &filters, &audience, range);
    builder
        .push(" GROUP BY e.session_id), activities AS (SELECT DISTINCT e.session_id, (date_trunc('day', e.created_at AT TIME ZONE ")
        .push_bind(timezone)
        .push(")::date - c.cohort_date)::int AS day FROM website_event e JOIN cohort_items c ON c.session_id = e.session_id WHERE e.website_id = ")
        .push_bind(website_id)
        .push(" AND e.created_at BETWEEN ")
        .push_bind(range.start)
        .push(" AND ")
        .push_bind(range.end)
        .push("), cohort_size AS (SELECT cohort_date, count(*)::bigint AS visitors FROM cohort_items GROUP BY 1), cohort_days AS (SELECT c.cohort_date, a.day, count(*)::bigint AS return_visitors FROM activities a JOIN cohort_items c ON c.session_id = a.session_id GROUP BY 1, 2) SELECT to_char(d.cohort_date, 'YYYY-MM-DD') AS date, d.day, s.visitors, d.return_visitors, d.return_visitors::float8 * 100 / s.visitors AS percentage FROM cohort_days d JOIN cohort_size s ON s.cohort_date = d.cohort_date WHERE d.day <= 31 ORDER BY 1, 2");
    match builder.build().fetch_all(&pool).await {
        Ok(rows) => json_response(Value::Array(
            rows.into_iter()
                .map(|row| {
                    json!({
                        "date": row.get::<String, _>("date"),
                        "day": row.get::<i32, _>("day"),
                        "visitors": row.get::<i64, _>("visitors"),
                        "returnVisitors": row.get::<i64, _>("return_visitors"),
                        "percentage": row.get::<f64, _>("percentage"),
                    })
                })
                .collect(),
        )),
        Err(error) => database_error(error),
    }
}

fn combine_duplicates(items: Vec<Value>) -> Vec<Value> {
    items.into_iter().fold(Vec::new(), |mut result, item| {
        if result.last() != Some(&item) {
            result.push(item);
        }
        result
    })
}

async fn journeys(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let steps = match params
        .get("steps")
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| (2..=7).contains(value))
    {
        Some(value) => value,
        None => return error_response(StatusCode::BAD_REQUEST, "Invalid steps"),
    };
    let (pool, range, filters, audience) = match context(
        &state,
        &headers,
        website_id,
        &params,
        &[
            "startAt",
            "endAt",
            "timezone",
            "unit",
            "steps",
            "startStep",
            "endStep",
        ],
        false,
        ShareSection::Journeys,
        ShareDataVisibility::Aggregate,
    )
    .await
    {
        Ok(context) => context,
        Err(response) => return response,
    };
    let mut builder = QueryBuilder::<Postgres>::new(
        "WITH events AS (SELECT e.visit_id, COALESCE(NULLIF(e.event_name, ''), e.url_path) AS event, row_number() OVER (PARTITION BY e.visit_id ORDER BY e.created_at, e.event_id) AS event_number FROM website_event e ",
    );
    if filters.needs_session {
        builder
            .push("JOIN session s ON s.session_id = e.session_id AND s.website_id = e.website_id ");
    }
    builder
        .push("WHERE e.website_id = ")
        .push_bind(website_id)
        .push(" AND e.created_at BETWEEN ")
        .push_bind(range.start)
        .push(" AND ")
        .push_bind(range.end);
    push_insight_filters(&mut builder, &filters, &audience, range);
    builder
        .push("), sequences AS (SELECT visit_id, array_agg(event ORDER BY event_number) FILTER (WHERE event_number <= ")
        .push_bind(steps)
        .push(") AS items FROM events GROUP BY visit_id) SELECT items, count(*)::bigint AS count FROM sequences WHERE cardinality(items) > 0");
    if let Some(start_step) = params.get("startStep") {
        builder.push(" AND items[1] = ").push_bind(start_step);
    }
    if let Some(end_step) = params.get("endStep") {
        builder
            .push(" AND items[cardinality(items)] = ")
            .push_bind(end_step);
    }
    builder.push(" GROUP BY items ORDER BY count DESC LIMIT 100");
    match builder.build().fetch_all(&pool).await {
        Ok(rows) => json_response(Value::Array(
            rows.into_iter()
                .map(|row| {
                    let mut items = row
                        .get::<Vec<String>, _>("items")
                        .into_iter()
                        .map(Value::String)
                        .collect::<Vec<_>>();
                    if items.len() < steps as usize {
                        items.push(Value::Null);
                    }
                    json!({
                        "items": combine_duplicates(items),
                        "count": row.get::<i64, _>("count"),
                    })
                })
                .collect(),
        )),
        Err(error) => database_error(error),
    }
}

fn push_attribution_base(
    builder: &mut QueryBuilder<'_, Postgres>,
    website_id: Uuid,
    range: DateRange,
    filters: &Filters,
    audience: &Audience,
    target_column: &'static str,
    target_type: i32,
    step: &str,
    first_click: bool,
) {
    builder.push("WITH conversions AS (SELECT e.session_id, max(e.created_at) AS max_dt FROM website_event e ");
    if filters.needs_session {
        builder
            .push("JOIN session s ON s.session_id = e.session_id AND s.website_id = e.website_id ");
    }
    builder
        .push("WHERE e.website_id = ")
        .push_bind(website_id)
        .push(" AND e.created_at BETWEEN ")
        .push_bind(range.start)
        .push(" AND ")
        .push_bind(range.end)
        .push(" AND e.event_type = ")
        .push_bind(target_type)
        .push(" AND ")
        .push(target_column)
        .push(" = ")
        .push_bind(step.to_string());
    push_insight_filters(builder, filters, audience, range);
    builder.push(" GROUP BY e.session_id), model AS (SELECT DISTINCT ON (e.session_id) e.* FROM conversions c JOIN website_event e ON e.session_id = c.session_id WHERE e.website_id = ")
        .push_bind(website_id)
        .push(" AND e.created_at BETWEEN ")
        .push_bind(range.start)
        .push(" AND ")
        .push_bind(range.end);
    if !first_click {
        builder.push(" AND e.created_at < c.max_dt");
    }
    builder
        .push(" ORDER BY e.session_id, e.created_at ")
        .push(if first_click { "ASC" } else { "DESC" })
        .push(", e.event_id ")
        .push(if first_click { "ASC" } else { "DESC" })
        .push(")");
}

async fn attribution(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let first_click = match params.get("model").map(String::as_str) {
        Some("first-click") => true,
        Some("last-click") => false,
        _ => return error_response(StatusCode::BAD_REQUEST, "Invalid attribution model"),
    };
    let (target_column, target_type) = match params.get("type").map(String::as_str) {
        Some("path") => ("e.url_path", 1),
        Some("event") => ("e.event_name", 2),
        _ => return error_response(StatusCode::BAD_REQUEST, "Invalid attribution type"),
    };
    let Some(step) = params.get("step") else {
        return error_response(StatusCode::BAD_REQUEST, "step is required");
    };
    if params
        .get("eventType")
        .and_then(|value| value.parse::<i32>().ok())
        .is_some_and(|value| value != target_type)
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            "eventType conflicts with attribution type",
        );
    }
    let (pool, range, filters, audience) = match context(
        &state,
        &headers,
        website_id,
        &params,
        &[
            "startAt", "endAt", "timezone", "unit", "model", "type", "step", "currency",
        ],
        false,
        ShareSection::Attribution,
        ShareDataVisibility::Aggregate,
    )
    .await
    {
        Ok(context) => context,
        Err(response) => return response,
    };

    let mut builder = QueryBuilder::<Postgres>::new("");
    push_attribution_base(
        &mut builder,
        website_id,
        range,
        &filters,
        &audience,
        target_column,
        target_type,
        step,
        first_click,
    );
    builder.push(
        r#",
        dimensions AS (
          SELECT 'referrer'::text AS dimension, referrer_domain AS name, session_id
          FROM model
          WHERE COALESCE(referrer_domain, '') != ''
            AND referrer_domain != regexp_replace(hostname, '^www.', '')
          UNION ALL
          SELECT 'paidAds',
            CASE
              WHEN COALESCE(gclid, '') != '' THEN 'Google Ads'
              WHEN COALESCE(fbclid, '') != '' THEN 'Facebook / Meta'
              WHEN COALESCE(msclkid, '') != '' THEN 'Microsoft Ads'
              WHEN COALESCE(ttclid, '') != '' THEN 'TikTok Ads'
              WHEN COALESCE(li_fat_id, '') != '' THEN 'LinkedIn Ads'
              WHEN COALESCE(twclid, '') != '' THEN 'Twitter Ads (X)'
            END,
            session_id
          FROM model
          WHERE COALESCE(gclid, '') != '' OR COALESCE(fbclid, '') != ''
             OR COALESCE(msclkid, '') != '' OR COALESCE(ttclid, '') != ''
             OR COALESCE(li_fat_id, '') != '' OR COALESCE(twclid, '') != ''
          UNION ALL SELECT 'utm_source', utm_source, session_id FROM model WHERE COALESCE(utm_source, '') != ''
          UNION ALL SELECT 'utm_medium', utm_medium, session_id FROM model WHERE COALESCE(utm_medium, '') != ''
          UNION ALL SELECT 'utm_campaign', utm_campaign, session_id FROM model WHERE COALESCE(utm_campaign, '') != ''
          UNION ALL SELECT 'utm_content', utm_content, session_id FROM model WHERE COALESCE(utm_content, '') != ''
          UNION ALL SELECT 'utm_term', utm_term, session_id FROM model WHERE COALESCE(utm_term, '') != ''
        ),
        ranked AS (
          SELECT dimension, name, count(DISTINCT session_id)::bigint AS value,
            row_number() OVER (PARTITION BY dimension ORDER BY count(DISTINCT session_id) DESC, name) AS rank
          FROM dimensions
          GROUP BY dimension, name
        )
        SELECT dimension, name, value FROM ranked WHERE rank <= 20 ORDER BY dimension, value DESC
        "#,
    );
    let rows = match builder.build().fetch_all(&pool).await {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };
    let mut output = Map::new();
    for key in [
        "referrer",
        "paidAds",
        "utm_source",
        "utm_medium",
        "utm_campaign",
        "utm_content",
        "utm_term",
    ] {
        output.insert(key.to_string(), Value::Array(Vec::new()));
    }
    for row in rows {
        let dimension = row.get::<String, _>("dimension");
        if let Some(Value::Array(values)) = output.get_mut(&dimension) {
            values.push(json!({
                "name": row.get::<String, _>("name"),
                "value": row.get::<i64, _>("value"),
            }));
        }
    }

    let mut total_builder = QueryBuilder::<Postgres>::new(
        "SELECT count(*)::bigint AS pageviews, count(DISTINCT e.session_id)::bigint AS visitors, count(DISTINCT e.visit_id)::bigint AS visits FROM website_event e ",
    );
    if filters.needs_session {
        total_builder
            .push("JOIN session s ON s.session_id = e.session_id AND s.website_id = e.website_id ");
    }
    total_builder
        .push("WHERE e.website_id = ")
        .push_bind(website_id)
        .push(" AND e.created_at BETWEEN ")
        .push_bind(range.start)
        .push(" AND ")
        .push_bind(range.end)
        .push(" AND e.event_type = ")
        .push_bind(target_type)
        .push(" AND ")
        .push(target_column)
        .push(" = ")
        .push_bind(step);
    push_insight_filters(&mut total_builder, &filters, &audience, range);
    let total = match total_builder.build().fetch_one(&pool).await {
        Ok(row) => json!({
            "pageviews": row.get::<i64, _>("pageviews"),
            "visitors": row.get::<i64, _>("visitors"),
            "visits": row.get::<i64, _>("visits"),
        }),
        Err(error) => return database_error(error),
    };
    output.insert("total".to_string(), total);
    json_response(Value::Object(output))
}

fn performance_metric(params: &HashMap<String, String>) -> Result<&'static str, Response> {
    match params.get("metric").map(String::as_str).unwrap_or("lcp") {
        "lcp" => Ok("e.lcp"),
        "inp" => Ok("e.inp"),
        "cls" => Ok("e.cls"),
        "fcp" => Ok("e.fcp"),
        "ttfb" => Ok("e.ttfb"),
        _ => Err(error_response(
            StatusCode::BAD_REQUEST,
            "Invalid performance metric",
        )),
    }
}

fn push_event_joins(
    builder: &mut QueryBuilder<'_, Postgres>,
    filters: &Filters,
    force_session: bool,
) {
    if force_session || filters.needs_session {
        builder.push(
            " JOIN session s ON s.session_id = e.session_id AND s.website_id = e.website_id ",
        );
    }
}

async fn performance_chart(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let metric = match performance_metric(&params) {
        Ok(metric) => metric,
        Err(response) => return response,
    };
    let unit = match params.get("unit").map(String::as_str).unwrap_or("day") {
        "minute" => "minute",
        "hour" => "hour",
        "day" => "day",
        "month" => "month",
        "year" => "year",
        _ => return error_response(StatusCode::BAD_REQUEST, "Invalid unit"),
    };
    let (pool, range, filters, audience) = match context(
        &state,
        &headers,
        website_id,
        &params,
        &["startAt", "endAt", "timezone", "unit", "metric"],
        false,
        ShareSection::Performance,
        ShareDataVisibility::Aggregate,
    )
    .await
    {
        Ok(context) => context,
        Err(response) => return response,
    };
    let timezone = normalize_timezone(params.get("timezone").map(String::as_str).unwrap_or("UTC"));
    let mut builder = QueryBuilder::<Postgres>::new("SELECT to_char(date_trunc('");
    builder
        .push(unit)
        .push("', e.created_at AT TIME ZONE ")
        .push_bind(timezone)
        .push("), 'YYYY-MM-DD HH24:MI:SS') AS t, (percentile_cont(0.5) WITHIN GROUP (ORDER BY ")
        .push(metric)
        .push("))::float8 AS p50, (percentile_cont(0.75) WITHIN GROUP (ORDER BY ")
        .push(metric)
        .push("))::float8 AS p75, (percentile_cont(0.95) WITHIN GROUP (ORDER BY ")
        .push(metric)
        .push("))::float8 AS p95 FROM website_event e ");
    push_event_joins(&mut builder, &filters, false);
    builder
        .push(" WHERE e.website_id = ")
        .push_bind(website_id)
        .push(" AND e.event_type = 5 AND e.created_at BETWEEN ")
        .push_bind(range.start)
        .push(" AND ")
        .push_bind(range.end);
    push_insight_filters(&mut builder, &filters, &audience, range);
    builder.push(" GROUP BY 1 ORDER BY 1");
    match builder.build().fetch_all(&pool).await {
        Ok(rows) => json_response(json!({
            "chart": rows.into_iter().map(|row| json!({
                "t": row.get::<String, _>("t"),
                "p50": row.get::<Option<f64>, _>("p50"),
                "p75": row.get::<Option<f64>, _>("p75"),
                "p95": row.get::<Option<f64>, _>("p95"),
            })).collect::<Vec<_>>()
        })),
        Err(error) => database_error(error),
    }
}

async fn performance_stats(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let metric = match performance_metric(&params) {
        Ok(metric) => metric,
        Err(response) => return response,
    };
    let (pool, range, filters, audience) = match context(
        &state,
        &headers,
        website_id,
        &params,
        &["startAt", "endAt", "timezone", "unit", "metric"],
        false,
        ShareSection::Performance,
        ShareDataVisibility::Aggregate,
    )
    .await
    {
        Ok(context) => context,
        Err(response) => return response,
    };
    let mut builder = QueryBuilder::<Postgres>::new(
        "SELECT \
         COALESCE((percentile_cont(0.5) WITHIN GROUP (ORDER BY e.lcp))::float8, 0) AS lcp_p50, \
         COALESCE((percentile_cont(0.75) WITHIN GROUP (ORDER BY e.lcp))::float8, 0) AS lcp_p75, \
         COALESCE((percentile_cont(0.95) WITHIN GROUP (ORDER BY e.lcp))::float8, 0) AS lcp_p95, \
         COALESCE((percentile_cont(0.5) WITHIN GROUP (ORDER BY e.inp))::float8, 0) AS inp_p50, \
         COALESCE((percentile_cont(0.75) WITHIN GROUP (ORDER BY e.inp))::float8, 0) AS inp_p75, \
         COALESCE((percentile_cont(0.95) WITHIN GROUP (ORDER BY e.inp))::float8, 0) AS inp_p95, \
         COALESCE((percentile_cont(0.5) WITHIN GROUP (ORDER BY e.cls))::float8, 0) AS cls_p50, \
         COALESCE((percentile_cont(0.75) WITHIN GROUP (ORDER BY e.cls))::float8, 0) AS cls_p75, \
         COALESCE((percentile_cont(0.95) WITHIN GROUP (ORDER BY e.cls))::float8, 0) AS cls_p95, \
         COALESCE((percentile_cont(0.5) WITHIN GROUP (ORDER BY e.fcp))::float8, 0) AS fcp_p50, \
         COALESCE((percentile_cont(0.75) WITHIN GROUP (ORDER BY e.fcp))::float8, 0) AS fcp_p75, \
         COALESCE((percentile_cont(0.95) WITHIN GROUP (ORDER BY e.fcp))::float8, 0) AS fcp_p95, \
         COALESCE((percentile_cont(0.5) WITHIN GROUP (ORDER BY e.ttfb))::float8, 0) AS ttfb_p50, \
         COALESCE((percentile_cont(0.75) WITHIN GROUP (ORDER BY e.ttfb))::float8, 0) AS ttfb_p75, \
         COALESCE((percentile_cont(0.95) WITHIN GROUP (ORDER BY e.ttfb))::float8, 0) AS ttfb_p95, \
         count(",
    );
    builder
        .push(metric)
        .push(")::bigint AS count FROM website_event e ");
    push_event_joins(&mut builder, &filters, false);
    builder
        .push(" WHERE e.website_id = ")
        .push_bind(website_id)
        .push(" AND e.event_type = 5 AND e.created_at BETWEEN ")
        .push_bind(range.start)
        .push(" AND ")
        .push_bind(range.end);
    push_insight_filters(&mut builder, &filters, &audience, range);
    match builder.build().fetch_one(&pool).await {
        Ok(row) => {
            let percentile = |prefix: &str| {
                json!({
                    "p50": row.get::<f64, _>(format!("{prefix}_p50").as_str()),
                    "p75": row.get::<f64, _>(format!("{prefix}_p75").as_str()),
                    "p95": row.get::<f64, _>(format!("{prefix}_p95").as_str()),
                })
            };
            json_response(json!({
                "lcp": percentile("lcp"),
                "inp": percentile("inp"),
                "cls": percentile("cls"),
                "fcp": percentile("fcp"),
                "ttfb": percentile("ttfb"),
                "count": row.get::<i64, _>("count"),
            }))
        }
        Err(error) => database_error(error),
    }
}

async fn performance_metrics(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let metric = match performance_metric(&params) {
        Ok(metric) => metric,
        Err(response) => return response,
    };
    let (dimension, session_dimension) = match params.get("type").map(String::as_str) {
        Some("path") => ("e.url_path", false),
        Some("title") => ("e.page_title", false),
        Some("device") => ("s.device", true),
        Some("browser") => ("s.browser", true),
        _ => return error_response(StatusCode::BAD_REQUEST, "Invalid metric type"),
    };
    let limit = match params.get("limit") {
        Some(value) => match value
            .parse::<i64>()
            .ok()
            .filter(|value| (1..=500).contains(value))
        {
            Some(value) => Some(value),
            None => return error_response(StatusCode::BAD_REQUEST, "Invalid limit"),
        },
        None if params.get("type").map(String::as_str) == Some("device") => None,
        None => Some(500),
    };
    let (pool, range, filters, audience) = match context(
        &state,
        &headers,
        website_id,
        &params,
        &[
            "startAt", "endAt", "timezone", "unit", "metric", "type", "limit",
        ],
        false,
        ShareSection::Performance,
        ShareDataVisibility::Aggregate,
    )
    .await
    {
        Ok(context) => context,
        Err(response) => return response,
    };
    let mut builder = QueryBuilder::<Postgres>::new("SELECT ");
    builder
        .push(dimension)
        .push("::text AS name, (percentile_cont(0.5) WITHIN GROUP (ORDER BY ")
        .push(metric)
        .push("))::float8 AS p50, (percentile_cont(0.75) WITHIN GROUP (ORDER BY ")
        .push(metric)
        .push("))::float8 AS p75, (percentile_cont(0.95) WITHIN GROUP (ORDER BY ")
        .push(metric)
        .push("))::float8 AS p95, count(")
        .push(metric)
        .push(")::bigint AS count FROM website_event e ");
    push_event_joins(&mut builder, &filters, session_dimension);
    builder
        .push(" WHERE e.website_id = ")
        .push_bind(website_id)
        .push(" AND e.event_type = 5 AND e.created_at BETWEEN ")
        .push_bind(range.start)
        .push(" AND ")
        .push_bind(range.end);
    push_insight_filters(&mut builder, &filters, &audience, range);
    builder.push(" GROUP BY 1 ORDER BY p75 DESC");
    if let Some(limit) = limit {
        builder.push(" LIMIT ").push_bind(limit);
    }
    match builder.build().fetch_all(&pool).await {
        Ok(rows) => json_response(Value::Array(
            rows.into_iter()
                .map(|row| {
                    json!({
                        "name": row.get::<Option<String>, _>("name"),
                        "p50": row.get::<Option<f64>, _>("p50"),
                        "p75": row.get::<Option<f64>, _>("p75"),
                        "p95": row.get::<Option<f64>, _>("p95"),
                        "count": row.get::<i64, _>("count"),
                    })
                })
                .collect(),
        )),
        Err(error) => database_error(error),
    }
}

async fn utm_metrics(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let column = match params.get("type").map(String::as_str) {
        Some("utm_source") => "e.utm_source",
        Some("utm_medium") => "e.utm_medium",
        Some("utm_campaign") => "e.utm_campaign",
        Some("utm_term") => "e.utm_term",
        Some("utm_content") => "e.utm_content",
        _ => return error_response(StatusCode::BAD_REQUEST, "Invalid UTM type"),
    };
    if params
        .get("eventType")
        .and_then(|value| value.parse::<i32>().ok())
        .is_some_and(|value| value != 1)
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            "eventType conflicts with UTM metrics",
        );
    }
    let (pool, range, filters, audience) = match context(
        &state,
        &headers,
        website_id,
        &params,
        &["startAt", "endAt", "timezone", "unit", "type"],
        false,
        ShareSection::Utm,
        ShareDataVisibility::Aggregate,
    )
    .await
    {
        Ok(context) => context,
        Err(response) => return response,
    };
    let mut builder = QueryBuilder::<Postgres>::new("SELECT ");
    builder
        .push(column)
        .push("::text AS utm, count(*)::bigint AS views FROM website_event e ");
    push_event_joins(&mut builder, &filters, false);
    builder
        .push(" WHERE e.website_id = ")
        .push_bind(website_id)
        .push(" AND e.event_type = 1 AND e.created_at BETWEEN ")
        .push_bind(range.start)
        .push(" AND ")
        .push_bind(range.end)
        .push(" AND COALESCE(")
        .push(column)
        .push(", '') != ''");
    push_insight_filters(&mut builder, &filters, &audience, range);
    builder.push(" GROUP BY 1 ORDER BY 2 DESC LIMIT 50");
    match builder.build().fetch_all(&pool).await {
        Ok(rows) => json_response(Value::Array(
            rows.into_iter()
                .map(|row| {
                    json!({
                        "utm": row.get::<String, _>("utm"),
                        "views": row.get::<i64, _>("views"),
                    })
                })
                .collect(),
        )),
        Err(error) => database_error(error),
    }
}

fn breakdown_field(name: &str) -> Option<(&'static str, &'static str, bool)> {
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
        _ => return None,
    })
}

async fn breakdown(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let Some(raw_fields) = params.get("fields") else {
        return error_response(StatusCode::BAD_REQUEST, "fields is required");
    };
    let names = match serde_json::from_str::<Vec<String>>(raw_fields) {
        Ok(fields) if !fields.is_empty() => fields,
        _ => return error_response(StatusCode::BAD_REQUEST, "Invalid fields"),
    };
    let mut fields = Vec::with_capacity(names.len());
    for name in &names {
        match breakdown_field(name) {
            Some(field) => fields.push(field),
            None => return error_response(StatusCode::BAD_REQUEST, "Invalid breakdown field"),
        }
    }
    if params
        .get("eventType")
        .and_then(|value| value.parse::<i32>().ok())
        .is_some_and(|value| value != 1)
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            "eventType conflicts with breakdown",
        );
    }
    let (pool, range, filters, audience) = match context(
        &state,
        &headers,
        website_id,
        &params,
        &["startAt", "endAt", "timezone", "unit", "fields"],
        false,
        ShareSection::Breakdown,
        ShareDataVisibility::Aggregate,
    )
    .await
    {
        Ok(context) => context,
        Err(response) => return response,
    };
    let needs_session = fields.iter().any(|field| field.2);
    let mut builder = QueryBuilder::<Postgres>::new("WITH per_visit AS (SELECT ");
    for (index, (name, expression, _)) in fields.iter().enumerate() {
        if index > 0 {
            builder.push(", ");
        }
        builder
            .push(*expression)
            .push("::text AS \"")
            .push(*name)
            .push("\"");
    }
    builder.push(", e.session_id, e.visit_id, count(*)::bigint AS c, min(e.created_at) AS min_time, max(e.created_at) AS max_time FROM website_event e ");
    push_event_joins(&mut builder, &filters, needs_session);
    builder
        .push(" WHERE e.website_id = ")
        .push_bind(website_id)
        .push(" AND e.event_type = 1 AND e.created_at BETWEEN ")
        .push_bind(range.start)
        .push(" AND ")
        .push_bind(range.end);
    push_insight_filters(&mut builder, &filters, &audience, range);
    builder.push(" GROUP BY ");
    for (index, (name, _, _)) in fields.iter().enumerate() {
        if index > 0 {
            builder.push(", ");
        }
        builder.push("\"").push(*name).push("\"");
    }
    builder.push(", e.session_id, e.visit_id)");
    if !filters.exclude_bounce {
        builder
            .push(", custom_visits AS (SELECT session_id, visit_id, 1::int AS has_custom_event FROM website_event WHERE website_id = ")
            .push_bind(website_id)
            .push(" AND created_at BETWEEN ")
            .push_bind(range.start)
            .push(" AND ")
            .push_bind(range.end)
            .push(" AND event_type = 2 GROUP BY session_id, visit_id)");
    }
    builder.push(" SELECT COALESCE(sum(t.c), 0)::bigint AS views, count(DISTINCT t.session_id)::bigint AS visitors, count(DISTINCT t.visit_id)::bigint AS visits, ");
    if filters.exclude_bounce {
        builder.push("0::bigint AS bounces, ");
    } else {
        builder.push("COALESCE(sum(CASE WHEN t.c = 1 AND COALESCE(c.has_custom_event, 0) = 0 THEN 1 ELSE 0 END), 0)::bigint AS bounces, ");
    }
    builder.push("COALESCE(sum(floor(EXTRACT(EPOCH FROM (t.max_time - t.min_time)))), 0)::bigint AS totaltime");
    for (name, _, _) in &fields {
        builder.push(", t.\"").push(*name).push("\"");
    }
    builder.push(" FROM per_visit t ");
    if !filters.exclude_bounce {
        builder.push(
            "LEFT JOIN custom_visits c ON c.session_id = t.session_id AND c.visit_id = t.visit_id ",
        );
    }
    builder.push("GROUP BY ");
    for (index, (name, _, _)) in fields.iter().enumerate() {
        if index > 0 {
            builder.push(", ");
        }
        builder.push("t.\"").push(*name).push("\"");
    }
    builder.push(" ORDER BY visitors DESC, views DESC LIMIT 500");
    match builder.build().fetch_all(&pool).await {
        Ok(rows) => json_response(Value::Array(
            rows.into_iter()
                .map(|row| {
                    let mut value = Map::from_iter([
                        ("views".to_string(), json!(row.get::<i64, _>("views"))),
                        ("visitors".to_string(), json!(row.get::<i64, _>("visitors"))),
                        ("visits".to_string(), json!(row.get::<i64, _>("visits"))),
                        ("bounces".to_string(), json!(row.get::<i64, _>("bounces"))),
                        (
                            "totaltime".to_string(),
                            json!(row.get::<i64, _>("totaltime")),
                        ),
                    ]);
                    for (name, _, _) in &fields {
                        value.insert(
                            (*name).to_string(),
                            json!(row.get::<Option<String>, _>(*name)),
                        );
                    }
                    Value::Object(value)
                })
                .collect(),
        )),
        Err(error) => database_error(error),
    }
}

fn report_request(
    body: &Value,
    expected_type: &str,
) -> Result<(Uuid, HashMap<String, String>), Response> {
    let object = body
        .as_object()
        .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, "Invalid request body"))?;
    if object.get("type").and_then(Value::as_str) != Some(expected_type) {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "Report type does not match the route",
        ));
    }
    let website_id = object
        .get("websiteId")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, "websiteId must be a UUID"))?;
    let parameters = object
        .get("parameters")
        .and_then(Value::as_object)
        .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, "parameters must be an object"))?;
    let mut params = report_filter_params(object.get("filters"))?;
    for (source, target) in [("startDate", "startAt"), ("endDate", "endAt")] {
        let value = match parameters.get(source) {
            Some(Value::String(value)) => value.clone(),
            Some(Value::Number(value)) => value.to_string(),
            Some(_) => {
                return Err(error_response(
                    StatusCode::BAD_REQUEST,
                    format!("Invalid parameter: {source}"),
                ))
            }
            None => {
                return Err(error_response(
                    StatusCode::BAD_REQUEST,
                    format!("{source} is required"),
                ))
            }
        };
        if parse_date(&value).is_none() {
            return Err(error_response(
                StatusCode::BAD_REQUEST,
                format!("Invalid parameter: {source}"),
            ));
        }
        params.insert(target.to_string(), value);
    }
    for (key, value) in parameters {
        let allowed = match expected_type {
            "attribution" => matches!(key.as_str(), "model" | "type" | "step" | "currency"),
            "breakdown" => key == "fields",
            "journey" => matches!(
                key.as_str(),
                "steps" | "startStep" | "endStep" | "eventType"
            ),
            "retention" => key == "timezone",
            "revenue" => matches!(key.as_str(), "unit" | "timezone" | "currency" | "compare"),
            "performance" => matches!(key.as_str(), "unit" | "timezone" | "metric"),
            "utm" => false,
            _ => false,
        };
        if !allowed {
            continue;
        }
        let value = match value {
            Value::String(value) => value.clone(),
            Value::Number(value) => value.to_string(),
            Value::Bool(value) => value.to_string(),
            Value::Array(_) => serde_json::to_string(value).map_err(|_| {
                error_response(StatusCode::BAD_REQUEST, format!("Invalid parameter: {key}"))
            })?,
            _ => {
                return Err(error_response(
                    StatusCode::BAD_REQUEST,
                    format!("Unsupported parameter value: {key}"),
                ))
            }
        };
        params.insert(key.clone(), value);
    }
    Ok((website_id, params))
}

async fn response_value(response: Response) -> Result<Value, Response> {
    if !response.status().is_success() {
        return Err(response);
    }
    let bytes = to_bytes(response.into_body(), 16 * 1024 * 1024)
        .await
        .map_err(|_| error_response(StatusCode::INTERNAL_SERVER_ERROR, "Unable to read report"))?;
    serde_json::from_slice(&bytes)
        .map_err(|_| error_response(StatusCode::INTERNAL_SERVER_ERROR, "Unable to decode report"))
}

async fn report_attribution(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let (website_id, params) = match report_request(&body, "attribution") {
        Ok(request) => request,
        Err(response) => return response,
    };
    attribution(State(state), Path(website_id), Query(params), headers).await
}

async fn report_breakdown(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let (website_id, params) = match report_request(&body, "breakdown") {
        Ok(request) => request,
        Err(response) => return response,
    };
    breakdown(State(state), Path(website_id), Query(params), headers).await
}

async fn report_journey(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let (website_id, params) = match report_request(&body, "journey") {
        Ok(request) => request,
        Err(response) => return response,
    };
    journeys(State(state), Path(website_id), Query(params), headers).await
}

async fn report_retention(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let (website_id, params) = match report_request(&body, "retention") {
        Ok(request) => request,
        Err(response) => return response,
    };
    retention(State(state), Path(website_id), Query(params), headers).await
}

async fn revenue_report_value(
    state: AppState,
    headers: HeaderMap,
    website_id: Uuid,
    params: HashMap<String, String>,
) -> Result<Value, Response> {
    let chart = response_value(
        revenue_chart(
            State(state.clone()),
            Path(website_id),
            Query(params.clone()),
            headers.clone(),
        )
        .await,
    )
    .await?;
    let total = response_value(
        revenue_stats(
            State(state.clone()),
            Path(website_id),
            Query(params.clone()),
            headers.clone(),
        )
        .await,
    )
    .await?;
    let mut dimensions = Map::new();
    for kind in ["country", "region", "referrer", "channel"] {
        let mut dimension_params = params.clone();
        dimension_params.insert("type".to_string(), kind.to_string());
        dimensions.insert(
            kind.to_string(),
            response_value(
                revenue_metrics(
                    State(state.clone()),
                    Path(website_id),
                    Query(dimension_params),
                    headers.clone(),
                )
                .await,
            )
            .await?,
        );
    }
    Ok(json!({
        "chart": chart.get("chart").cloned().unwrap_or_else(|| Value::Array(Vec::new())),
        "total": total,
        "country": dimensions.remove("country").unwrap_or_else(|| Value::Array(Vec::new())),
        "region": dimensions.remove("region").unwrap_or_else(|| Value::Array(Vec::new())),
        "referrer": dimensions.remove("referrer").unwrap_or_else(|| Value::Array(Vec::new())),
        "channel": dimensions.remove("channel").unwrap_or_else(|| Value::Array(Vec::new())),
    }))
}

async fn report_revenue(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let (website_id, params) = match report_request(&body, "revenue") {
        Ok(request) => request,
        Err(response) => return response,
    };
    match revenue_report_value(state, headers, website_id, params).await {
        Ok(value) => json_response(value),
        Err(response) => response,
    }
}

async fn performance_report_value(
    state: AppState,
    headers: HeaderMap,
    website_id: Uuid,
    params: HashMap<String, String>,
) -> Result<Value, Response> {
    let chart = response_value(
        performance_chart(
            State(state.clone()),
            Path(website_id),
            Query(params.clone()),
            headers.clone(),
        )
        .await,
    )
    .await?;
    let summary = response_value(
        performance_stats(
            State(state.clone()),
            Path(website_id),
            Query(params.clone()),
            headers.clone(),
        )
        .await,
    )
    .await?;
    let mut dimensions = Map::new();
    for (kind, name) in [
        ("path", "pages"),
        ("title", "pageTitles"),
        ("device", "devices"),
        ("browser", "browsers"),
    ] {
        let mut dimension_params = params.clone();
        dimension_params.insert("type".to_string(), kind.to_string());
        dimensions.insert(
            name.to_string(),
            response_value(
                performance_metrics(
                    State(state.clone()),
                    Path(website_id),
                    Query(dimension_params),
                    headers.clone(),
                )
                .await,
            )
            .await?,
        );
    }
    Ok(json!({
        "chart": chart.get("chart").cloned().unwrap_or_else(|| Value::Array(Vec::new())),
        "summary": summary,
        "pages": dimensions.remove("pages").unwrap_or_else(|| Value::Array(Vec::new())),
        "pageTitles": dimensions.remove("pageTitles").unwrap_or_else(|| Value::Array(Vec::new())),
        "devices": dimensions.remove("devices").unwrap_or_else(|| Value::Array(Vec::new())),
        "browsers": dimensions.remove("browsers").unwrap_or_else(|| Value::Array(Vec::new())),
    }))
}

async fn report_performance(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let (website_id, params) = match report_request(&body, "performance") {
        Ok(request) => request,
        Err(response) => return response,
    };
    match performance_report_value(state, headers, website_id, params).await {
        Ok(value) => json_response(value),
        Err(response) => response,
    }
}

async fn utm_report_value(
    state: AppState,
    headers: HeaderMap,
    website_id: Uuid,
    params: HashMap<String, String>,
) -> Result<Value, Response> {
    let mut output = Map::new();
    for kind in [
        "utm_source",
        "utm_medium",
        "utm_campaign",
        "utm_term",
        "utm_content",
    ] {
        let mut metric_params = params.clone();
        metric_params.insert("type".to_string(), kind.to_string());
        output.insert(
            kind.to_string(),
            response_value(
                utm_metrics(
                    State(state.clone()),
                    Path(website_id),
                    Query(metric_params),
                    headers.clone(),
                )
                .await,
            )
            .await?,
        );
    }
    Ok(Value::Object(output))
}

async fn report_utm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let (website_id, params) = match report_request(&body, "utm") {
        Ok(request) => request,
        Err(response) => return response,
    };
    match utm_report_value(state, headers, website_id, params).await {
        Ok(value) => json_response(value),
        Err(response) => response,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_only_implemented_routes() {
        let routes = [
            "/api/websites/{website_id}/revenue/chart",
            "/api/websites/{website_id}/revenue/metrics",
            "/api/websites/{website_id}/revenue/sessions",
            "/api/websites/{website_id}/revenue/stats",
            "/api/websites/{website_id}/retention",
            "/api/websites/{website_id}/journeys",
            "/api/websites/{website_id}/attribution",
            "/api/websites/{website_id}/performance/chart",
            "/api/websites/{website_id}/performance/metrics",
            "/api/websites/{website_id}/performance/stats",
            "/api/websites/{website_id}/utm/metrics",
            "/api/websites/{website_id}/breakdown",
            "/api/reports/attribution",
            "/api/reports/breakdown",
            "/api/reports/journey",
            "/api/reports/retention",
            "/api/reports/revenue",
            "/api/reports/performance",
            "/api/reports/utm",
        ];
        assert_eq!(routes.len(), 19);
    }

    #[test]
    fn requires_resolved_audiences_and_ignores_legacy_pivot_filters() {
        let params = HashMap::from([("segment".to_string(), Uuid::new_v4().to_string())]);
        assert!(parse_filters(&params).is_err());
        let params = HashMap::from([("pf_country".to_string(), "eq.US".to_string())]);
        assert!(parse_filters(&params).is_ok());
        let params = HashMap::from([
            ("epf".to_string(), "ignored".to_string()),
            ("spf".to_string(), "ignored".to_string()),
        ]);
        assert!(parse_filters(&params).is_ok());
        let params = HashMap::from([("epf0".to_string(), "1.eq.plan.pro".to_string())]);
        assert!(parse_filters(&params).is_err());
    }

    #[test]
    fn defaults_to_non_test_data_and_keeps_requested_filters() {
        let params = HashMap::from([("browser".to_string(), "c.Safari".to_string())]);
        let filters = parse_filters(&params).unwrap();
        assert!(filters.needs_session);
        assert!(filters
            .values
            .iter()
            .any(|filter| filter.name == "browser" && filter.operator == "c"));
        assert!(filters.values.iter().any(|filter| {
            filter.name == "tag"
                && filter.operator == "neq"
                && filter.values == ["test".to_string()]
        }));
    }

    #[test]
    fn journey_output_collapses_only_adjacent_duplicates() {
        assert_eq!(
            combine_duplicates(vec![
                json!("/"),
                json!("/"),
                json!("/pricing"),
                json!("/"),
                Value::Null,
                Value::Null,
            ]),
            vec![json!("/"), json!("/pricing"), json!("/"), Value::Null]
        );
    }

    #[test]
    fn breakdown_fields_are_strictly_whitelisted() {
        assert_eq!(breakdown_field("browser").unwrap().1, "s.browser");
        assert!(breakdown_field("madeUpColumn").is_none());
    }

    #[test]
    fn report_aliases_normalize_structured_parameters() {
        let body = json!({
            "websiteId": Uuid::nil(),
            "type": "breakdown",
            "filters": { "startAt": 1, "endAt": 2 },
            "parameters": {
                "startDate": "1970-01-01T00:00:00.001Z",
                "endDate": "1970-01-01T00:00:00.002Z",
                "fields": ["path", "country"]
            }
        });
        let (_, params) = report_request(&body, "breakdown").unwrap();
        assert_eq!(
            params.get("startAt").map(String::as_str),
            Some("1970-01-01T00:00:00.001Z")
        );
        assert_eq!(
            params.get("fields").map(String::as_str),
            Some("[\"path\",\"country\"]")
        );
        assert!(!params.contains_key("startDate"));

        let utm = json!({
            "websiteId": Uuid::nil(),
            "type": "utm",
            "filters": { "startAt": 1, "endAt": 2, "timezone": "UTC" },
            "parameters": {
                "startDate": "1970-01-01T00:00:00.001Z",
                "endDate": "1970-01-01T00:00:00.002Z",
                "metric": "lcp"
            }
        });
        let (_, params) = report_request(&utm, "utm").unwrap();
        assert!(!params.contains_key("metric"));

        let nested = report_filter_params(Some(&json!({
            "browser": "eq.Chrome",
            "browser1": "neq.Firefox",
            "epf0": "1.eq.plan%2Eid.pro",
            "spf0": "4.bf.created%20at.2026-07-01",
            "eventPropertyFilters": [{ "ignored": true }]
        })))
        .unwrap();
        assert_eq!(nested.get("browser").map(String::as_str), Some("eq.Chrome"));
        assert_eq!(
            nested.get("browser1").map(String::as_str),
            Some("neq.Firefox")
        );
        assert_eq!(
            nested.get("epf0").map(String::as_str),
            Some("1.eq.plan%2Eid.pro")
        );
        assert_eq!(
            nested.get("spf0").map(String::as_str),
            Some("4.bf.created%20at.2026-07-01")
        );
        assert!(!nested.contains_key("eventPropertyFilters"));
    }

    #[test]
    fn previous_comparison_is_the_preceding_inclusive_window() {
        let range = comparison_range(
            DateRange {
                start: Utc.timestamp_millis_opt(10_000).unwrap(),
                end: Utc.timestamp_millis_opt(19_999).unwrap(),
            },
            "prev",
            Utc.timestamp_millis_opt(20_000).unwrap(),
        );
        assert_eq!(range.start.timestamp_millis(), 0);
        assert_eq!(range.end.timestamp_millis(), 9_999);
    }
}
