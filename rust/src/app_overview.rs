use super::{
    access::{website_pool, Access},
    error_response, json_response,
    lifecycle::{clamp_to_activity_floor, website_activity_floor},
    metrics::analytics_timezone,
    parse_ms_param, AppState,
};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Map, Value};
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use uuid::Uuid;

const DIMENSIONS: [(&str, &str); 5] = [
    ("TelemetryDeck.SDK.name", "sdkName"),
    ("TelemetryDeck.SDK.version", "sdkVersion"),
    ("TelemetryDeck.AppInfo.version", "appVersion"),
    ("TelemetryDeck.AppInfo.buildNumber", "buildNumber"),
    ("TelemetryDeck.Device.systemVersion", "systemVersion"),
];

fn is_test_mode(params: &HashMap<String, String>) -> bool {
    params
        .get("testMode")
        .is_some_and(|value| value.eq_ignore_ascii_case("true"))
        || params
            .get("tag")
            .is_some_and(|value| value == "test" || value == "eq.test")
}

fn required_date_bounds(
    params: &HashMap<String, String>,
) -> Result<(DateTime<Utc>, DateTime<Utc>), Response> {
    let start = parse_ms_param(params, "startAt").or_else(|| parse_ms_param(params, "startDate"));
    let end = parse_ms_param(params, "endAt").or_else(|| parse_ms_param(params, "endDate"));
    match (start, end) {
        (Some(start), Some(end)) if start <= end => Ok((start, end)),
        (Some(start), Some(end)) => Ok((end, start)),
        _ => Err(error_response(
            StatusCode::BAD_REQUEST,
            "A valid start and end date are required",
        )),
    }
}

fn last_hour_cutoff(now: DateTime<Utc>) -> DateTime<Utc> {
    now - Duration::hours(1)
}

async fn summary(
    pool: &PgPool,
    website_id: Uuid,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    now: DateTime<Utc>,
    minimum: Option<DateTime<Utc>>,
    test_mode: bool,
) -> Result<Value, sqlx::Error> {
    let row = sqlx::query(
        r#"
        WITH scoped AS (
          SELECT e.event_id, e.session_id, e.created_at, s.distinct_id
          FROM website_event e
          JOIN session s
            ON s.website_id = e.website_id AND s.session_id = e.session_id
          WHERE e.website_id = $1
            AND e.created_at BETWEEN $2 AND $3
            AND (($6 AND e.tag = 'test') OR (NOT $6 AND COALESCE(e.tag, '') != 'test'))
        ),
        last_hour AS (
          SELECT count(DISTINCT s.distinct_id) FILTER (
            WHERE NULLIF(s.distinct_id, '') IS NOT NULL
          )::bigint AS users
          FROM website_event e
          JOIN session s
            ON s.website_id = e.website_id AND s.session_id = e.session_id
          WHERE e.website_id = $1
            AND e.created_at >= $4
            AND e.created_at <= $5
            AND (($6 AND e.tag = 'test') OR (NOT $6 AND COALESCE(e.tag, '') != 'test'))
        )
        SELECT
          count(*)::bigint AS events,
          count(DISTINCT distinct_id) FILTER (
            WHERE NULLIF(distinct_id, '') IS NOT NULL
          )::bigint AS total_users,
          (SELECT users FROM last_hour)::bigint AS users_last_hour
        FROM scoped
        "#,
    )
    .bind(website_id)
    .bind(start)
    .bind(end)
    .bind(clamp_to_activity_floor(last_hour_cutoff(now), minimum))
    .bind(now)
    .bind(test_mode)
    .fetch_one(pool)
    .await?;

    Ok(json!({
        "users": row.get::<i64, _>("total_users"),
        "usersLastHour": row.get::<i64, _>("users_last_hour"),
        "events": row.get::<i64, _>("events"),
        "errors": Value::Null,
    }))
}

async fn daily_users(
    pool: &PgPool,
    website_id: Uuid,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    timezone: &str,
    test_mode: bool,
) -> Result<Vec<Value>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        WITH bounds AS (
          SELECT
            ($2::timestamptz AT TIME ZONE $4)::date AS start_day,
            ($3::timestamptz AT TIME ZONE $4)::date AS end_day
        ),
        days AS (
          SELECT generate_series(start_day, end_day, interval '1 day')::date AS day
          FROM bounds
        ),
        counts AS (
          SELECT
            (e.created_at AT TIME ZONE $4)::date AS day,
            count(DISTINCT s.distinct_id) FILTER (
              WHERE NULLIF(s.distinct_id, '') IS NOT NULL
            )::bigint AS users
          FROM website_event e
          JOIN session s
            ON s.website_id = e.website_id AND s.session_id = e.session_id
          WHERE e.website_id = $1
            AND e.created_at BETWEEN $2 AND $3
            AND (($5 AND e.tag = 'test') OR (NOT $5 AND COALESCE(e.tag, '') != 'test'))
          GROUP BY 1
        )
        SELECT to_char(days.day, 'YYYY-MM-DD') AS x, COALESCE(counts.users, 0)::bigint AS y
        FROM days
        LEFT JOIN counts USING (day)
        ORDER BY days.day
        "#,
    )
    .bind(website_id)
    .bind(start)
    .bind(end)
    .bind(timezone)
    .bind(test_mode)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| json!({ "x": row.get::<String, _>("x"), "y": row.get::<i64, _>("y") }))
        .collect())
}

async fn dimensions(
    pool: &PgPool,
    website_id: Uuid,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    test_mode: bool,
) -> Result<Map<String, Value>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        WITH scoped AS (
          SELECT e.event_id, e.created_at, s.distinct_id
          FROM website_event e
          JOIN session s
            ON s.website_id = e.website_id AND s.session_id = e.session_id
          WHERE e.website_id = $1
            AND e.created_at BETWEEN $2 AND $3
            AND NULLIF(s.distinct_id, '') IS NOT NULL
            AND (($4 AND e.tag = 'test') OR (NOT $4 AND COALESCE(e.tag, '') != 'test'))
        ),
        identities AS (
          SELECT DISTINCT distinct_id FROM scoped
        ),
        keys(data_key, dimension) AS (
          VALUES
            ('TelemetryDeck.SDK.name', 'sdkName'),
            ('TelemetryDeck.SDK.version', 'sdkVersion'),
            ('TelemetryDeck.AppInfo.version', 'appVersion'),
            ('TelemetryDeck.AppInfo.buildNumber', 'buildNumber'),
            ('TelemetryDeck.Device.systemVersion', 'systemVersion')
        ),
        ranked AS (
          SELECT
            scoped.distinct_id,
            keys.dimension,
            d.string_value,
            row_number() OVER (
              PARTITION BY scoped.distinct_id, keys.dimension
              ORDER BY scoped.created_at DESC, scoped.event_id DESC
            ) AS row_number
          FROM scoped
          JOIN event_data d
            ON d.website_id = $1 AND d.website_event_id = scoped.event_id
          JOIN keys ON keys.data_key = d.data_key
        ),
        latest AS (
          SELECT distinct_id, dimension, string_value
          FROM ranked
          WHERE row_number = 1
        )
        SELECT
          keys.dimension,
          COALESCE(NULLIF(latest.string_value, ''), 'Unknown') AS value,
          count(*)::bigint AS users
        FROM identities
        CROSS JOIN keys
        LEFT JOIN latest
          ON latest.distinct_id = identities.distinct_id
         AND latest.dimension = keys.dimension
        GROUP BY keys.dimension, COALESCE(NULLIF(latest.string_value, ''), 'Unknown')
        ORDER BY keys.dimension, users DESC, value
        "#,
    )
    .bind(website_id)
    .bind(start)
    .bind(end)
    .bind(test_mode)
    .fetch_all(pool)
    .await?;

    let mut result = Map::new();
    for (_, dimension) in DIMENSIONS {
        result.insert(dimension.to_string(), Value::Array(Vec::new()));
    }
    for row in rows {
        let dimension: String = row.get("dimension");
        if let Some(Value::Array(values)) = result.get_mut(&dimension) {
            values.push(json!({
                "value": row.get::<String, _>("value"),
                "users": row.get::<i64, _>("users"),
            }));
        }
    }
    Ok(result)
}

async fn retention(
    pool: &PgPool,
    website_id: Uuid,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    minimum: Option<DateTime<Utc>>,
    timezone: &str,
    test_mode: bool,
) -> Result<Vec<Value>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        WITH first_seen AS (
          SELECT
            s.distinct_id,
            min((e.created_at AT TIME ZONE $4)::date) AS cohort_day
          FROM website_event e
          JOIN session s
            ON s.website_id = e.website_id AND s.session_id = e.session_id
          WHERE e.website_id = $1
            AND ($6::timestamptz IS NULL OR e.created_at >= $6)
            AND e.created_at <= $3
            AND NULLIF(s.distinct_id, '') IS NOT NULL
            AND (($5 AND e.tag = 'test') OR (NOT $5 AND COALESCE(e.tag, '') != 'test'))
          GROUP BY s.distinct_id
        ),
        cohort AS (
          SELECT distinct_id, cohort_day
          FROM first_seen
          WHERE cohort_day BETWEEN
            ($2::timestamptz AT TIME ZONE $4)::date
            AND ($3::timestamptz AT TIME ZONE $4)::date
        ),
        activity AS (
          SELECT DISTINCT
            s.distinct_id,
            (e.created_at AT TIME ZONE $4)::date AS activity_day
          FROM website_event e
          JOIN session s
            ON s.website_id = e.website_id AND s.session_id = e.session_id
          WHERE e.website_id = $1
            AND e.created_at BETWEEN $2 AND $3
            AND NULLIF(s.distinct_id, '') IS NOT NULL
            AND (($5 AND e.tag = 'test') OR (NOT $5 AND COALESCE(e.tag, '') != 'test'))
        ),
        intervals(day) AS (VALUES (1), (7), (14))
        SELECT
          intervals.day,
          count(cohort.distinct_id) FILTER (
            WHERE cohort.cohort_day + intervals.day
              <= ($3::timestamptz AT TIME ZONE $4)::date
          )::bigint AS eligible_users,
          count(cohort.distinct_id) FILTER (
            WHERE cohort.cohort_day + intervals.day
              <= ($3::timestamptz AT TIME ZONE $4)::date
              AND EXISTS (
                SELECT 1 FROM activity
                WHERE activity.distinct_id = cohort.distinct_id
                  AND activity.activity_day = cohort.cohort_day + intervals.day
              )
          )::bigint AS returning_users
        FROM intervals
        LEFT JOIN cohort ON true
        GROUP BY intervals.day
        ORDER BY intervals.day
        "#,
    )
    .bind(website_id)
    .bind(start)
    .bind(end)
    .bind(timezone)
    .bind(test_mode)
    .bind(minimum)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let eligible = row.get::<i64, _>("eligible_users");
            let returning = row.get::<i64, _>("returning_users");
            json!({
                "day": row.get::<i32, _>("day"),
                "eligibleUsers": eligible,
                "returningUsers": returning,
                "percentage": if eligible == 0 {
                    Value::Null
                } else {
                    json!((returning as f64 * 1000.0 / eligible as f64).round() / 10.0)
                },
            })
        })
        .collect())
}

pub async fn get_app_overview(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match website_pool(&state, &headers, website_id, Access::Read).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (requested_start, end) = match required_date_bounds(&params) {
        Ok(bounds) => bounds,
        Err(response) => return response,
    };
    let minimum = match website_activity_floor(&pool, website_id).await {
        Ok(minimum) => minimum,
        Err(response) => return response,
    };
    let start = clamp_to_activity_floor(requested_start, minimum);
    let timezone = match analytics_timezone(&pool, &params, false).await {
        Ok(timezone) => timezone,
        Err(response) => return response,
    };
    let test_mode = is_test_mode(&params);
    let now = Utc::now();

    let (mut summary, daily_users, dimensions, retention) = match tokio::try_join!(
        summary(&pool, website_id, start, end, now, minimum, test_mode),
        daily_users(&pool, website_id, start, end, &timezone, test_mode),
        dimensions(&pool, website_id, start, end, test_mode),
        retention(&pool, website_id, start, end, minimum, &timezone, test_mode),
    ) {
        Ok(data) => data,
        Err(error) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string());
        }
    };

    let most_used_sdk = dimensions
        .get("sdkName")
        .and_then(Value::as_array)
        .and_then(|values| values.first())
        .and_then(|item| item.get("value"))
        .cloned()
        .unwrap_or(Value::Null);
    summary["mostUsedSdk"] = most_used_sdk;

    json_response(json!({
        "identity": "clientUser",
        "range": {
            "startAt": start,
            "endAt": end,
            "timezone": timezone,
        },
        "summary": summary,
        "dailyUsers": daily_users,
        "retention": retention,
        "dimensions": dimensions,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn last_hour_is_sixty_minutes_and_deduplicates_sdk_identities() {
        let now = DateTime::parse_from_rfc3339("2026-11-20T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let cutoff = last_hour_cutoff(now);
        let events = [
            ("user-a", now - Duration::minutes(45)),
            ("user-a", now - Duration::minutes(10)),
            ("user-b", now - Duration::minutes(61)),
        ];
        let users = events
            .into_iter()
            .filter(|(_, created_at)| *created_at >= cutoff && *created_at <= now)
            .map(|(identity, _)| identity)
            .collect::<HashSet<_>>();

        assert_eq!(cutoff, now - Duration::minutes(60));
        assert_eq!(users, HashSet::from(["user-a"]));
    }
}
