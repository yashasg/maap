use crate::{
    access::{website_pool, Access},
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
use sqlx::{postgres::PgRow, Postgres, QueryBuilder, Row};
use std::{collections::HashMap, time::Instant};
use uuid::Uuid;

const TOTAL_SESSIONS_KEY: &str = "TelemetryDeck.Retention.totalSessionsCount";
const AVERAGE_SECONDS_KEY: &str = "TelemetryDeck.Retention.averageSessionSeconds";
const HOUR_OF_DAY_KEY: &str = "TelemetryDeck.Calendar.hourOfDay";
const DAY_OF_WEEK_KEY: &str = "TelemetryDeck.Calendar.dayOfWeek";
const IS_WEEKEND_KEY: &str = "TelemetryDeck.Calendar.isWeekend";
const PREFERRED_REGION_KEY: &str = "TelemetryDeck.UserPreference.region";

pub(crate) fn register_routes(router: Router<AppState>) -> Router<AppState> {
    router.route(
        "/api/websites/{website_id}/app-reports/activation",
        get(activation),
    )
}

#[derive(Debug)]
struct ReportRow {
    section: String,
    metric: String,
    bucket: String,
    users: i64,
}

impl ReportRow {
    fn from_row(row: PgRow) -> Self {
        Self {
            section: row.get("section"),
            metric: row.get("metric"),
            bucket: row.get("bucket"),
            users: row.get("users"),
        }
    }
}

fn push_activation_ctes(query: &mut QueryBuilder<'_, Postgres>, scope: &LifecycleScope) {
    push_identified_events_cte(query, scope);
    query
        .push(
            r#",
            activation_inputs AS MATERIALIZED (
                SELECT
                    ie.user_id,
                    ie.event_id,
                    ie.created_at,
                    ie.device,
                    ie.platform,
                    ie.language,
                    max(ed.string_value) FILTER (
                        WHERE ed.data_key = "#,
        )
        .push_bind(TOTAL_SESSIONS_KEY)
        .push(
            r#"
                    ) AS total_sessions_text,
                    max(ed.string_value) FILTER (
                        WHERE ed.data_key = "#,
        )
        .push_bind(AVERAGE_SECONDS_KEY)
        .push(
            r#"
                    ) AS average_seconds_text,
                    max(ed.string_value) FILTER (
                        WHERE ed.data_key = "#,
        )
        .push_bind(HOUR_OF_DAY_KEY)
        .push(
            r#"
                    ) AS user_hour_text,
                    max(ed.string_value) FILTER (
                        WHERE ed.data_key = "#,
        )
        .push_bind(DAY_OF_WEEK_KEY)
        .push(
            r#"
                    ) AS user_day_text,
                    max(ed.string_value) FILTER (
                        WHERE ed.data_key = "#,
        )
        .push_bind(IS_WEEKEND_KEY)
        .push(
            r#"
                    ) AS user_weekend_text,
                    max(ed.string_value) FILTER (
                        WHERE ed.data_key = "#,
        )
        .push_bind(PREFERRED_REGION_KEY)
        .push(
            r#"
                    ) AS preferred_region
                FROM identified_events ie
                LEFT JOIN event_data ed
                  ON ed.website_id = "#,
        )
        .push_bind(scope.website_id)
        .push(
            r#"
                 AND ed.website_event_id = ie.event_id
                 AND ed.data_key IN ("#,
        )
        .push_bind(TOTAL_SESSIONS_KEY)
        .push(", ")
        .push_bind(AVERAGE_SECONDS_KEY)
        .push(", ")
        .push_bind(HOUR_OF_DAY_KEY)
        .push(", ")
        .push_bind(DAY_OF_WEEK_KEY)
        .push(", ")
        .push_bind(IS_WEEKEND_KEY)
        .push(", ")
        .push_bind(PREFERRED_REGION_KEY)
        .push(
            r#")
                GROUP BY
                    ie.user_id, ie.event_id, ie.created_at, ie.device, ie.platform, ie.language
            ),
            activation_observations AS MATERIALIZED (
                SELECT
                    *,
                    CASE
                        WHEN total_sessions_text ~ '^[0-9]+$'
                            THEN total_sessions_text::integer
                    END AS total_sessions,
                    CASE
                        WHEN average_seconds_text ~ '^-?[0-9]+([.][0-9]+)?$'
                            THEN average_seconds_text::numeric
                    END AS average_seconds,
                    CASE
                        WHEN user_hour_text ~ '^[0-9]+$'
                            THEN user_hour_text::integer
                    END AS user_hour,
                    CASE
                        WHEN user_day_text ~ '^[0-9]+$'
                            THEN user_day_text::integer
                    END AS user_day,
                    CASE lower(user_weekend_text)
                        WHEN 'true' THEN true
                        WHEN 'false' THEN false
                    END AS user_weekend
                FROM activation_inputs
            ),
            activation_candidates AS MATERIALIZED (
                SELECT
                    *,
                    CASE
                        WHEN total_sessions = 1 THEN average_seconds
                        WHEN total_sessions BETWEEN 2 AND 6
                            THEN average_seconds * (total_sessions - 1)
                    END AS accumulated_seconds
                FROM activation_observations
                WHERE average_seconds >= 0
            ),
            activated_users AS MATERIALIZED (
                SELECT DISTINCT ON (user_id)
                    user_id,
                    created_at AS activation_at,
                    device,
                    platform,
                    language,
                    preferred_region,
                    user_hour,
                    user_day,
                    user_weekend
                FROM activation_candidates
                WHERE total_sessions BETWEEN 1 AND 6
                  AND accumulated_seconds >= 300
                ORDER BY user_id, created_at, event_id
            )
            "#,
        );
}

fn push_series(
    query: &mut QueryBuilder<'_, Postgres>,
    scope: &LifecycleScope,
    metric: &'static str,
    unit: &'static str,
    start: chrono::DateTime<chrono::Utc>,
) {
    let format = match unit {
        "hour" => r#"YYYY-MM-DD"T"HH24:00"#,
        "day" | "week" | "month" => "YYYY-MM-DD",
        _ => unreachable!("validated activation grain"),
    };
    query
        .push("SELECT 'series'::text AS section, ")
        .push_bind(metric)
        .push("::text AS metric, to_char(date_trunc('")
        .push(unit)
        .push("', activation_at AT TIME ZONE ")
        .push_bind(scope.timezone.clone())
        .push("), '")
        .push(format)
        .push(
            r#"')::text AS bucket, count(*)::bigint AS users
            FROM activated_users
            WHERE activation_at >= "#,
        )
        .push_bind(start)
        .push(" AND activation_at <= ")
        .push_bind(scope.end)
        .push(" GROUP BY 3");
}

fn push_breakdown(
    query: &mut QueryBuilder<'_, Postgres>,
    scope: &LifecycleScope,
    metric: &'static str,
    field: &'static str,
    include_unknown: bool,
) {
    query
        .push("SELECT 'breakdown'::text AS section, ")
        .push_bind(metric)
        .push("::text AS metric, ");
    if include_unknown {
        query
            .push("COALESCE(NULLIF(")
            .push(field)
            .push(", ''), 'Unknown')");
    } else {
        query.push(field);
    }
    query
        .push(
            r#"::text AS bucket, count(*)::bigint AS users
            FROM activated_users
            WHERE activation_at >= "#,
        )
        .push_bind(scope.start)
        .push(" AND activation_at <= ")
        .push_bind(scope.end);
    if !include_unknown {
        query
            .push(" AND NULLIF(")
            .push(field)
            .push(", '') IS NOT NULL");
    }
    query.push(" GROUP BY 3");
}

fn activation_query(scope: &LifecycleScope) -> QueryBuilder<'static, Postgres> {
    let mut query = QueryBuilder::new("");
    push_activation_ctes(&mut query, scope);
    let four_weeks_start = std::cmp::max(scope.start, scope.end - chrono::Duration::days(28));
    let weekly_start = std::cmp::max(
        scope.start,
        scope
            .end
            .checked_sub_months(Months::new(3))
            .unwrap_or(scope.start),
    );
    let monthly_start = std::cmp::max(
        scope.start,
        scope
            .end
            .checked_sub_months(Months::new(12))
            .unwrap_or(scope.start),
    );

    query
        .push(
            r#"SELECT 'summary'::text AS section, 'totalSignals'::text AS metric,
                ''::text AS bucket, count(*)::bigint AS users
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
            SELECT 'summary', 'identifiedSignals', '', count(*)
            FROM identified_events
            WHERE created_at >= "#,
        )
        .push_bind(scope.start)
        .push(" AND created_at <= ")
        .push_bind(scope.end)
        .push(
            r#" UNION ALL
            SELECT 'summary', 'inputSignals', '', count(*)
            FROM activation_observations
            WHERE created_at >= "#,
        )
        .push_bind(scope.start)
        .push(" AND created_at <= ")
        .push_bind(scope.end)
        .push(" AND total_sessions IS NOT NULL AND average_seconds IS NOT NULL")
        .push(
            r#" UNION ALL
            SELECT 'summary', 'activatedUsers', '', count(*)
            FROM activated_users
            WHERE activation_at >= "#,
        )
        .push_bind(scope.start)
        .push(" AND activation_at <= ")
        .push_bind(scope.end);

    for (metric, field) in [
        ("hasTotalSessions", "total_sessions_text"),
        ("hasAverageSeconds", "average_seconds_text"),
        ("hasHourOfDay", "user_hour_text"),
        ("hasDayOfWeek", "user_day_text"),
        ("hasIsWeekend", "user_weekend_text"),
        ("hasPreferredRegion", "preferred_region"),
    ] {
        query
            .push(" UNION ALL SELECT 'availability', ")
            .push_bind(metric)
            .push("::text, '', count(*) FILTER (WHERE NULLIF(")
            .push(field)
            .push(
                r#", '') IS NOT NULL)
                FROM activation_inputs
                WHERE created_at >= "#,
            )
            .push_bind(scope.start)
            .push(" AND created_at <= ")
            .push_bind(scope.end);
    }

    query.push(" UNION ALL ");
    push_series(
        &mut query,
        scope,
        "hourly",
        "hour",
        std::cmp::max(scope.start, scope.end - chrono::Duration::hours(24)),
    );
    query.push(" UNION ALL ");
    push_series(&mut query, scope, "daily", "day", scope.start);
    query.push(" UNION ALL ");
    push_series(&mut query, scope, "weekly", "week", weekly_start);
    query.push(" UNION ALL ");
    push_series(&mut query, scope, "monthly", "month", monthly_start);

    query
        .push(
            r#" UNION ALL
            SELECT 'pattern', 'hourOfDay', user_hour::text, count(*)
            FROM activated_users
            WHERE activation_at >= "#,
        )
        .push_bind(scope.start)
        .push(" AND activation_at <= ")
        .push_bind(scope.end)
        .push(" AND user_hour BETWEEN 1 AND 24 GROUP BY 3")
        .push(
            r#" UNION ALL
            SELECT 'pattern', 'hourOfDay4Weeks',
                extract(hour FROM activation_at AT TIME ZONE "#,
        )
        .push_bind(scope.timezone.clone())
        .push(
            r#")::integer::text, count(*)
            FROM activated_users
            WHERE activation_at >= "#,
        )
        .push_bind(four_weeks_start)
        .push(" AND activation_at <= ")
        .push_bind(scope.end)
        .push(" GROUP BY 3")
        .push(
            r#" UNION ALL
            SELECT 'pattern', 'dayOfWeek', user_day::text, count(*)
            FROM activated_users
            WHERE activation_at >= "#,
        )
        .push_bind(scope.start)
        .push(" AND activation_at <= ")
        .push_bind(scope.end)
        .push(" AND user_day BETWEEN 1 AND 7 GROUP BY 3")
        .push(
            r#" UNION ALL
            SELECT 'pattern', 'dayOfWeek4Weeks',
                extract(isodow FROM activation_at AT TIME ZONE "#,
        )
        .push_bind(scope.timezone.clone())
        .push(
            r#")::integer::text, count(*)
            FROM activated_users
            WHERE activation_at >= "#,
        )
        .push_bind(four_weeks_start)
        .push(" AND activation_at <= ")
        .push_bind(scope.end)
        .push(" GROUP BY 3")
        .push(
            r#" UNION ALL
            SELECT 'pattern', 'weekpart',
                CASE WHEN user_weekend THEN 'Weekend' ELSE 'Weekday' END, count(*)
            FROM activated_users
            WHERE activation_at >= "#,
        )
        .push_bind(scope.start)
        .push(" AND activation_at <= ")
        .push_bind(scope.end)
        .push(" AND user_weekend IS NOT NULL GROUP BY 3")
        .push(
            r#" UNION ALL
            SELECT 'pattern', 'weekpart4Weeks',
                CASE WHEN extract(isodow FROM activation_at AT TIME ZONE "#,
        )
        .push_bind(scope.timezone.clone())
        .push(
            r#") IN (6, 7) THEN 'Weekend' ELSE 'Weekday' END, count(*)
            FROM activated_users
            WHERE activation_at >= "#,
        )
        .push_bind(four_weeks_start)
        .push(" AND activation_at <= ")
        .push_bind(scope.end)
        .push(" GROUP BY 3");

    for (metric, field, include_unknown) in [
        ("device", "device", true),
        ("platform", "platform", true),
        ("region", "preferred_region", false),
        ("language", "language", false),
    ] {
        query.push(" UNION ALL ");
        push_breakdown(&mut query, scope, metric, field, include_unknown);
    }
    query.push(" ORDER BY section, metric, bucket");
    query
}

async fn activation(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match website_pool(&state, &headers, website_id, Access::Read).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let scope = match resolve_scope(&pool, website_id, &params).await {
        Ok(scope) => scope,
        Err(response) => return response,
    };
    let started = Instant::now();
    let rows = match activation_query(&scope).build().fetch_all(&pool).await {
        Ok(rows) => rows.into_iter().map(ReportRow::from_row).collect(),
        Err(error) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string());
        }
    };
    let query_ms = started.elapsed().as_secs_f64() * 1000.0;
    json_response(response_json(&scope, rows, query_ms))
}

fn response_json(scope: &LifecycleScope, rows: Vec<ReportRow>, query_ms: f64) -> Value {
    let mut summary = HashMap::new();
    let mut availability = HashMap::new();
    let mut series: HashMap<String, Vec<Value>> = HashMap::new();
    let mut patterns: HashMap<String, Vec<Value>> = HashMap::new();
    let mut breakdowns: HashMap<String, Vec<Value>> = HashMap::new();

    for row in rows {
        match row.section.as_str() {
            "summary" => {
                summary.insert(row.metric, row.users);
            }
            "availability" => {
                availability.insert(row.metric, row.users);
            }
            "series" => series.entry(row.metric).or_default().push(json!({
                "key": row.bucket,
                "label": row.bucket,
                "count": row.users,
            })),
            "pattern" => patterns.entry(row.metric).or_default().push(json!({
                "key": row.bucket,
                "label": row.bucket,
                "count": row.users,
            })),
            "breakdown" => breakdowns.entry(row.metric).or_default().push(json!({
                "key": row.bucket,
                "label": row.bucket,
                "count": row.users,
            })),
            _ => {}
        }
    }

    let total_signals = summary.remove("totalSignals").unwrap_or_default();
    let identified_signals = summary.remove("identifiedSignals").unwrap_or_default();
    let input_signals = summary.remove("inputSignals").unwrap_or_default();
    let activated_users = summary.remove("activatedUsers").unwrap_or_default();
    let keys = [
        ("hasTotalSessions", TOTAL_SESSIONS_KEY),
        ("hasAverageSeconds", AVERAGE_SECONDS_KEY),
        ("hasHourOfDay", HOUR_OF_DAY_KEY),
        ("hasDayOfWeek", DAY_OF_WEEK_KEY),
        ("hasIsWeekend", IS_WEEKEND_KEY),
        ("hasPreferredRegion", PREFERRED_REGION_KEY),
    ];
    let missing_keys = keys
        .iter()
        .filter_map(|(metric, key)| {
            (availability.remove(*metric).unwrap_or_default() == 0).then_some(*key)
        })
        .collect::<Vec<_>>();
    let core_ready =
        !missing_keys.contains(&TOTAL_SESSIONS_KEY) && !missing_keys.contains(&AVERAGE_SECONDS_KEY);
    let status = if total_signals == 0 {
        "empty"
    } else if identified_signals == 0 {
        "identity-missing"
    } else if !core_ready || input_signals == 0 {
        "configuration-missing"
    } else if input_signals < identified_signals {
        "partial"
    } else {
        "ready"
    };

    json!({
        "definition": {
            "rule": "At least 5 minutes accumulated total usage time within the first 5 sessions.",
            "thresholdSeconds": 300,
            "sessionLimit": 5,
            "source": "TelemetryDeck Activation Analytics documentation",
            "configurable": false,
            "calculation": "Maap uses the earliest stored SDK signal that proves the threshold was reached. For sessions 2–5, completed-session averages provide a lower bound; a fifth-session activation can become provable when session 6 starts."
        },
        "configuration": {
            "status": status,
            "coreReady": core_ready,
            "missingKeys": missing_keys,
            "requiredAllowlist": keys.map(|(_, key)| key),
            "environmentVariable": "TD_ALLOWED_KEYS",
            "historicalBackfill": false
        },
        "scope": {
            "startAt": scope.start,
            "endAt": scope.end,
            "timezone": scope.timezone,
            "testMode": scope.test_mode,
            "totalSignals": total_signals,
            "identifiedSignals": identified_signals,
            "activationInputSignals": input_signals,
            "activatedUsers": activated_users
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
        "query": {
            "milliseconds": query_ms,
            "identity": "clientUser",
            "unidentifiedSignalsExcluded": true
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
    fn query_applies_the_documented_threshold_and_deduplicates_qualifying_users() {
        let query = activation_query(&scope());
        let sql = query.sql();

        assert!(sql.contains("WHEN total_sessions = 1 THEN average_seconds"));
        assert!(sql.contains("average_seconds * (total_sessions - 1)"));
        assert!(sql.contains("DISTINCT ON (user_id)"));
        assert!(sql.contains("accumulated_seconds >= 300"));
        assert!(sql.contains("total_sessions BETWEEN 1 AND 6"));
        assert!(!sql.contains("session_id AS user_id"));
    }

    #[test]
    fn response_requires_core_fields_and_reports_missing_configuration() {
        let rows = vec![
            ReportRow {
                section: "summary".into(),
                metric: "totalSignals".into(),
                bucket: String::new(),
                users: 4,
            },
            ReportRow {
                section: "summary".into(),
                metric: "identifiedSignals".into(),
                bucket: String::new(),
                users: 4,
            },
        ];
        let body = response_json(&scope(), rows, 4.5);

        assert_eq!(body["configuration"]["status"], "configuration-missing");
        assert_eq!(body["configuration"]["coreReady"], false);
        assert_eq!(body["scope"]["activatedUsers"], 0);
        assert_eq!(body["definition"]["thresholdSeconds"], 300);
        assert_eq!(body["definition"]["sessionLimit"], 5);
    }

    #[test]
    fn response_keeps_repeated_qualifying_evidence_as_one_reported_user() {
        let rows = vec![
            ReportRow {
                section: "summary".into(),
                metric: "totalSignals".into(),
                bucket: String::new(),
                users: 8,
            },
            ReportRow {
                section: "summary".into(),
                metric: "identifiedSignals".into(),
                bucket: String::new(),
                users: 8,
            },
            ReportRow {
                section: "summary".into(),
                metric: "inputSignals".into(),
                bucket: String::new(),
                users: 8,
            },
            ReportRow {
                section: "summary".into(),
                metric: "activatedUsers".into(),
                bucket: String::new(),
                users: 1,
            },
            ReportRow {
                section: "availability".into(),
                metric: "hasTotalSessions".into(),
                bucket: String::new(),
                users: 8,
            },
            ReportRow {
                section: "availability".into(),
                metric: "hasAverageSeconds".into(),
                bucket: String::new(),
                users: 8,
            },
        ];
        let body = response_json(&scope(), rows, 2.0);

        assert_eq!(body["configuration"]["status"], "ready");
        assert_eq!(body["scope"]["activatedUsers"], 1);
    }
}
