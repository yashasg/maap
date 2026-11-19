use super::*;
use crate::audience::Audience;
use chrono::{Duration, Months, NaiveDate, NaiveDateTime};
use sqlx::{Postgres, QueryBuilder};

#[derive(Clone, Copy)]
struct DateRange {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
}

#[derive(Clone)]
enum FilterValue {
    Text {
        operator: &'static str,
        values: Vec<String>,
    },
    Integer(i32),
}

#[derive(Clone)]
struct FilterSpec {
    name: &'static str,
    column: &'static str,
    value: FilterValue,
}

struct Filters {
    specs: Vec<FilterSpec>,
    match_any: bool,
    exclude_bounce: bool,
    needs_session: bool,
    has_event_filter: bool,
}

pub(crate) struct AnalyticsFilterSet(Filters);

impl AnalyticsFilterSet {
    pub(crate) fn from_params(params: &HashMap<String, String>) -> Result<Self, Response> {
        build_filters(params, None)
            .map(Self)
            .map_err(|message| error_response(StatusCode::BAD_REQUEST, message))
    }

    pub(crate) fn push_session_join(&self, builder: &mut QueryBuilder<'_, Postgres>) {
        if self.0.needs_session {
            builder.push(
                " JOIN session s ON s.session_id = e.session_id AND s.website_id = e.website_id ",
            );
        }
    }

    pub(crate) fn push(&self, builder: &mut QueryBuilder<'_, Postgres>) {
        push_filters(builder, &self.0);
    }
}

pub(crate) async fn analytics_timezone(
    pool: &PgPool,
    params: &HashMap<String, String>,
    required: bool,
) -> Result<String, Response> {
    let Some(timezone) = params.get("timezone") else {
        return if required {
            Err(error_response(StatusCode::BAD_REQUEST, "Missing timezone"))
        } else {
            Ok("UTC".to_string())
        };
    };
    let timezone = normalize_timezone(timezone);
    let valid = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM pg_timezone_names WHERE name = $1)",
    )
    .bind(timezone)
    .fetch_one(pool)
    .await
    .map_err(|error| error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    if valid {
        Ok(timezone.to_string())
    } else {
        Err(error_response(StatusCode::BAD_REQUEST, "Invalid timezone"))
    }
}

pub(crate) fn push_time_bucket(
    builder: &mut QueryBuilder<'_, Postgres>,
    field: &'static str,
    unit: &str,
    timezone: &str,
) -> Result<(), Response> {
    let (unit, format) = match unit {
        "minute" => ("minute", r#"YYYY-MM-DD"T"HH24:MI:00"Z""#),
        "hour" => ("hour", r#"YYYY-MM-DD"T"HH24:00:00"Z""#),
        "day" => ("day", r#"YYYY-MM-DD"T"HH24:00:00"Z""#),
        "month" => ("month", r#"YYYY-MM-01"T"HH24:00:00"Z""#),
        "year" => ("year", r#"YYYY-01-01"T"HH24:00:00"Z""#),
        _ => return Err(error_response(StatusCode::BAD_REQUEST, "Invalid unit")),
    };
    builder
        .push("to_char(date_trunc('")
        .push(unit)
        .push("', ")
        .push(field)
        .push(" AT TIME ZONE ")
        .push_bind(timezone.to_string())
        .push("), '")
        .push(format)
        .push("')");
    Ok(())
}

pub(crate) fn analytics_compare_range(
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    compare: &str,
    now: DateTime<Utc>,
) -> Result<(DateTime<Utc>, DateTime<Utc>), Response> {
    if !matches!(compare, "prev" | "yoy") {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "Invalid comparison",
        ));
    }
    let range = comparison_range(DateRange { start, end }, compare, now);
    Ok((range.start, range.end))
}

fn filter_column(name: &str) -> Option<(&'static str, &'static str, bool)> {
    Some(match name {
        "path" => ("path", "e.url_path", false),
        "entry" => ("entry", "e.url_path", false),
        "exit" => ("exit", "e.url_path", false),
        "referrer" => ("referrer", "e.referrer_domain", false),
        "domain" => ("domain", "e.referrer_domain", false),
        "title" => ("title", "e.page_title", false),
        "query" => ("query", "e.url_query", false),
        "os" => ("os", "s.os", true),
        "browser" => ("browser", "s.browser", true),
        "device" => ("device", "s.device", true),
        "screen" => ("screen", "s.screen", true),
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

fn event_filter(name: &str) -> bool {
    matches!(
        name,
        "path"
            | "entry"
            | "exit"
            | "referrer"
            | "domain"
            | "title"
            | "query"
            | "tag"
            | "hostname"
            | "event"
            | "utmSource"
            | "utmMedium"
            | "utmCampaign"
            | "utmContent"
            | "utmTerm"
            | "eventType"
    )
}

pub(crate) fn encode_property_name(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(
                *byte,
                b'!' | b'\'' | b'(' | b')' | b'*' | b'-' | b'_' | b'~'
            )
        {
            encoded.push(*byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn stored_segment_filters(
    parameters: &Value,
    existing: &HashMap<String, String>,
) -> Result<Vec<(String, String)>, &'static str> {
    let parameters = parameters
        .as_object()
        .ok_or("Segment parameters must be an object")?;
    if parameters
        .keys()
        .any(|key| !matches!(key.as_str(), "filters" | "sessionPropertyFilters" | "match"))
    {
        return Err("Segment contains unsupported parameters");
    }
    let mut output = Vec::new();
    let mut counts = HashMap::<String, usize>::new();
    if let Some(filters) = parameters.get("filters") {
        let filters = filters
            .as_array()
            .ok_or("Segment filters must be an array")?;
        for filter in filters {
            let filter = filter.as_object().ok_or("Invalid segment filter")?;
            if filter
                .keys()
                .any(|key| !matches!(key.as_str(), "name" | "operator" | "value"))
            {
                return Err("Segment filter contains unsupported values");
            }
            let name = filter
                .get("name")
                .and_then(Value::as_str)
                .ok_or("Invalid segment filter name")?;
            let operator = filter
                .get("operator")
                .and_then(Value::as_str)
                .ok_or("Invalid segment filter operator")?;
            let value = filter
                .get("value")
                .and_then(Value::as_str)
                .ok_or("Invalid segment filter value")?;
            if filter_column(name).is_none() {
                return Err("Invalid segment filter name");
            }
            if !matches!(operator, "eq" | "neq" | "c" | "dnc" | "re" | "nre") {
                return Err("Unsupported segment filter operator");
            }
            let count = counts.entry(name.to_string()).or_default();
            let key = if *count == 0 {
                name.to_string()
            } else {
                format!("{name}{count}")
            };
            *count += 1;
            let value = if name == "eventType" {
                if operator != "eq"
                    || value
                        .parse::<i32>()
                        .ok()
                        .filter(|value| *value > 0)
                        .is_none()
                {
                    return Err("Invalid event type segment filter");
                }
                value.to_string()
            } else {
                format!("{operator}.{value}")
            };
            output.push((key, value));
        }
    }
    if let Some(filters) = parameters.get("sessionPropertyFilters") {
        let filters = filters
            .as_array()
            .ok_or("Invalid session property filters")?;
        let mut index = 0;
        for filter in filters {
            let filter = filter
                .as_object()
                .ok_or("Invalid session property filter")?;
            if filter.keys().any(|key| {
                !matches!(
                    key.as_str(),
                    "propertyName" | "dataType" | "operator" | "value"
                )
            }) {
                return Err("Session property filter contains unsupported values");
            }
            let property_name = filter
                .get("propertyName")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or("Invalid session property name")?;
            let data_type = filter
                .get("dataType")
                .and_then(Value::as_i64)
                .filter(|value| (1..=5).contains(value))
                .ok_or("Invalid session property data type")?;
            let operator = filter
                .get("operator")
                .and_then(Value::as_str)
                .filter(|value| {
                    matches!(
                        *value,
                        "eq" | "neq"
                            | "c"
                            | "dnc"
                            | "re"
                            | "nre"
                            | "s"
                            | "ns"
                            | "t"
                            | "f"
                            | "gt"
                            | "gte"
                            | "lt"
                            | "lte"
                            | "bf"
                            | "af"
                    )
                })
                .ok_or("Invalid session property operator")?;
            let value = filter
                .get("value")
                .and_then(Value::as_str)
                .ok_or("Invalid session property value")?;
            while existing.contains_key(&format!("spf{index}")) {
                index += 1;
            }
            output.push((
                format!("spf{index}"),
                format!(
                    "{data_type}.{operator}.{}.{value}",
                    encode_property_name(property_name)
                ),
            ));
            index += 1;
        }
    }
    if let Some(value) = parameters.get("match") {
        let value = value.as_str().ok_or("Invalid segment match")?;
        if !matches!(value, "all" | "any") {
            return Err("Invalid segment match");
        }
        output.push(("match".to_string(), value.to_string()));
    }
    Ok(output)
}

pub(crate) async fn resolve_segment(
    pool: &PgPool,
    website_id: Uuid,
    params: &mut HashMap<String, String>,
) -> Result<(), Response> {
    let Some(segment) = params.remove("segment") else {
        return Ok(());
    };
    let segment_id = Uuid::parse_str(&segment)
        .map_err(|_| error_response(StatusCode::BAD_REQUEST, "Invalid segment"))?;
    let row = sqlx::query(
        r#"
        SELECT type, parameters::text AS parameters
        FROM segment
        WHERE segment_id = $1 AND website_id = $2
        "#,
    )
    .bind(segment_id)
    .bind(website_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
    .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, "Invalid segment"))?;
    if row.get::<String, _>("type") != "segment" {
        return Err(error_response(StatusCode::BAD_REQUEST, "Invalid segment"));
    }
    let parameters = serde_json::from_str::<Value>(&row.get::<String, _>("parameters"))
        .map_err(|_| error_response(StatusCode::BAD_REQUEST, "Invalid segment parameters"))?;
    let filters = stored_segment_filters(&parameters, params)
        .map_err(|message| error_response(StatusCode::BAD_REQUEST, message))?;
    params.extend(filters);
    Ok(())
}

fn parse_filter(value: &str) -> Result<FilterValue, String> {
    let (operator, value) = value
        .split_once('.')
        .filter(|(operator, _)| matches!(*operator, "eq" | "neq" | "c" | "dnc" | "re" | "nre"))
        .unwrap_or(("eq", value));
    let operator = match operator {
        "eq" => "eq",
        "neq" => "neq",
        "c" => "c",
        "dnc" => "dnc",
        "re" => "re",
        "nre" => "nre",
        _ => return Err("Invalid filter operator".to_string()),
    };
    let values = if matches!(operator, "eq" | "neq") {
        value.split(',').map(str::to_string).collect()
    } else {
        vec![value.to_string()]
    };
    Ok(FilterValue::Text { operator, values })
}

fn build_filters(
    params: &HashMap<String, String>,
    force_event_type: Option<i32>,
) -> Result<Filters, String> {
    if params.keys().any(|key| {
        key == "segment" || key == "cohort" || key.starts_with("epf") || key.starts_with("spf")
    }) {
        return Err("Invalid audience filter".to_string());
    }
    if params.keys().any(|key| key.starts_with("pf_")) {
        return Err("Invalid property filter".to_string());
    }

    let mut specs = Vec::new();
    let mut has_tag = false;
    let mut needs_session = false;
    let mut has_event_filter = false;

    let mut entries: Vec<_> = params.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    for (key, value) in entries {
        let base = key.trim_end_matches(|c: char| c.is_ascii_digit());
        let Some((name, column, session_column)) = filter_column(base) else {
            continue;
        };
        if name == "eventType" && force_event_type.is_some() {
            continue;
        }
        let parsed = if name == "eventType" {
            let value = value
                .parse::<i32>()
                .ok()
                .filter(|value| *value > 0)
                .ok_or_else(|| "Invalid event type filter".to_string())?;
            FilterValue::Integer(value)
        } else {
            parse_filter(value)?
        };
        has_tag |= name == "tag";
        needs_session |= session_column || name == "referrer";
        has_event_filter |= event_filter(name);
        specs.push(FilterSpec {
            name,
            column,
            value: parsed,
        });
    }

    if let Some(event_type) = force_event_type {
        specs.push(FilterSpec {
            name: "eventType",
            column: "e.event_type",
            value: FilterValue::Integer(event_type),
        });
        has_event_filter = true;
    }

    if !has_tag {
        let test_only = params.get("testMode").map(String::as_str) == Some("true");
        specs.push(FilterSpec {
            name: "tag",
            column: "e.tag",
            value: FilterValue::Text {
                operator: if test_only { "eq" } else { "neq" },
                values: vec!["test".to_string()],
            },
        });
        has_event_filter = true;
    }

    Ok(Filters {
        specs,
        match_any: params.get("match").map(String::as_str) == Some("any"),
        exclude_bounce: params
            .get("excludeBounce")
            .is_some_and(|value| !value.is_empty()),
        needs_session,
        has_event_filter,
    })
}

fn push_filter(builder: &mut QueryBuilder<'_, Postgres>, filter: &FilterSpec) {
    match &filter.value {
        FilterValue::Integer(value) => {
            builder.push(filter.column).push(" = ").push_bind(*value);
        }
        FilterValue::Text { operator, values } => match *operator {
            "eq" => {
                builder
                    .push(filter.column)
                    .push(" = ANY(")
                    .push_bind(values.clone())
                    .push(")");
            }
            "neq" => {
                builder
                    .push("COALESCE(")
                    .push(filter.column)
                    .push(", '') != ALL(")
                    .push_bind(values.clone())
                    .push(")");
            }
            "c" => {
                builder
                    .push(filter.column)
                    .push(" ILIKE ")
                    .push_bind(format!("%{}%", values[0]));
            }
            "dnc" => {
                builder
                    .push("COALESCE(")
                    .push(filter.column)
                    .push(", '') NOT ILIKE ")
                    .push_bind(format!("%{}%", values[0]));
            }
            "re" => {
                builder
                    .push(filter.column)
                    .push(" ~* ")
                    .push_bind(values[0].clone());
            }
            "nre" => {
                builder
                    .push("COALESCE(")
                    .push(filter.column)
                    .push(", '') !~* ")
                    .push_bind(values[0].clone());
            }
            _ => unreachable!(),
        },
    }
}

fn push_filters(builder: &mut QueryBuilder<'_, Postgres>, filters: &Filters) {
    let (always, matched): (Vec<_>, Vec<_>) = filters
        .specs
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
    if filters.specs.iter().any(|filter| filter.name == "referrer") {
        builder.push(
            " AND (e.referrer_domain != regexp_replace(e.hostname, '^www.', '') OR e.referrer_domain IS NULL)",
        );
    }
}

fn push_common_joins(
    builder: &mut QueryBuilder<'_, Postgres>,
    filters: &Filters,
    website_id: Uuid,
    range: DateRange,
    force_session: bool,
) {
    if filters.exclude_bounce {
        builder
            .push(
                r#"
                JOIN (
                    SELECT session_id, visit_id
                    FROM website_event
                    WHERE website_id = "#,
            )
            .push_bind(website_id)
            .push(" AND created_at BETWEEN ")
            .push_bind(range.start)
            .push(" AND ")
            .push_bind(range.end)
            .push(
                r#"
                      AND event_type != 5
                    GROUP BY session_id, visit_id
                    HAVING SUM(CASE WHEN event_type NOT IN (2, 5) THEN 1 ELSE 0 END) > 1
                       OR (
                         SUM(CASE WHEN event_type NOT IN (2, 5) THEN 1 ELSE 0 END) = 1
                         AND SUM(CASE WHEN event_type = 2 THEN 1 ELSE 0 END) > 0
                       )
                ) xb ON xb.session_id = e.session_id AND xb.visit_id = e.visit_id
                "#,
            );
    }
    if force_session || filters.needs_session {
        builder.push(
            " JOIN session s ON s.session_id = e.session_id AND s.website_id = e.website_id ",
        );
    }
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
        "24h" => "SELECT CURRENT_TIMESTAMP - INTERVAL '24 hours' AS start_date, CURRENT_TIMESTAMP AS end_date, $1::text",
        "7d" => "SELECT CURRENT_TIMESTAMP - INTERVAL '7 days' AS start_date, CURRENT_TIMESTAMP AS end_date, $1::text",
        "30d" => "SELECT CURRENT_TIMESTAMP - INTERVAL '30 days' AS start_date, CURRENT_TIMESTAMP AS end_date, $1::text",
        "today" | "0day" => r#"
            SELECT
              date_trunc('day', CURRENT_TIMESTAMP AT TIME ZONE $1) AT TIME ZONE $1 AS start_date,
              (date_trunc('day', CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 day' - INTERVAL '1 millisecond') AT TIME ZONE $1 AS end_date
        "#,
        "24hour" => r#"
            SELECT
              (date_trunc('hour', CURRENT_TIMESTAMP AT TIME ZONE $1) - INTERVAL '24 hours') AT TIME ZONE $1 AS start_date,
              (date_trunc('hour', CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 hour' - INTERVAL '1 millisecond') AT TIME ZONE $1 AS end_date
        "#,
        "0week" => r#"
            SELECT
              (date_trunc('week', (CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 day') - INTERVAL '1 day') AT TIME ZONE $1 AS start_date,
              (date_trunc('week', (CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 day') + INTERVAL '6 days 23 hours 59 minutes 59.999 seconds') AT TIME ZONE $1 AS end_date
        "#,
        "7day" => r#"
            SELECT
              (date_trunc('day', CURRENT_TIMESTAMP AT TIME ZONE $1) - INTERVAL '7 days') AT TIME ZONE $1 AS start_date,
              (date_trunc('day', CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 day' - INTERVAL '1 millisecond') AT TIME ZONE $1 AS end_date
        "#,
        "0month" => r#"
            SELECT
              date_trunc('month', CURRENT_TIMESTAMP AT TIME ZONE $1) AT TIME ZONE $1 AS start_date,
              (date_trunc('month', CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 month' - INTERVAL '1 millisecond') AT TIME ZONE $1 AS end_date
        "#,
        "30day" | "90day" => {
            if period == "30day" {
                r#"
                    SELECT
                      (date_trunc('day', CURRENT_TIMESTAMP AT TIME ZONE $1) - INTERVAL '30 days') AT TIME ZONE $1 AS start_date,
                      (date_trunc('day', CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 day' - INTERVAL '1 millisecond') AT TIME ZONE $1 AS end_date
                "#
            } else {
                r#"
                    SELECT
                      (date_trunc('day', CURRENT_TIMESTAMP AT TIME ZONE $1) - INTERVAL '90 days') AT TIME ZONE $1 AS start_date,
                      (date_trunc('day', CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 day' - INTERVAL '1 millisecond') AT TIME ZONE $1 AS end_date
                "#
            }
        }
        "0year" => r#"
            SELECT
              date_trunc('year', CURRENT_TIMESTAMP AT TIME ZONE $1) AT TIME ZONE $1 AS start_date,
              (date_trunc('year', CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 year' - INTERVAL '1 millisecond') AT TIME ZONE $1 AS end_date
        "#,
        "6month" | "12month" => {
            if period == "6month" {
                r#"
                    SELECT
                      (date_trunc('month', CURRENT_TIMESTAMP AT TIME ZONE $1) - INTERVAL '6 months') AT TIME ZONE $1 AS start_date,
                      (date_trunc('month', CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 month' - INTERVAL '1 millisecond') AT TIME ZONE $1 AS end_date
                "#
            } else {
                r#"
                    SELECT
                      (date_trunc('month', CURRENT_TIMESTAMP AT TIME ZONE $1) - INTERVAL '12 months') AT TIME ZONE $1 AS start_date,
                      (date_trunc('month', CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 month' - INTERVAL '1 millisecond') AT TIME ZONE $1 AS end_date
                "#
            }
        }
        _ => return Err(error_response(StatusCode::BAD_REQUEST, "Invalid period")),
    };
    let row = sqlx::query(sql)
        .bind(timezone)
        .fetch_one(pool)
        .await
        .map_err(|error| error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    Ok(DateRange {
        start: row.get("start_date"),
        end: row.get("end_date"),
    })
}

async fn request_date_range(
    pool: &PgPool,
    website_id: Uuid,
    params: &HashMap<String, String>,
) -> Result<DateRange, Response> {
    if params.contains_key("timezone") {
        analytics_timezone(pool, params, false).await?;
    }
    let explicit = ["startAt", "endAt", "startDate", "endDate"]
        .iter()
        .any(|key| params.contains_key(*key));
    let mut range = if let Some(period) = params.get("period") {
        if explicit {
            return Err(error_response(
                StatusCode::BAD_REQUEST,
                "period cannot be combined with an explicit date range",
            ));
        }
        period_range(
            pool,
            period,
            params.get("timezone").map(String::as_str).unwrap_or("UTC"),
        )
        .await?
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

fn metric_dimension(kind: &str) -> Option<(&'static str, bool, bool)> {
    Some(match kind {
        "path" => ("e.url_path", false, false),
        "fullPath" => (
            "CASE WHEN COALESCE(e.url_query, '') != '' THEN e.url_path || '?' || e.url_query ELSE e.url_path END",
            false,
            false,
        ),
        "entry" | "exit" => ("x.url_path", false, false),
        "referrer" | "domain" => ("e.referrer_domain", false, false),
        "title" => ("e.page_title", false, false),
        "query" => ("e.url_query", false, false),
        "event" => ("e.event_name", false, true),
        "tag" => ("e.tag", false, false),
        "hostname" => ("e.hostname", false, false),
        "utmSource" => ("e.utm_source", false, false),
        "utmMedium" => ("e.utm_medium", false, false),
        "utmCampaign" => ("e.utm_campaign", false, false),
        "utmContent" => ("e.utm_content", false, false),
        "utmTerm" => ("e.utm_term", false, false),
        "browser" => ("s.browser", true, false),
        "os" => ("s.os", true, false),
        "device" => ("s.device", true, false),
        "screen" => ("s.screen", true, false),
        "language" => ("lower(left(s.language, 2))", true, false),
        "country" => ("s.country", true, false),
        "city" => ("s.city", true, false),
        "region" => ("s.region", true, false),
        "distinctId" => ("s.distinct_id", true, false),
        _ => return None,
    })
}

fn pagination(params: &HashMap<String, String>) -> Result<(i64, i64), String> {
    let limit = params
        .get("limit")
        .map(|value| value.parse::<i64>())
        .transpose()
        .map_err(|_| "Invalid limit".to_string())?
        .unwrap_or(500);
    let offset = params
        .get("offset")
        .map(|value| value.parse::<i64>())
        .transpose()
        .map_err(|_| "Invalid offset".to_string())?
        .unwrap_or(0);
    if limit < 0 || offset < 0 {
        return Err("Invalid pagination".to_string());
    }
    Ok((limit, offset))
}

async fn query_metric_rows(
    pool: &PgPool,
    website_id: Uuid,
    range: DateRange,
    kind: &str,
    params: &HashMap<String, String>,
    audience: &Audience,
) -> Result<Vec<Value>, Response> {
    let Some((column, session_dimension, custom_event)) = metric_dimension(kind) else {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "Invalid metric type",
        ));
    };
    let mut filter_params = params.clone();
    if let Some(search) = params.get("search") {
        filter_params.insert(kind.to_string(), format!("c.{search}"));
    }
    let filters = build_filters(&filter_params, if custom_event { Some(2) } else { None })
        .map_err(|message| error_response(StatusCode::BAD_REQUEST, message))?;
    let (limit, offset) =
        pagination(params).map_err(|message| error_response(StatusCode::BAD_REQUEST, message))?;
    let include_country = matches!(kind, "city" | "region");

    let mut builder = QueryBuilder::<Postgres>::new("SELECT (");
    builder.push(column).push(")::text AS x, ");
    if custom_event {
        builder.push("COUNT(*)::bigint AS y");
    } else {
        builder.push("COUNT(DISTINCT e.session_id)::bigint AS y");
    }
    if include_country {
        builder.push(", s.country AS country");
    }
    builder.push(" FROM website_event e ");

    push_common_joins(&mut builder, &filters, website_id, range, session_dimension);
    if matches!(kind, "entry" | "exit") {
        builder
            .push(
                r#"
                JOIN (
                    SELECT DISTINCT ON (visit_id) visit_id, url_path, url_query
                    FROM website_event
                    WHERE website_id = "#,
            )
            .push_bind(website_id)
            .push(" AND created_at BETWEEN ")
            .push_bind(range.start)
            .push(" AND ")
            .push_bind(range.end)
            .push(" AND event_type NOT IN (2, 5) ORDER BY visit_id, created_at ")
            .push(if kind == "entry" { "ASC" } else { "DESC" })
            .push(") x ON x.visit_id = e.visit_id ");
    }
    builder
        .push(" WHERE e.website_id = ")
        .push_bind(website_id)
        .push(" AND e.created_at BETWEEN ")
        .push_bind(range.start)
        .push(" AND ")
        .push_bind(range.end);
    if !custom_event {
        builder.push(" AND e.event_type NOT IN (2, 5)");
    }
    builder
        .push(" AND COALESCE((")
        .push(column)
        .push(")::text, '') != ''");
    if kind == "fullPath" {
        if let Some(search) = params.get("search") {
            builder
                .push(" AND (")
                .push(column)
                .push(")::text ILIKE ")
                .push_bind(format!("%{search}%"));
        }
    }
    if matches!(kind, "referrer" | "domain") {
        builder.push(
            " AND e.referrer_domain != regexp_replace(e.hostname, '^www.', '') AND e.referrer_domain != ''",
        );
    }
    push_filters(&mut builder, &filters);
    if !audience.is_empty() {
        audience.push(&mut builder, "e", range.start, range.end);
    }
    builder.push(" GROUP BY 1");
    if include_country {
        builder.push(", 3");
    }
    builder
        .push(" ORDER BY 2 DESC LIMIT ")
        .push_bind(limit)
        .push(" OFFSET ")
        .push_bind(offset);

    let rows =
        builder.build().fetch_all(pool).await.map_err(|error| {
            error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
        })?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let mut value = json!({
                "x": row.get::<Option<String>, _>("x"),
                "y": row.get::<i64, _>("y")
            });
            if include_country {
                value["country"] = json!(row.get::<Option<String>, _>("country"));
            }
            value
        })
        .collect())
}

const PAID_AD_PARAMS: &[&str] = &[
    "ad_id=",
    "aid=",
    "dclid=",
    "epik=",
    "gclid=",
    "li_fat_id=",
    "msclkid=",
    "ob_click_id=",
    "pc_id=",
    "rdt_cid=",
    "scid=",
    "ttclid=",
    "twclid=",
    "utm_medium=cpc",
    "utm_medium=paid",
    "utm_medium=paid_social",
    "utm_source=google",
];
const LLM_DOMAINS: &[&str] = &[
    "chatgpt.com",
    "claude.ai",
    "copilot.microsoft.com",
    "gemini.google.com",
    "meta.ai",
    "perplexity.ai",
];
const SEARCH_DOMAINS: &[&str] = &[
    "baidu.com",
    "bing.com",
    "duckduckgo.com",
    "ecosia.org",
    "google.",
    "msn.com",
    "search.brave.com",
    "yandex.",
];
const SOCIAL_DOMAINS: &[&str] = &[
    "bsky.app",
    "facebook.com",
    "fb.com",
    "ig.com",
    "instagram.com",
    "linkedin.",
    "news.ycombinator.com",
    "pinterest.",
    "reddit.",
    "snapchat.",
    "t.co",
    "threads.net",
    "tiktok.",
    "twitter.com",
    "x.com",
];
const EMAIL_DOMAINS: &[&str] = &[
    "gmail.",
    "hotmail.",
    "mail.yahoo.",
    "outlook.",
    "proton.me",
    "protonmail.",
];
const SHOPPING_DOMAINS: &[&str] = &[
    "alibaba.com",
    "aliexpress.com",
    "amazon.",
    "bestbuy.com",
    "ebay.com",
    "etsy.com",
    "newegg.com",
    "target.com",
    "walmart.com",
];
const VIDEO_DOMAINS: &[&str] = &["twitch.", "youtube."];

fn push_contains_any(
    builder: &mut QueryBuilder<'_, Postgres>,
    column: &'static str,
    values: &[&str],
) {
    builder.push("(");
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            builder.push(" OR ");
        }
        builder
            .push("strpos(lower(COALESCE(")
            .push(column)
            .push(", '')), ")
            .push_bind(value.to_ascii_lowercase())
            .push(") > 0");
    }
    builder.push(")");
}

async fn query_channel_rows(
    pool: &PgPool,
    website_id: Uuid,
    range: DateRange,
    params: &HashMap<String, String>,
    audience: &Audience,
) -> Result<Vec<Value>, Response> {
    let filters = build_filters(params, None)
        .map_err(|message| error_response(StatusCode::BAD_REQUEST, message))?;
    let mut builder = QueryBuilder::<Postgres>::new(
        r#"
        WITH prefix AS (
          SELECT
            CASE WHEN e.utm_medium LIKE 'p%'
        "#,
    );
    for value in ["ppc", "retargeting", "paid"] {
        builder
            .push(" OR strpos(lower(COALESCE(e.utm_medium, '')), ")
            .push_bind(value)
            .push(") > 0");
    }
    builder.push(
        r#" THEN 'paid' ELSE 'organic' END AS prefix,
            e.referrer_domain, e.url_query, e.utm_medium, e.utm_source,
            e.session_id, e.visit_id, e.event_id, e.created_at, e.hostname
          FROM website_event e
        "#,
    );
    push_common_joins(&mut builder, &filters, website_id, range, false);
    builder
        .push(" WHERE e.website_id = ")
        .push_bind(website_id)
        .push(" AND e.event_type NOT IN (2, 5)")
        .push(" AND e.created_at BETWEEN ")
        .push_bind(range.start)
        .push(" AND ")
        .push_bind(range.end);
    push_filters(&mut builder, &filters);
    if !audience.is_empty() {
        audience.push(&mut builder, "e", range.start, range.end);
    }
    builder.push(
        r#"
        ),
        channels AS (
          SELECT CASE
            WHEN referrer_domain = '' AND url_query = '' THEN 'direct'
            WHEN "#,
    );
    push_contains_any(&mut builder, "url_query", PAID_AD_PARAMS);
    builder.push(" THEN 'paidAds' WHEN ");
    push_contains_any(&mut builder, "utm_medium", &["referral", "app", "link"]);
    builder.push(
        " THEN 'referral' WHEN strpos(lower(COALESCE(utm_medium, '')), 'affiliate') > 0 THEN 'affiliate' \
         WHEN strpos(lower(COALESCE(utm_medium, '')), 'sms') > 0 OR strpos(lower(COALESCE(utm_source, '')), 'sms') > 0 THEN 'sms' \
         WHEN ",
    );
    push_contains_any(&mut builder, "referrer_domain", LLM_DOMAINS);
    builder.push(" THEN 'llm' WHEN ");
    push_contains_any(&mut builder, "referrer_domain", SEARCH_DOMAINS);
    builder.push(
        " OR strpos(lower(COALESCE(utm_medium, '')), 'organic') > 0 THEN prefix || 'Search' WHEN ",
    );
    push_contains_any(&mut builder, "referrer_domain", SOCIAL_DOMAINS);
    builder.push(" THEN prefix || 'Social' WHEN ");
    push_contains_any(&mut builder, "referrer_domain", EMAIL_DOMAINS);
    builder.push(" OR strpos(lower(COALESCE(utm_medium, '')), 'mail') > 0 THEN 'email' WHEN ");
    push_contains_any(&mut builder, "referrer_domain", SHOPPING_DOMAINS);
    builder.push(
        " OR strpos(lower(COALESCE(utm_medium, '')), 'shop') > 0 THEN prefix || 'Shopping' WHEN ",
    );
    push_contains_any(&mut builder, "referrer_domain", VIDEO_DOMAINS);
    builder.push(
        r#" OR strpos(lower(COALESCE(utm_medium, '')), 'video') > 0 THEN prefix || 'Video'
            WHEN referrer_domain != regexp_replace(hostname, '^www.', '') AND referrer_domain != '' THEN 'referral'
            ELSE '' END AS x,
            session_id, visit_id, event_id, created_at
          FROM prefix
        ),
        visit_channels AS (
          SELECT session_id, visit_id, COALESCE(NULLIF(x, ''), 'direct') AS x
          FROM (
            SELECT x, session_id, visit_id,
              row_number() OVER (
                PARTITION BY session_id, visit_id
                ORDER BY CASE WHEN x != '' THEN 0 ELSE 1 END, created_at, event_id
              ) AS row_num
            FROM channels
          ) ranked_channels
          WHERE row_num = 1
        )
        SELECT x, COUNT(DISTINCT session_id)::bigint AS y
        FROM visit_channels
        GROUP BY x
        ORDER BY y DESC
        "#,
    );
    let rows =
        builder.build().fetch_all(pool).await.map_err(|error| {
            error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
        })?;
    Ok(rows
        .into_iter()
        .map(|row| json!({ "x": row.get::<String, _>("x"), "y": row.get::<i64, _>("y") }))
        .collect())
}

pub(super) async fn handle_website_metrics(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(mut params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match crate::access::website_section_pool(
        &state,
        &headers,
        website_id,
        &[
            crate::access::ShareSection::Overview,
            crate::access::ShareSection::Events,
            crate::access::ShareSection::Sessions,
            crate::access::ShareSection::Compare,
            crate::access::ShareSection::Breakdown,
            crate::access::ShareSection::Utm,
            crate::access::ShareSection::Attribution,
        ],
        crate::access::ShareDataVisibility::Aggregate,
    )
    .await
    {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let audience = match Audience::resolve(&pool, website_id, &mut params).await {
        Ok(audience) => audience,
        Err(response) => return response,
    };
    let range = match request_date_range(&pool, website_id, &params).await {
        Ok(range) => range,
        Err(response) => return response,
    };
    let Some(kind) = params.get("type") else {
        return error_response(StatusCode::BAD_REQUEST, "Invalid metric type");
    };
    let result = if kind == "channel" {
        query_channel_rows(&pool, website_id, range, &params, &audience).await
    } else {
        query_metric_rows(&pool, website_id, range, kind, &params, &audience).await
    };
    match result {
        Ok(rows) => with_rust_header(Json(rows).into_response()),
        Err(response) => response,
    }
}

async fn query_stats_filtered(
    pool: &PgPool,
    website_id: Uuid,
    range: DateRange,
    params: &HashMap<String, String>,
    audience: &Audience,
    has_event_properties: bool,
) -> Result<Value, Response> {
    let filters = build_filters(params, None)
        .map_err(|message| error_response(StatusCode::BAD_REQUEST, message))?;
    let filtered_path = filters.has_event_filter || has_event_properties;
    let mut builder = QueryBuilder::<Postgres>::new(
        "SELECT CAST(COALESCE(SUM(t.c), 0) AS bigint) AS pageviews, \
         COUNT(DISTINCT t.session_id)::bigint AS visitors, \
         COUNT(DISTINCT t.visit_id)::bigint AS visits, ",
    );
    if filters.exclude_bounce {
        builder.push("0::bigint AS bounces, ");
    } else if filtered_path {
        builder.push(
            "CAST(COALESCE(SUM(CASE WHEN t.c = 1 AND COALESCE(custom.has_custom_event, 0) = 0 THEN 1 ELSE 0 END), 0) AS bigint) AS bounces, ",
        );
    } else {
        builder.push(
            "CAST(COALESCE(SUM(CASE WHEN t.c = 1 AND t.has_custom_event = 0 THEN 1 ELSE 0 END), 0) AS bigint) AS bounces, ",
        );
    }
    builder.push(
        "CAST(COALESCE(SUM(floor(EXTRACT(EPOCH FROM (t.max_time - t.min_time)))), 0) AS bigint) AS totaltime FROM (SELECT e.session_id, e.visit_id, ",
    );
    if filtered_path {
        builder.push(
            "COUNT(*)::bigint AS c, MIN(e.created_at) AS min_time, MAX(e.created_at) AS max_time",
        );
    } else {
        builder.push(
            "SUM(CASE WHEN e.event_type NOT IN (2, 5) THEN 1 ELSE 0 END)::bigint AS c, \
             MIN(CASE WHEN e.event_type NOT IN (2, 5) THEN e.created_at END) AS min_time, \
             MAX(CASE WHEN e.event_type NOT IN (2, 5) THEN e.created_at END) AS max_time, \
             MAX(CASE WHEN e.event_type = 2 THEN 1 ELSE 0 END)::int AS has_custom_event",
        );
    }
    builder.push(" FROM website_event e ");
    push_common_joins(&mut builder, &filters, website_id, range, false);
    builder
        .push(" WHERE e.website_id = ")
        .push_bind(website_id)
        .push(" AND e.created_at BETWEEN ")
        .push_bind(range.start)
        .push(" AND ")
        .push_bind(range.end);
    if filtered_path {
        builder.push(" AND e.event_type NOT IN (2, 5)");
    } else {
        builder.push(" AND e.event_type != 5");
    }
    push_filters(&mut builder, &filters);
    if !audience.is_empty() {
        audience.push(&mut builder, "e", range.start, range.end);
    }
    builder.push(" GROUP BY e.session_id, e.visit_id");
    if !filtered_path {
        builder.push(" HAVING SUM(CASE WHEN e.event_type NOT IN (2, 5) THEN 1 ELSE 0 END) > 0");
    }
    builder.push(") t");
    if filtered_path && !filters.exclude_bounce {
        builder
            .push(
                " LEFT JOIN (SELECT session_id, visit_id, 1::int AS has_custom_event FROM website_event WHERE website_id = ",
            )
            .push_bind(website_id)
            .push(" AND created_at BETWEEN ")
            .push_bind(range.start)
            .push(" AND ")
            .push_bind(range.end)
            .push(
                " AND event_type = 2 GROUP BY session_id, visit_id) custom \
                 ON custom.session_id = t.session_id AND custom.visit_id = t.visit_id",
            );
    }
    let row =
        builder.build().fetch_one(pool).await.map_err(|error| {
            error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
        })?;
    Ok(json!({
        "pageviews": row.get::<i64, _>("pageviews"),
        "visitors": row.get::<i64, _>("visitors"),
        "visits": row.get::<i64, _>("visits"),
        "bounces": row.get::<i64, _>("bounces"),
        "totaltime": row.get::<i64, _>("totaltime")
    }))
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

pub(super) async fn handle_website_stats(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(mut params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match crate::access::website_section_pool(
        &state,
        &headers,
        website_id,
        &[
            crate::access::ShareSection::Overview,
            crate::access::ShareSection::Compare,
        ],
        crate::access::ShareDataVisibility::Aggregate,
    )
    .await
    {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let has_event_properties = params.keys().any(|key| {
        key.strip_prefix("epf")
            .is_some_and(|suffix| !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()))
    });
    let audience = match Audience::resolve(&pool, website_id, &mut params).await {
        Ok(audience) => audience,
        Err(response) => return response,
    };
    let range = match request_date_range(&pool, website_id, &params).await {
        Ok(range) => range,
        Err(response) => return response,
    };
    let compare = params.get("compare").map(String::as_str).unwrap_or("prev");
    if !matches!(compare, "prev" | "yoy") {
        return error_response(StatusCode::BAD_REQUEST, "Invalid comparison");
    }
    let current = match query_stats_filtered(
        &pool,
        website_id,
        range,
        &params,
        &audience,
        has_event_properties,
    )
    .await
    {
        Ok(value) => value,
        Err(response) => return response,
    };
    let comparison = match query_stats_filtered(
        &pool,
        website_id,
        comparison_range(range, compare, Utc::now()),
        &params,
        &audience,
        has_event_properties,
    )
    .await
    {
        Ok(value) => value,
        Err(response) => return response,
    };
    let mut output = current.as_object().cloned().unwrap_or_default();
    output.insert("comparison".to_string(), comparison);
    with_rust_header(Json(Value::Object(output)).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previous_comparison_keeps_inclusive_window() {
        let start = Utc.timestamp_millis_opt(10_000).unwrap();
        let end = Utc.timestamp_millis_opt(19_999).unwrap();
        let range = comparison_range(
            DateRange { start, end },
            "prev",
            Utc.timestamp_millis_opt(20_000).unwrap(),
        );
        assert_eq!(range.start.timestamp_millis(), 0);
        assert_eq!(range.end.timestamp_millis(), 9_999);
    }

    #[test]
    fn filter_names_are_whitelisted() {
        assert!(filter_column("browser").is_some());
        assert!(filter_column("screen").is_some());
        assert!(filter_column("entry").is_some());
        assert!(filter_column("madeUpColumn").is_none());
    }

    #[test]
    fn stored_segments_validate_and_preserve_match() {
        let filters = stored_segment_filters(
            &json!({
                "filters": [
                    { "name": "path", "operator": "eq", "value": "/pricing" },
                    { "name": "path", "operator": "c", "value": "/docs" }
                ],
                "match": "any"
            }),
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(
            filters,
            [
                ("path".to_string(), "eq./pricing".to_string()),
                ("path1".to_string(), "c./docs".to_string()),
                ("match".to_string(), "any".to_string())
            ]
        );
        assert!(stored_segment_filters(
            &json!({
                "filters": [{ "name": "timezone", "operator": "eq", "value": "UTC" }]
            }),
            &HashMap::new()
        )
        .is_err());
        assert!(stored_segment_filters(
            &json!({
                "filters": [{ "name": "path", "operator": "gt", "value": "/pricing" }]
            }),
            &HashMap::new()
        )
        .is_err());
        assert!(stored_segment_filters(&json!({ "dateRange": "30day" }), &HashMap::new()).is_err());
    }

    #[test]
    fn stored_session_properties_merge_without_overwriting_request_filters() {
        let filters = stored_segment_filters(
            &json!({
                "sessionPropertyFilters": [
                    {
                        "propertyName": "user.id",
                        "dataType": 1,
                        "operator": "eq",
                        "value": "abc.123"
                    },
                    {
                        "propertyName": "created at",
                        "dataType": 4,
                        "operator": "bf",
                        "value": "2026-07-01"
                    }
                ]
            }),
            &HashMap::from([
                ("spf0".to_string(), "1.eq.country.US".to_string()),
                ("spf2".to_string(), "1.eq.plan.pro".to_string()),
            ]),
        )
        .unwrap();
        assert_eq!(
            filters,
            [
                ("spf1".to_string(), "1.eq.user%2Eid.abc.123".to_string()),
                (
                    "spf3".to_string(),
                    "4.bf.created%20at.2026-07-01".to_string()
                )
            ]
        );
        assert!(stored_segment_filters(
            &json!({
                "sessionPropertyFilters": [{
                    "propertyName": "plan",
                    "dataType": 1,
                    "operator": "unknown",
                    "value": "pro"
                }]
            }),
            &HashMap::new()
        )
        .is_err());
    }
}
