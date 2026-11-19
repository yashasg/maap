use crate::error_response;
use axum::{http::StatusCode, response::Response};
use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;
use serde_json::Value;
use sqlx::{PgPool, Postgres, QueryBuilder, Row};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub(crate) struct Cohort {
    website_id: Uuid,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    action: Predicate,
    filters: Vec<Predicate>,
    match_any: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct StoredCohort {
    #[serde(default)]
    filters: Option<Vec<StoredFilter>>,
    #[serde(rename = "dateRange")]
    date_range: String,
    action: StoredAction,
    #[serde(rename = "match")]
    match_mode: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct StoredFilter {
    name: String,
    operator: String,
    value: Value,
}

#[derive(Clone, Debug, Deserialize)]
struct StoredAction {
    #[serde(rename = "type")]
    kind: String,
    value: String,
}

#[derive(Clone, Debug)]
struct Predicate {
    column: Column,
    operator: Operator,
    value: PredicateValue,
    always_and: bool,
    excludes_internal_referrer: bool,
}

#[derive(Clone, Copy, Debug)]
enum Column {
    Event(&'static str),
    Session(&'static str),
}

#[derive(Clone, Copy, Debug)]
enum Operator {
    Equals,
    NotEquals,
    Contains,
    DoesNotContain,
    Regex,
    NotRegex,
}

#[derive(Clone, Debug)]
enum PredicateValue {
    Text(Vec<String>),
    Integer(Vec<i32>),
}

impl Cohort {
    pub(crate) async fn take(
        pool: &PgPool,
        website_id: Uuid,
        params: &mut HashMap<String, String>,
    ) -> Result<Option<Self>, Response> {
        let Some(raw_id) = params.remove("cohort") else {
            return Ok(None);
        };
        let cohort_id = Uuid::parse_str(&raw_id)
            .map_err(|_| error_response(StatusCode::BAD_REQUEST, "Invalid cohort"))?;
        let row = sqlx::query(
            r#"
            SELECT parameters::text AS parameters
            FROM segment
            WHERE segment_id = $1
              AND website_id = $2
              AND type = 'cohort'
            "#,
        )
        .bind(cohort_id)
        .bind(website_id)
        .fetch_optional(pool)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, "Invalid cohort"))?;
        let stored: StoredCohort = serde_json::from_str(&row.get::<String, _>("parameters"))
            .map_err(|_| error_response(StatusCode::BAD_REQUEST, "Invalid cohort parameters"))?;
        let (start, end) = resolve_date_range(pool, &stored.date_range).await?;
        let match_any = match stored.match_mode.as_deref() {
            None | Some("all") => false,
            Some("any") => true,
            Some(_) => {
                return Err(error_response(
                    StatusCode::BAD_REQUEST,
                    "Invalid cohort match",
                ))
            }
        };
        let action = action_predicate(stored.action)?;
        let filters = stored
            .filters
            .unwrap_or_default()
            .into_iter()
            .map(filter_predicate)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|message| error_response(StatusCode::BAD_REQUEST, message))?;

        Ok(Some(Self {
            website_id,
            start,
            end,
            action,
            filters,
            match_any,
        }))
    }

    pub(crate) fn push(&self, query: &mut QueryBuilder<'_, Postgres>, event_alias: &'static str) {
        query
            .push(" AND ")
            .push(event_alias)
            .push(".website_id = ")
            .push_bind(self.website_id)
            .push(" AND ")
            .push(event_alias)
            .push(
                ".session_id IN (
                    SELECT DISTINCT ce.session_id
                    FROM website_event ce
                    JOIN session cs
                      ON cs.session_id = ce.session_id
                     AND cs.website_id = ce.website_id
                    WHERE ce.website_id = ",
            )
            .push_bind(self.website_id)
            .push(" AND ce.created_at BETWEEN ")
            .push_bind(self.start)
            .push(" AND ")
            .push_bind(self.end)
            .push(" AND ");
        push_predicate(query, &self.action);

        let (always, matched): (Vec<_>, Vec<_>) = self
            .filters
            .iter()
            .partition(|filter| filter.always_and || !self.match_any);
        if !matched.is_empty() {
            query.push(" AND (");
            for (index, filter) in matched.into_iter().enumerate() {
                if index > 0 {
                    query.push(" OR ");
                }
                push_predicate(query, filter);
            }
            query.push(")");
        }
        for filter in always {
            query.push(" AND ");
            push_predicate(query, filter);
        }
        if self
            .filters
            .iter()
            .any(|filter| filter.excludes_internal_referrer)
        {
            query.push(
                " AND (ce.referrer_domain != regexp_replace(ce.hostname, '^www.', '') \
                 OR ce.referrer_domain IS NULL)",
            );
        }
        query.push(")");
    }
}

fn internal_error(error: sqlx::Error) -> Response {
    error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}

fn action_predicate(action: StoredAction) -> Result<Predicate, Response> {
    if action.value.is_empty() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "Invalid cohort action",
        ));
    }
    let column = match action.kind.as_str() {
        "path" => Column::Event("url_path"),
        "event" => Column::Event("event_name"),
        _ => {
            return Err(error_response(
                StatusCode::BAD_REQUEST,
                "This cohort uses a retired action and must be updated.",
            ))
        }
    };
    Ok(Predicate {
        column,
        operator: Operator::Equals,
        value: PredicateValue::Text(action.value.split(',').map(str::to_owned).collect()),
        always_and: true,
        excludes_internal_referrer: false,
    })
}

fn filter_predicate(filter: StoredFilter) -> Result<Predicate, &'static str> {
    let (column, event_type, excludes_internal_referrer) =
        filter_column(&filter.name).ok_or("Invalid cohort filter name")?;
    let operator = match filter.operator.as_str() {
        "eq" => Operator::Equals,
        "neq" => Operator::NotEquals,
        "c" => Operator::Contains,
        "dnc" => Operator::DoesNotContain,
        "re" => Operator::Regex,
        "nre" => Operator::NotRegex,
        _ => return Err("Unsupported cohort filter operator"),
    };
    let raw = stored_value(&filter.value).ok_or("Invalid cohort filter value")?;
    let value = if event_type {
        if !matches!(operator, Operator::Equals | Operator::NotEquals) {
            return Err("Unsupported event type cohort filter operator");
        }
        PredicateValue::Integer(
            raw.split(',')
                .map(str::parse::<i32>)
                .collect::<Result<Vec<_>, _>>()
                .ok()
                .filter(|values| !values.is_empty() && values.iter().all(|value| *value > 0))
                .ok_or("Invalid event type cohort filter")?,
        )
    } else if matches!(operator, Operator::Equals | Operator::NotEquals) {
        PredicateValue::Text(raw.split(',').map(str::to_owned).collect())
    } else {
        PredicateValue::Text(vec![raw])
    };

    Ok(Predicate {
        column,
        operator,
        value,
        always_and: event_type,
        excludes_internal_referrer,
    })
}

fn stored_value(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Array(values) => values
            .iter()
            .map(stored_value)
            .collect::<Option<Vec<_>>>()
            .map(|values| values.join(",")),
        Value::Null | Value::Object(_) => None,
    }
}

fn filter_column(name: &str) -> Option<(Column, bool, bool)> {
    Some(match name {
        "path" | "entry" | "exit" => (Column::Event("url_path"), false, false),
        "referrer" => (Column::Event("referrer_domain"), false, true),
        "domain" => (Column::Event("referrer_domain"), false, false),
        "hostname" => (Column::Event("hostname"), false, false),
        "distinctId" => (Column::Session("distinct_id"), false, false),
        "title" => (Column::Event("page_title"), false, false),
        "query" => (Column::Event("url_query"), false, false),
        "os" => (Column::Session("os"), false, false),
        "browser" => (Column::Session("browser"), false, false),
        "device" => (Column::Session("device"), false, false),
        "country" => (Column::Session("country"), false, false),
        "region" => (Column::Session("region"), false, false),
        "city" => (Column::Session("city"), false, false),
        "language" => (Column::Session("language"), false, false),
        "event" => (Column::Event("event_name"), false, false),
        "tag" => (Column::Event("tag"), false, false),
        "eventType" => (Column::Event("event_type"), true, false),
        "utmSource" => (Column::Event("utm_source"), false, false),
        "utmMedium" => (Column::Event("utm_medium"), false, false),
        "utmCampaign" => (Column::Event("utm_campaign"), false, false),
        "utmContent" => (Column::Event("utm_content"), false, false),
        "utmTerm" => (Column::Event("utm_term"), false, false),
        _ => return None,
    })
}

fn push_predicate(query: &mut QueryBuilder<'_, Postgres>, predicate: &Predicate) {
    match (&predicate.operator, &predicate.value) {
        (Operator::Equals, PredicateValue::Text(values)) => {
            push_column(query, predicate.column);
            query.push(" = ANY(").push_bind(values.clone()).push(")");
        }
        (Operator::Equals, PredicateValue::Integer(values)) => {
            push_column(query, predicate.column);
            query.push(" = ANY(").push_bind(values.clone()).push(")");
        }
        (Operator::NotEquals, PredicateValue::Text(values)) => {
            query.push("COALESCE(");
            push_column(query, predicate.column);
            query
                .push(", '') != ALL(")
                .push_bind(values.clone())
                .push(")");
        }
        (Operator::NotEquals, PredicateValue::Integer(values)) => {
            query.push("COALESCE(");
            push_column(query, predicate.column);
            query
                .push(", 0) != ALL(")
                .push_bind(values.clone())
                .push(")");
        }
        (Operator::Contains, PredicateValue::Text(values)) => {
            push_column(query, predicate.column);
            query.push(" ILIKE ").push_bind(format!("%{}%", values[0]));
        }
        (Operator::DoesNotContain, PredicateValue::Text(values)) => {
            query.push("COALESCE(");
            push_column(query, predicate.column);
            query
                .push(", '') NOT ILIKE ")
                .push_bind(format!("%{}%", values[0]));
        }
        (Operator::Regex, PredicateValue::Text(values)) => {
            push_column(query, predicate.column);
            query.push(" ~* ").push_bind(values[0].clone());
        }
        (Operator::NotRegex, PredicateValue::Text(values)) => {
            query.push("COALESCE(");
            push_column(query, predicate.column);
            query.push(", '') !~* ").push_bind(values[0].clone());
        }
        _ => unreachable!("integer cohort filters only support equality operators"),
    }
}

fn push_column(query: &mut QueryBuilder<'_, Postgres>, column: Column) {
    match column {
        Column::Event(column) => {
            query.push("ce.").push(column);
        }
        Column::Session(column) => {
            query.push("cs.").push(column);
        }
    }
}

async fn resolve_date_range(
    pool: &PgPool,
    value: &str,
) -> Result<(DateTime<Utc>, DateTime<Utc>), Response> {
    if let Some(range) = fixed_date_range(value) {
        return range.map_err(|message| error_response(StatusCode::BAD_REQUEST, message));
    }
    let (sql, amount) = relative_date_range(value)
        .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, "Invalid cohort date range"))?;
    // Node uses process-local time here; the self-hosted Docker server contract is UTC.
    let mut query = sqlx::query(sql).bind("UTC");
    if let Some(amount) = amount {
        query = query.bind(amount);
    }
    let row = query.fetch_one(pool).await.map_err(internal_error)?;
    Ok((row.get("start_date"), row.get("end_date")))
}

fn fixed_date_range(value: &str) -> Option<Result<(DateTime<Utc>, DateTime<Utc>), &'static str>> {
    let rest = value.strip_prefix("range:")?;
    Some((|| {
        let mut parts = rest.split(':');
        let start = parts
            .next()
            .filter(|part| !part.is_empty())
            .and_then(timestamp_millis)
            .ok_or("Invalid cohort date range")?;
        let end = parts
            .next()
            .filter(|part| !part.is_empty())
            .and_then(timestamp_millis)
            .ok_or("Invalid cohort date range")?;
        if parts.next().is_some() {
            return Err("Invalid cohort date range");
        }
        Ok((start, end))
    })())
}

fn timestamp_millis(value: &str) -> Option<DateTime<Utc>> {
    let value = value.parse::<f64>().ok()?;
    if !value.is_finite() || value < i64::MIN as f64 || value > i64::MAX as f64 {
        return None;
    }
    Utc.timestamp_millis_opt(value as i64).single()
}

fn relative_date_range(value: &str) -> Option<(&'static str, Option<i64>)> {
    match value {
        "24h" => return Some((SLIDING_24_HOURS, None)),
        "7d" => return Some((SLIDING_7_DAYS, None)),
        "30d" => return Some((SLIDING_30_DAYS, None)),
        "today" => return Some((CALENDAR_DAY, Some(0))),
        _ => {}
    }
    for (unit, sql) in [
        ("hour", CALENDAR_HOUR),
        ("day", CALENDAR_DAY),
        ("week", CALENDAR_WEEK),
        ("month", CALENDAR_MONTH),
        ("year", CALENDAR_YEAR),
    ] {
        if let Some(amount) = value
            .strip_suffix(unit)
            .filter(|amount| {
                !amount.is_empty()
                    && amount
                        .chars()
                        .all(|character| character.is_ascii_digit() || character == '-')
            })
            .and_then(|amount| amount.parse::<i64>().ok())
        {
            return Some((sql, Some(amount)));
        }
    }
    None
}

const SLIDING_24_HOURS: &str = r#"
    SELECT CURRENT_TIMESTAMP - INTERVAL '24 hours' AS start_date,
           CURRENT_TIMESTAMP AS end_date,
           $1::text
"#;
const SLIDING_7_DAYS: &str = r#"
    SELECT CURRENT_TIMESTAMP - INTERVAL '7 days' AS start_date,
           CURRENT_TIMESTAMP AS end_date,
           $1::text
"#;
const SLIDING_30_DAYS: &str = r#"
    SELECT CURRENT_TIMESTAMP - INTERVAL '30 days' AS start_date,
           CURRENT_TIMESTAMP AS end_date,
           $1::text
"#;
const CALENDAR_HOUR: &str = r#"
    SELECT
      (date_trunc('hour', CURRENT_TIMESTAMP AT TIME ZONE $1)
        - ($2::bigint * INTERVAL '1 hour')) AT TIME ZONE $1 AS start_date,
      (date_trunc('hour', CURRENT_TIMESTAMP AT TIME ZONE $1)
        + INTERVAL '1 hour' - INTERVAL '1 millisecond') AT TIME ZONE $1 AS end_date
"#;
const CALENDAR_DAY: &str = r#"
    SELECT
      (date_trunc('day', CURRENT_TIMESTAMP AT TIME ZONE $1)
        - ($2::bigint * INTERVAL '1 day')) AT TIME ZONE $1 AS start_date,
      (date_trunc('day', CURRENT_TIMESTAMP AT TIME ZONE $1)
        + INTERVAL '1 day' - INTERVAL '1 millisecond') AT TIME ZONE $1 AS end_date
"#;
const CALENDAR_WEEK: &str = r#"
    SELECT
      (date_trunc('week', (CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 day')
        - INTERVAL '1 day' - ($2::bigint * INTERVAL '1 week')) AT TIME ZONE $1 AS start_date,
      (date_trunc('week', (CURRENT_TIMESTAMP AT TIME ZONE $1) + INTERVAL '1 day')
        + INTERVAL '6 days 23 hours 59 minutes 59.999 seconds') AT TIME ZONE $1 AS end_date
"#;
const CALENDAR_MONTH: &str = r#"
    SELECT
      (date_trunc('month', CURRENT_TIMESTAMP AT TIME ZONE $1)
        - ($2::bigint * INTERVAL '1 month')) AT TIME ZONE $1 AS start_date,
      (date_trunc('month', CURRENT_TIMESTAMP AT TIME ZONE $1)
        + INTERVAL '1 month' - INTERVAL '1 millisecond') AT TIME ZONE $1 AS end_date
"#;
const CALENDAR_YEAR: &str = r#"
    SELECT
      (date_trunc('year', CURRENT_TIMESTAMP AT TIME ZONE $1)
        - ($2::bigint * INTERVAL '1 year')) AT TIME ZONE $1 AS start_date,
      (date_trunc('year', CURRENT_TIMESTAMP AT TIME ZONE $1)
        + INTERVAL '1 year' - INTERVAL '1 millisecond') AT TIME ZONE $1 AS end_date
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn text_filter(name: &str, operator: &str, value: Value) -> Result<Predicate, &'static str> {
        filter_predicate(StoredFilter {
            name: name.to_string(),
            operator: operator.to_string(),
            value,
        })
    }

    #[test]
    fn parses_fixed_and_relative_date_ranges() {
        let start = 1_700_000_000_000;
        let end = start + 1000;
        let (parsed_start, parsed_end) = fixed_date_range(&format!("range:{start}:{end}"))
            .unwrap()
            .unwrap();
        assert_eq!(parsed_start.timestamp_millis(), start);
        assert_eq!(parsed_end.timestamp_millis(), end);
        assert!(fixed_date_range("range:2:1").unwrap().is_ok());
        assert!(fixed_date_range("range:NaN:2").unwrap().is_err());

        assert_eq!(relative_date_range("30d").unwrap().0, SLIDING_30_DAYS);
        assert_eq!(relative_date_range("0month").unwrap().1, Some(0));
        assert_eq!(relative_date_range("6month").unwrap().1, Some(6));
        assert_eq!(relative_date_range("today").unwrap().1, Some(0));
        assert!(relative_date_range("+1day").is_none());
        assert!(relative_date_range("all").is_none());
    }

    #[test]
    fn whitelists_columns_and_preserves_repeated_session_filters() {
        assert!(text_filter("timezone", "eq", Value::String("UTC".into())).is_err());
        assert!(text_filter("screen", "eq", Value::String("1920x1080".into())).is_err());
        assert!(text_filter("browser", "s", Value::String(String::new())).is_err());

        let cohort = Cohort {
            website_id: Uuid::nil(),
            start: Utc.timestamp_millis_opt(1).unwrap(),
            end: Utc.timestamp_millis_opt(2).unwrap(),
            action: action_predicate(StoredAction {
                kind: "event".into(),
                value: "signup".into(),
            })
            .unwrap(),
            filters: vec![
                text_filter("browser", "eq", Value::String("Chrome".into())).unwrap(),
                text_filter("browser", "neq", Value::String("Firefox".into())).unwrap(),
            ],
            match_any: false,
        };
        let mut query = QueryBuilder::<Postgres>::new("SELECT 1 FROM website_event e WHERE TRUE");
        cohort.push(&mut query, "e");
        let sql = query.sql();
        assert_eq!(sql.matches("cs.browser").count(), 2);
        assert!(sql.contains("JOIN session cs"));
        assert!(sql.contains("ce.website_id = $"));
        assert!(sql.contains("e.website_id = $"));
        assert!(sql.contains("e.session_id IN"));
    }

    #[test]
    fn binds_values_and_keeps_action_outside_any_match() {
        let attack = "x') OR TRUE --";
        let cohort = Cohort {
            website_id: Uuid::nil(),
            start: Utc.timestamp_millis_opt(1).unwrap(),
            end: Utc.timestamp_millis_opt(2).unwrap(),
            action: action_predicate(StoredAction {
                kind: "path".into(),
                value: "/signup".into(),
            })
            .unwrap(),
            filters: vec![
                text_filter("country", "eq", Value::String("US".into())).unwrap(),
                text_filter("title", "re", Value::String(attack.into())).unwrap(),
                text_filter("eventType", "eq", Value::String("2".into())).unwrap(),
            ],
            match_any: true,
        };
        let mut query = QueryBuilder::<Postgres>::new("SELECT 1 FROM website_event e WHERE TRUE");
        cohort.push(&mut query, "e");
        let sql = query.sql();
        assert!(!sql.contains(attack));
        assert!(sql.contains("AND ce.url_path = ANY("));
        assert!(sql.contains("AND (cs.country = ANY("));
        assert!(sql.contains(" OR ce.page_title ~* "));
        let event_type = sql.find("AND ce.event_type = ANY(").unwrap();
        let any_group_end = sql.find(") AND ce.event_type").unwrap();
        assert!(event_type >= any_group_end);
    }
}
