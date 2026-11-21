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
use chrono::Months;
use serde_json::{json, Value};
use sqlx::{postgres::PgRow, QueryBuilder, Row};
use std::{collections::HashMap, time::Instant};
use uuid::Uuid;

pub(crate) fn register_routes(router: Router<AppState>) -> Router<AppState> {
    router.route(
        "/api/websites/{website_id}/app-reports/acquisition",
        get(acquisition),
    )
}

#[derive(Debug)]
struct ReportRow {
    section: String,
    metric: String,
    bucket: String,
    active_users: Option<i64>,
    new_users: Option<i64>,
    users: Option<i64>,
}

impl ReportRow {
    fn from_row(row: PgRow) -> Self {
        Self {
            section: row.get("section"),
            metric: row.get("metric"),
            bucket: row.get("bucket"),
            active_users: row.get("active_users"),
            new_users: row.get("new_users"),
            users: row.get("users"),
        }
    }
}

fn push_breakdown(
    query: &mut QueryBuilder<'_, sqlx::Postgres>,
    scope: &LifecycleScope,
    metric: &'static str,
    field: &'static str,
) {
    query
        .push("SELECT 'breakdown'::text AS section, ")
        .push_bind(metric)
        .push("::text AS metric, COALESCE(NULLIF(")
        .push(field)
        .push(
            r#", ''), 'Unknown')::text AS bucket,
            NULL::bigint AS active_users,
            NULL::bigint AS new_users,
            count(*)::bigint AS users
        FROM first_seen_users
        WHERE first_seen >= "#,
        )
        .push_bind(scope.start)
        .push(" AND first_seen <= ")
        .push_bind(scope.end)
        .push(
            r#"
        GROUP BY 3
        "#,
        );
}

fn acquisition_query(scope: &LifecycleScope) -> QueryBuilder<'static, sqlx::Postgres> {
    let mut query = QueryBuilder::new("");
    push_identified_events_cte(&mut query, scope);
    let hourly_start = clamp_rolling_start(scope, scope.end - chrono::Duration::hours(24));
    let four_weeks_start = clamp_rolling_start(scope, scope.end - chrono::Duration::days(28));
    let weekly_start = clamp_rolling_start(
        scope,
        scope
            .end
            .checked_sub_months(Months::new(3))
            .unwrap_or(scope.end),
    );
    let monthly_start = clamp_rolling_start(
        scope,
        scope
            .end
            .checked_sub_months(Months::new(12))
            .unwrap_or(scope.end),
    );

    query
        .push(
            r#"SELECT
            'summary'::text AS section,
            'totalSignals'::text AS metric,
            ''::text AS bucket,
            NULL::bigint AS active_users,
            NULL::bigint AS new_users,
            count(*)::bigint AS users
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
            SELECT 'summary'::text, 'identifiedSignals'::text, ''::text,
            NULL::bigint, NULL::bigint, count(*)::bigint
            FROM identified_events
            WHERE created_at >= "#,
        )
        .push_bind(scope.start)
        .push(" AND created_at <= ")
        .push_bind(scope.end)
        .push(" UNION ALL ");

    push_complete_series(&mut query, scope, "hourly", "hour", hourly_start);
    query.push(" UNION ALL ");
    push_complete_series(&mut query, scope, "daily", "day", scope.start);
    query.push(" UNION ALL ");
    push_complete_series(&mut query, scope, "weekly", "week", weekly_start);
    query.push(" UNION ALL ");
    push_complete_series(&mut query, scope, "monthly", "month", monthly_start);
    for (metric, bucket_sql, start) in [
        (
            "hourOfDay",
            "extract(hour FROM first_seen AT TIME ZONE ",
            scope.start,
        ),
        (
            "hourOfDay4Weeks",
            "extract(hour FROM first_seen AT TIME ZONE ",
            four_weeks_start,
        ),
    ] {
        query
            .push(" UNION ALL SELECT 'pattern'::text, ")
            .push_bind(metric)
            .push("::text, ")
            .push(bucket_sql)
            .push_bind(scope.timezone.clone())
            .push(
                r#")::integer::text,
                NULL::bigint, NULL::bigint, count(*)::bigint
                FROM first_seen_users
                WHERE first_seen >= "#,
            )
            .push_bind(start)
            .push(" AND first_seen <= ")
            .push_bind(scope.end)
            .push(" GROUP BY 3");
    }
    for (metric, bucket_sql, start) in [
        (
            "dayOfWeek",
            "extract(isodow FROM first_seen AT TIME ZONE ",
            scope.start,
        ),
        (
            "dayOfWeek4Weeks",
            "extract(isodow FROM first_seen AT TIME ZONE ",
            four_weeks_start,
        ),
    ] {
        query
            .push(" UNION ALL SELECT 'pattern'::text, ")
            .push_bind(metric)
            .push("::text, ")
            .push(bucket_sql)
            .push_bind(scope.timezone.clone())
            .push(
                r#")::integer::text,
                NULL::bigint, NULL::bigint, count(*)::bigint
                FROM first_seen_users
                WHERE first_seen >= "#,
            )
            .push_bind(start)
            .push(" AND first_seen <= ")
            .push_bind(scope.end)
            .push(" GROUP BY 3");
    }
    for (metric, start) in [
        ("weekpart", scope.start),
        ("weekpart4Weeks", four_weeks_start),
    ] {
        query
            .push(" UNION ALL SELECT 'pattern'::text, ")
            .push_bind(metric)
            .push("::text, CASE WHEN extract(isodow FROM first_seen AT TIME ZONE ")
            .push_bind(scope.timezone.clone())
            .push(
                r#") IN (6, 7) THEN 'Weekend' ELSE 'Weekday' END,
                NULL::bigint, NULL::bigint, count(*)::bigint
                FROM first_seen_users
                WHERE first_seen >= "#,
            )
            .push_bind(start)
            .push(" AND first_seen <= ")
            .push_bind(scope.end)
            .push(" GROUP BY 3");
    }
    for (metric, field) in [
        ("device", "device"),
        ("platform", "platform"),
        ("region", "region"),
        ("language", "language"),
    ] {
        query.push(" UNION ALL ");
        push_breakdown(&mut query, scope, metric, field);
    }
    query.push(" ORDER BY section, metric, bucket");
    query
}

fn clamp_rolling_start(
    scope: &LifecycleScope,
    rolling_start: chrono::DateTime<chrono::Utc>,
) -> chrono::DateTime<chrono::Utc> {
    scope
        .minimum
        .map_or(rolling_start, |minimum| rolling_start.max(minimum))
}

fn push_complete_series(
    query: &mut QueryBuilder<'_, sqlx::Postgres>,
    scope: &LifecycleScope,
    metric: &'static str,
    unit: &'static str,
    start: chrono::DateTime<chrono::Utc>,
) {
    let format = match unit {
        "hour" => r#"YYYY-MM-DD"T"HH24:00:00"Z""#,
        "day" => r#"YYYY-MM-DD"T"00:00:00"Z""#,
        "week" | "month" => r#"YYYY-MM-DD"T"00:00:00"Z""#,
        _ => unreachable!("validated acquisition grain"),
    };
    query
        .push("SELECT 'series'::text, ")
        .push_bind(metric)
        .push("::text, to_char(date_trunc('")
        .push(unit)
        .push("', ie.created_at AT TIME ZONE ")
        .push_bind(scope.timezone.clone())
        .push("), '")
        .push(format)
        .push(
            r#"'),
            count(DISTINCT ie.user_id)::bigint,
            count(DISTINCT ie.user_id) FILTER (
                WHERE date_trunc('"#,
        )
        .push(unit)
        .push("', first_seen.first_seen AT TIME ZONE ")
        .push_bind(scope.timezone.clone())
        .push(") = date_trunc('")
        .push(unit)
        .push("', ie.created_at AT TIME ZONE ")
        .push_bind(scope.timezone.clone())
        .push(
            r#")
            )::bigint,
            NULL::bigint
        FROM identified_events ie
        JOIN first_seen_users first_seen USING (user_id)
        WHERE ie.created_at >= "#,
        )
        .push_bind(start)
        .push(" AND ie.created_at <= ")
        .push_bind(scope.end)
        .push(" GROUP BY 3");
}

async fn acquisition(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match website_section_pool(
        &state,
        &headers,
        website_id,
        &[ShareSection::Overview],
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
    let rows = match acquisition_query(&scope).build().fetch_all(&pool).await {
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
    let mut summary: HashMap<String, i64> = HashMap::new();

    for row in rows {
        match row.section.as_str() {
            "series" => series.entry(row.metric).or_default().push(json!({
                "bucket": row.bucket,
                "activeUsers": row.active_users.unwrap_or_default(),
                "newUsers": row.new_users.unwrap_or_default(),
            })),
            "pattern" => patterns.entry(row.metric).or_default().push(json!({
                "bucket": row.bucket,
                "users": row.users.unwrap_or_default(),
            })),
            "breakdown" => breakdowns.entry(row.metric).or_default().push(json!({
                "name": row.bucket,
                "users": row.users.unwrap_or_default(),
            })),
            "summary" => {
                summary.insert(row.metric, row.users.unwrap_or_default());
            }
            _ => {}
        }
    }

    json!({
        "identity": {
            "unit": "appUser",
            "source": "clientUser",
            "unidentifiedEventsExcluded": true
        },
        "range": {
            "startAt": scope.start,
            "endAt": scope.end,
            "timezone": scope.timezone,
            "testMode": scope.test_mode,
            "totalSignals": summary.remove("totalSignals").unwrap_or_default(),
            "identifiedSignals": summary.remove("identifiedSignals").unwrap_or_default()
        },
        "series": {
            "hourly": series.remove("hourly").unwrap_or_default(),
            "daily": series.remove("daily").unwrap_or_default(),
            "weekly": series.remove("weekly").unwrap_or_default(),
            "monthly": series.remove("monthly").unwrap_or_default()
        },
        "patterns": {
            "hourOfDay": patterns.remove("hourOfDay").unwrap_or_default(),
            "hourOfDay4Weeks": patterns.remove("hourOfDay4Weeks").unwrap_or_default(),
            "dayOfWeek": patterns.remove("dayOfWeek").unwrap_or_default(),
            "dayOfWeek4Weeks": patterns.remove("dayOfWeek4Weeks").unwrap_or_default(),
            "weekpart": patterns.remove("weekpart").unwrap_or_default(),
            "weekpart4Weeks": patterns.remove("weekpart4Weeks").unwrap_or_default()
        },
        "breakdowns": {
            "device": breakdowns.remove("device").unwrap_or_default(),
            "platform": breakdowns.remove("platform").unwrap_or_default(),
            "region": breakdowns.remove("region").unwrap_or_default(),
            "language": breakdowns.remove("language").unwrap_or_default()
        },
        "availability": {
            "userLocalTime": false,
            "reason": "The SDK user's local timezone is not stored per signal."
        },
        "query": {
            "milliseconds": query_ms,
            "grains": {
                "hourly": { "unit": "hour", "window": "last24Hours" },
                "daily": { "unit": "day", "window": "selectedRange" },
                "weekly": { "unit": "week", "window": "last3CalendarMonths" },
                "monthly": { "unit": "month", "window": "last12CalendarMonths" }
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
    fn acquisition_query_keeps_first_seen_outside_selected_range() {
        let query = acquisition_query(&scope());
        let sql = query.sql();
        let cte_end = sql.find("first_seen_users AS").unwrap();
        let identity_sql = &sql[..cte_end];

        assert!(!identity_sql.contains("created_at >="));
        assert!(sql.contains("first_seen.first_seen"));
        assert!(sql.contains("count(DISTINCT ie.user_id)"));
        assert!(!sql.contains("count(DISTINCT ie.session_id)"));
    }

    #[test]
    fn response_documents_query_grain_and_unavailable_user_local_time() {
        let body = response_json(&scope(), vec![], 12.5);
        assert_eq!(body["identity"]["source"], "clientUser");
        assert_eq!(body["query"]["grains"]["daily"]["unit"], "day");
        assert_eq!(body["availability"]["userLocalTime"], false);
        assert_eq!(body["query"]["milliseconds"], 12.5);
    }

    #[test]
    fn rolling_windows_clamp_to_policy_floor_not_selected_display_start() {
        let mut scoped = scope();
        scoped.minimum = Some(Utc.timestamp_opt(1_650_000_000, 0).single().unwrap());
        scoped.start = Utc.timestamp_opt(1_699_000_000, 0).single().unwrap();
        let older_window = Utc.timestamp_opt(1_600_000_000, 0).single().unwrap();
        let allowed_window = Utc.timestamp_opt(1_680_000_000, 0).single().unwrap();

        assert_eq!(
            clamp_rolling_start(&scoped, older_window),
            scoped.minimum.unwrap()
        );
        assert_eq!(clamp_rolling_start(&scoped, allowed_window), allowed_window);
        assert!(clamp_rolling_start(&scoped, allowed_window) < scoped.start);
    }
}
