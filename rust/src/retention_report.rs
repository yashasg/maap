use crate::{
    access::{website_section_pool, ShareDataVisibility, ShareSection},
    error_response, json_response,
    lifecycle::{push_identified_events_cte, resolve_scope, LifecycleScope},
    AppState,
};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::get,
    Router,
};
use serde_json::{json, Value};
use sqlx::{postgres::PgRow, Postgres, QueryBuilder, Row};
use std::{collections::HashMap, time::Instant};
use uuid::Uuid;

pub(crate) fn register_routes(router: Router<AppState>) -> Router<AppState> {
    router.route(
        "/api/websites/{website_id}/app-reports/retention",
        get(retention),
    )
}

#[derive(Debug)]
struct ReportRow {
    section: String,
    metric: String,
    bucket: String,
    count: i64,
}

impl ReportRow {
    fn from_row(row: PgRow) -> Self {
        Self {
            section: row.get("section"),
            metric: row.get("metric"),
            bucket: row.get("bucket"),
            count: row.get("count"),
        }
    }
}

fn push_returning_condition(
    query: &mut QueryBuilder<'_, Postgres>,
    scope: &LifecycleScope,
    unit: &'static str,
) {
    query
        .push(" first_seen.first_seen < (date_trunc('")
        .push(unit)
        .push("', ie.created_at AT TIME ZONE ")
        .push_bind(scope.timezone.clone())
        .push(") AT TIME ZONE ")
        .push_bind(scope.timezone.clone())
        .push(")");
}

fn push_series(
    query: &mut QueryBuilder<'_, Postgres>,
    scope: &LifecycleScope,
    metric: &'static str,
    unit: &'static str,
    start: chrono::DateTime<chrono::Utc>,
) {
    let format = match unit {
        "hour" => r#"YYYY-MM-DD"T"HH24:00:00"Z""#,
        "day" | "week" | "month" => r#"YYYY-MM-DD"T"00:00:00"Z""#,
        _ => unreachable!("validated retention grain"),
    };
    query
        .push("SELECT 'series'::text AS section, ")
        .push_bind(metric)
        .push("::text AS metric, to_char(date_trunc('")
        .push(unit)
        .push("', ie.created_at AT TIME ZONE ")
        .push_bind(scope.timezone.clone())
        .push("), '")
        .push(format)
        .push(
            r#"') AS bucket,
            count(DISTINCT ie.user_id)::bigint AS count
        FROM identified_events ie
        JOIN first_seen_users first_seen USING (user_id)
        WHERE ie.created_at >= "#,
        )
        .push_bind(start)
        .push(" AND ie.created_at <= ")
        .push_bind(scope.end)
        .push(" AND ");
    push_returning_condition(query, scope, unit);
    query.push(" GROUP BY 3");
}

fn push_pattern(
    query: &mut QueryBuilder<'_, Postgres>,
    scope: &LifecycleScope,
    metric: &'static str,
    bucket: &'static str,
    start: chrono::DateTime<chrono::Utc>,
) {
    query
        .push("SELECT 'pattern'::text AS section, ")
        .push_bind(metric)
        .push("::text AS metric, ");
    match bucket {
        "hour" => {
            query
                .push("extract(hour FROM ie.created_at AT TIME ZONE ")
                .push_bind(scope.timezone.clone())
                .push(")::integer::text");
        }
        "dayOfWeek" => {
            query
                .push("extract(isodow FROM ie.created_at AT TIME ZONE ")
                .push_bind(scope.timezone.clone())
                .push(")::integer::text");
        }
        "weekpart" => {
            query
                .push("CASE WHEN extract(isodow FROM ie.created_at AT TIME ZONE ")
                .push_bind(scope.timezone.clone())
                .push(") IN (6, 7) THEN 'Weekend' ELSE 'Weekday' END");
        }
        _ => unreachable!("validated retention pattern"),
    }
    query
        .push(" AS bucket, count(DISTINCT ie.user_id)::bigint AS count ")
        .push("FROM daily_returning_events ie WHERE ie.created_at >= ")
        .push_bind(start)
        .push(" AND ie.created_at <= ")
        .push_bind(scope.end)
        .push(" GROUP BY 3");
}

fn push_breakdown(
    query: &mut QueryBuilder<'_, Postgres>,
    scope: &LifecycleScope,
    metric: &'static str,
    field: &'static str,
) {
    query
        .push("SELECT 'breakdown'::text AS section, ")
        .push_bind(metric)
        .push("::text AS metric, COALESCE(NULLIF(ie.")
        .push(field)
        .push(
            r#", ''), 'Unknown')::text AS bucket,
            count(DISTINCT ie.user_id)::bigint AS count
        FROM latest_returning_users ie
        WHERE ie.created_at >= "#,
        )
        .push_bind(scope.start)
        .push(" AND ie.created_at <= ")
        .push_bind(scope.end)
        .push(" GROUP BY 3");
}

fn retention_query(scope: &LifecycleScope) -> QueryBuilder<'static, Postgres> {
    let mut query = QueryBuilder::new("");
    let last_24_hours = std::cmp::max(scope.start, scope.end - chrono::Duration::hours(24));
    let last_28_days = std::cmp::max(scope.start, scope.end - chrono::Duration::days(28));
    let last_93_days = std::cmp::max(scope.start, scope.end - chrono::Duration::days(93));
    let last_366_days = std::cmp::max(scope.start, scope.end - chrono::Duration::days(366));
    push_identified_events_cte(&mut query, scope);
    query
        .push(
            r#", daily_returning_events AS MATERIALIZED (
                SELECT ie.*
                FROM identified_events ie
                JOIN first_seen_users first_seen USING (user_id)
                WHERE ie.created_at >= "#,
        )
        .push_bind(scope.start)
        .push(" AND ie.created_at <= ")
        .push_bind(scope.end)
        .push(" AND ");
    push_returning_condition(&mut query, scope, "day");
    query.push(
        r#"
            ),
            latest_returning_users AS MATERIALIZED (
                SELECT DISTINCT ON (user_id)
                    user_id, event_id, created_at, device, platform, language
                FROM daily_returning_events
                ORDER BY user_id, created_at DESC, event_id DESC
            )
            "#,
    );
    push_series(&mut query, scope, "hourly", "hour", last_24_hours);
    query.push(" UNION ALL ");
    push_series(&mut query, scope, "daily", "day", scope.start);
    query.push(" UNION ALL ");
    push_series(&mut query, scope, "weekly", "week", last_93_days);
    query.push(" UNION ALL ");
    push_series(&mut query, scope, "monthly", "month", last_366_days);

    for (metric, bucket, start) in [
        ("hourOfDay", "hour", scope.start),
        ("hourOfDayFourWeeks", "hour", last_28_days),
        ("dayOfWeek", "dayOfWeek", scope.start),
        ("dayOfWeekFourWeeks", "dayOfWeek", last_28_days),
        ("weekpart", "weekpart", scope.start),
        ("weekpartFourWeeks", "weekpart", last_28_days),
    ] {
        query.push(" UNION ALL ");
        push_pattern(&mut query, scope, metric, bucket, start);
    }

    for (metric, field) in [
        ("device", "device"),
        ("platform", "platform"),
        ("language", "language"),
    ] {
        query.push(" UNION ALL ");
        push_breakdown(&mut query, scope, metric, field);
    }

    query
        .push(
            r#" UNION ALL
            SELECT 'scope'::text, 'totalSignals'::text, ''::text, count(*)::bigint
            FROM website_event e
            WHERE e.website_id = "#,
        )
        .push_bind(scope.website_id)
        .push(" AND e.created_at >= ")
        .push_bind(scope.start)
        .push(" AND e.created_at <= ")
        .push_bind(scope.end)
        .push(" AND ((")
        .push_bind(scope.test_mode)
        .push(" AND e.tag = 'test') OR (NOT ")
        .push_bind(scope.test_mode)
        .push(" AND COALESCE(e.tag, '') <> 'test'))")
        .push(
            r#" UNION ALL
            SELECT 'scope'::text, 'identifiedSignals'::text, ''::text, count(*)::bigint
            FROM identified_events ie
            WHERE ie.created_at >= "#,
        )
        .push_bind(scope.start)
        .push(" AND ie.created_at <= ")
        .push_bind(scope.end)
        .push(
            r#" UNION ALL
            SELECT 'scope'::text, 'returningUsers'::text, ''::text, count(DISTINCT ie.user_id)::bigint
            FROM daily_returning_events ie"#,
        );
    query.push(" ORDER BY section, metric, bucket");
    query
}

async fn retention(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match website_section_pool(
        &state,
        &headers,
        website_id,
        &[ShareSection::Retention],
        ShareDataVisibility::Aggregate,
    )
    .await
    {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let scope = match resolve_scope(&pool, website_id, &params).await {
        Ok(scope) => scope,
        Err(response) => return response,
    };

    let started = Instant::now();
    let rows = match retention_query(&scope).build().fetch_all(&pool).await {
        Ok(rows) => rows
            .into_iter()
            .map(ReportRow::from_row)
            .collect::<Vec<_>>(),
        Err(error) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string());
        }
    };
    let query_ms = started.elapsed().as_secs_f64() * 1000.0;
    json_response(response_json(&scope, rows, query_ms))
}

fn response_json(scope: &LifecycleScope, rows: Vec<ReportRow>, query_ms: f64) -> Value {
    let mut series: HashMap<String, Vec<Value>> = HashMap::new();
    let mut patterns: HashMap<String, Vec<Value>> = HashMap::new();
    let mut breakdowns: HashMap<String, Vec<Value>> = HashMap::new();
    let mut scope_counts: HashMap<String, i64> = HashMap::new();

    for row in rows {
        match row.section.as_str() {
            "series" => series.entry(row.metric).or_default().push(json!({
                "bucket": row.bucket,
                "returningUsers": row.count,
            })),
            "pattern" => patterns.entry(row.metric).or_default().push(json!({
                "bucket": row.bucket,
                "users": row.count,
            })),
            "breakdown" => breakdowns.entry(row.metric).or_default().push(json!({
                "name": row.bucket,
                "users": row.count,
            })),
            "scope" => {
                scope_counts.insert(row.metric, row.count);
            }
            _ => {}
        }
    }

    json!({
        "criteria": {
            "verified": false,
            "rule": "An identified user has activity in a time bucket and their first stored signal is before that bucket began.",
            "verification": "TelemetryDeck parity has not been independently verified."
        },
        "identity": {
            "unit": "appUser",
            "source": "clientUser",
            "unidentifiedEventsExcluded": true
        },
        "range": {
            "startAt": scope.start,
            "endAt": scope.end,
            "timezone": scope.timezone,
            "testMode": scope.test_mode
        },
        "scope": {
            "totalSignals": scope_counts.remove("totalSignals").unwrap_or_default(),
            "identifiedSignals": scope_counts.remove("identifiedSignals").unwrap_or_default(),
            "returningUsers": scope_counts.remove("returningUsers").unwrap_or_default()
        },
        "series": {
            "hourly": series.remove("hourly").unwrap_or_default(),
            "daily": series.remove("daily").unwrap_or_default(),
            "weekly": series.remove("weekly").unwrap_or_default(),
            "monthly": series.remove("monthly").unwrap_or_default()
        },
        "patterns": {
            "hourOfDay": patterns.remove("hourOfDay").unwrap_or_default(),
            "hourOfDayFourWeeks": patterns.remove("hourOfDayFourWeeks").unwrap_or_default(),
            "dayOfWeek": patterns.remove("dayOfWeek").unwrap_or_default(),
            "dayOfWeekFourWeeks": patterns.remove("dayOfWeekFourWeeks").unwrap_or_default(),
            "weekpart": patterns.remove("weekpart").unwrap_or_default(),
            "weekpartFourWeeks": patterns.remove("weekpartFourWeeks").unwrap_or_default()
        },
        "breakdowns": {
            "device": breakdowns.remove("device").unwrap_or_default(),
            "platform": breakdowns.remove("platform").unwrap_or_default(),
            "region": breakdowns.remove("region").unwrap_or_default(),
            "language": breakdowns.remove("language").unwrap_or_default()
        },
        "availability": {
            "userLocalTime": false,
            "preferredRegion": false,
            "reason": "The SDK user's local timezone and preferred region are not stored per signal."
        },
        "query": {
            "milliseconds": query_ms,
            "grains": {
                "hourly": { "unit": "hour", "window": "last24HoursWithinSelectedRange" },
                "daily": { "unit": "day", "window": "selectedRange" },
                "weekly": { "unit": "week", "window": "last93DaysWithinSelectedRange" },
                "monthly": { "unit": "month", "window": "last366DaysWithinSelectedRange" }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn scope() -> LifecycleScope {
        LifecycleScope {
            website_id: Uuid::nil(),
            start: Utc.timestamp_opt(1_700_000_000, 0).single().unwrap(),
            end: Utc.timestamp_opt(1_700_086_400, 0).single().unwrap(),
            minimum: None,
            timezone: "America/Los_Angeles".into(),
            test_mode: false,
        }
    }

    #[test]
    fn returning_users_are_distinct_sdk_users_seen_before_the_bucket() {
        let query = retention_query(&scope());
        let sql = query.sql();

        assert!(sql.contains("count(DISTINCT ie.user_id)"));
        assert!(sql.contains("first_seen.first_seen < (date_trunc"));
        assert!(sql.contains("date_trunc('week'"));
        assert!(!sql.contains("count(DISTINCT ie.session_id)"));
        assert!(!sql.contains("percentage"));
        assert!(!sql.to_ascii_lowercase().contains("sum("));
    }

    #[test]
    fn response_marks_the_return_rule_unverified() {
        let body = response_json(&scope(), vec![], 8.0);

        assert_eq!(body["criteria"]["verified"], false);
        assert_eq!(body["identity"]["source"], "clientUser");
        assert_eq!(body["range"]["testMode"], false);
        assert_eq!(body["query"]["milliseconds"], 8.0);
    }
}
