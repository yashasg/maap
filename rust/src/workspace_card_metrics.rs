use super::{check_bearer_auth, error_response, json_response, AppState};
use axum::{
    extract::{rejection::JsonRejection, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::post,
    Json, Router,
};
use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

const MAX_WEBSITES: usize = 20;
const MAX_RANGE_MS: i64 = 366 * 24 * 60 * 60 * 1000;
const SHARE_TOKEN_HEADER: &str = "x-umami-share-token";
const SHARE_CONTEXT_HEADER: &str = "x-umami-share-context";

const PERSONAL_WEBSITES_SQL: &str = r#"
    SELECT website_id, domain
    FROM website
    WHERE website_id = ANY($1::uuid[])
      AND user_id = $2
      AND team_id IS NULL
      AND deleted_at IS NULL
"#;

const TEAM_WEBSITES_SQL: &str = r#"
    SELECT website_id, domain
    FROM website
    WHERE website_id = ANY($1::uuid[])
      AND team_id = $2
      AND deleted_at IS NULL
"#;

const METRICS_SQL: &str = r#"
    WITH identity_candidates AS (
        SELECT website_id, session_id, NULLIF(distinct_id, '') AS user_id
        FROM session
        WHERE website_id = ANY($1::uuid[])
          AND NULLIF(distinct_id, '') IS NOT NULL
        UNION ALL
        SELECT website_id, session_id, NULLIF(distinct_id, '') AS user_id
        FROM session_link
        WHERE website_id = ANY($1::uuid[])
          AND NULLIF(distinct_id, '') IS NOT NULL
    ),
    identity_sessions AS (
        SELECT website_id, session_id, min(user_id) AS user_id
        FROM identity_candidates
        GROUP BY website_id, session_id
        HAVING count(DISTINCT user_id) = 1
    ),
    event_metrics AS (
        SELECT
            e.website_id,
            count(DISTINCT CASE
                WHEN e.website_id = ANY($2::uuid[]) THEN identities.user_id
                WHEN e.event_type NOT IN (2, 5) THEN e.session_id::text
            END)::bigint AS users,
            count(*) FILTER (
                WHERE e.website_id = ANY($2::uuid[])
                  AND e.event_name = 'TelemetryDeck.Error.occurred'
            )::bigint AS app_errors
        FROM website_event e
        JOIN website w ON w.website_id = e.website_id
        LEFT JOIN identity_sessions identities
          ON identities.website_id = e.website_id
         AND identities.session_id = e.session_id
        WHERE e.website_id = ANY($1::uuid[])
          AND e.created_at BETWEEN
              greatest($3, COALESCE(w.reset_at, 'epoch'::timestamptz))
              AND $4
          AND (($5 AND e.tag = 'test') OR (NOT $5 AND COALESCE(e.tag, '') <> 'test'))
        GROUP BY e.website_id
    ),
    website_errors AS (
        SELECT ee.website_id, count(*)::bigint AS errors
        FROM error_event ee
        JOIN website w ON w.website_id = ee.website_id
        WHERE ee.website_id = ANY($1::uuid[])
          AND NOT (ee.website_id = ANY($2::uuid[]))
          AND ee.created_at BETWEEN $3 AND $4
          AND ee.created_at > COALESCE(w.reset_at, 'epoch'::timestamptz)
        GROUP BY ee.website_id
    )
    SELECT
        w.website_id,
        COALESCE(metrics.users, 0)::bigint AS users,
        CASE
            WHEN w.website_id = ANY($2::uuid[]) THEN COALESCE(metrics.app_errors, 0)
            ELSE COALESCE(website_errors.errors, 0)
        END::bigint AS errors
    FROM website w
    LEFT JOIN event_metrics metrics USING (website_id)
    LEFT JOIN website_errors USING (website_id)
    WHERE w.website_id = ANY($1::uuid[])
"#;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CardMetricsRequest {
    website_ids: Vec<Uuid>,
    start_at: i64,
    end_at: i64,
    test_mode: bool,
    team_id: Option<Uuid>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CardMetric {
    users: i64,
    errors: i64,
}

fn validate_request(
    request: &CardMetricsRequest,
) -> Result<(DateTime<Utc>, DateTime<Utc>), &'static str> {
    if request.website_ids.is_empty() || request.website_ids.len() > MAX_WEBSITES {
        return Err("websiteIds must contain between 1 and 20 IDs");
    }
    if request.website_ids.iter().collect::<HashSet<_>>().len() != request.website_ids.len() {
        return Err("websiteIds must be unique");
    }
    if request.start_at < 0 || request.end_at < 0 {
        return Err("Dates must be non-negative Unix milliseconds");
    }
    let Some(start) = Utc.timestamp_millis_opt(request.start_at).single() else {
        return Err("Invalid startAt");
    };
    let Some(end) = Utc.timestamp_millis_opt(request.end_at).single() else {
        return Err("Invalid endAt");
    };
    if start > end {
        return Err("The end date must not be before the start date");
    }
    if request
        .end_at
        .checked_sub(request.start_at)
        .is_none_or(|duration| duration > MAX_RANGE_MS)
    {
        return Err("The date range must not exceed 366 days");
    }
    Ok((start, end))
}

fn is_native_app_domain(domain: Option<&str>) -> bool {
    let Some(domain) = domain else {
        return false;
    };
    let value = domain.to_ascii_lowercase();
    matches!(value.as_str(), "app.ios" | "app.macos" | "app.android")
        || value.contains("apps.apple.com/")
        || value.contains("itunes.apple.com/")
        || value.contains("play.google.com/")
}

fn response_data(website_ids: &[Uuid], metrics: &HashMap<Uuid, CardMetric>) -> Value {
    json!({
        "data": website_ids.iter().map(|website_id| {
            let metric = metrics.get(website_id).copied().unwrap_or(CardMetric {
                users: 0,
                errors: 0,
            });
            json!({
                "websiteId": website_id,
                "users": metric.users,
                "errors": metric.errors,
            })
        }).collect::<Vec<_>>()
    })
}

async fn card_metrics(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: Result<Json<Value>, JsonRejection>,
) -> Response {
    if headers.contains_key(SHARE_TOKEN_HEADER) || headers.contains_key(SHARE_CONTEXT_HEADER) {
        return error_response(StatusCode::UNAUTHORIZED, "Unauthorized");
    }
    let Some(pool) = state.db.as_ref() else {
        return error_response(StatusCode::SERVICE_UNAVAILABLE, "Database unavailable");
    };
    let Some((user_id, role)) =
        check_bearer_auth(pool, &headers, "/api/websites/card-metrics").await
    else {
        return error_response(StatusCode::UNAUTHORIZED, "Unauthorized");
    };
    let Json(value) = match payload {
        Ok(payload) => payload,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid request body"),
    };
    let request: CardMetricsRequest = match serde_json::from_value(value) {
        Ok(request) => request,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid request body"),
    };
    let (start, end) = match validate_request(&request) {
        Ok(range) => range,
        Err(message) => return error_response(StatusCode::BAD_REQUEST, message),
    };

    if let Some(team_id) = request.team_id {
        let authorized = sqlx::query_scalar::<_, bool>(
            r#"
            SELECT EXISTS (
                SELECT 1
                FROM team t
                WHERE t.team_id = $1
                  AND t.deleted_at IS NULL
                  AND (
                    $3
                    OR EXISTS (
                        SELECT 1 FROM team_user tu
                        WHERE tu.team_id = t.team_id AND tu.user_id = $2
                    )
                  )
            )
            "#,
        )
        .bind(team_id)
        .bind(user_id)
        .bind(role == "admin")
        .fetch_one(pool)
        .await;
        match authorized {
            Ok(true) => (),
            Ok(false) => return error_response(StatusCode::UNAUTHORIZED, "Unauthorized"),
            Err(error) => {
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
            }
        }
    }

    let website_rows = match request.team_id {
        Some(team_id) => {
            sqlx::query(TEAM_WEBSITES_SQL)
                .bind(&request.website_ids)
                .bind(team_id)
                .fetch_all(pool)
                .await
        }
        None => {
            sqlx::query(PERSONAL_WEBSITES_SQL)
                .bind(&request.website_ids)
                .bind(user_id)
                .fetch_all(pool)
                .await
        }
    };
    let website_rows = match website_rows {
        Ok(rows) => rows,
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    };
    if website_rows.len() != request.website_ids.len() {
        return error_response(StatusCode::UNAUTHORIZED, "Unauthorized");
    }

    let native_website_ids = website_rows
        .iter()
        .filter(|row| is_native_app_domain(row.get::<Option<String>, _>("domain").as_deref()))
        .map(|row| row.get::<Uuid, _>("website_id"))
        .collect::<Vec<_>>();
    let rows = match sqlx::query(METRICS_SQL)
        .bind(&request.website_ids)
        .bind(&native_website_ids)
        .bind(start)
        .bind(end)
        .bind(request.test_mode)
        .fetch_all(pool)
        .await
    {
        Ok(rows) => rows,
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    };
    let metrics = rows
        .into_iter()
        .map(|row| {
            (
                row.get::<Uuid, _>("website_id"),
                CardMetric {
                    users: row.get("users"),
                    errors: row.get("errors"),
                },
            )
        })
        .collect::<HashMap<_, _>>();

    json_response(response_data(&request.website_ids, &metrics))
}

pub(super) fn register_routes(router: Router<AppState>) -> Router<AppState> {
    router.route("/api/websites/card-metrics", post(card_metrics))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> CardMetricsRequest {
        CardMetricsRequest {
            website_ids: vec![
                Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap(),
                Uuid::parse_str("22222222-2222-4222-8222-222222222222").unwrap(),
            ],
            start_at: 1_762_000_000_000,
            end_at: 1_763_000_000_000,
            test_mode: false,
            team_id: None,
        }
    }

    #[test]
    fn validates_batch_size_unique_ids_and_ordered_bounded_dates() {
        assert!(validate_request(&request()).is_ok());

        let mut duplicate = request();
        duplicate.website_ids[1] = duplicate.website_ids[0];
        assert_eq!(
            validate_request(&duplicate).unwrap_err(),
            "websiteIds must be unique"
        );

        let mut reversed = request();
        reversed.start_at = reversed.end_at + 1;
        assert_eq!(
            validate_request(&reversed).unwrap_err(),
            "The end date must not be before the start date"
        );

        let mut negative = request();
        negative.start_at = -1;
        assert_eq!(
            validate_request(&negative).unwrap_err(),
            "Dates must be non-negative Unix milliseconds"
        );

        let mut too_wide = request();
        too_wide.end_at = too_wide.start_at + MAX_RANGE_MS + 1;
        assert_eq!(
            validate_request(&too_wide).unwrap_err(),
            "The date range must not exceed 366 days"
        );
    }

    #[test]
    fn recognizes_only_native_app_domains() {
        assert!(is_native_app_domain(Some("app.ios")));
        assert!(is_native_app_domain(Some(
            "https://apps.apple.com/app/id123"
        )));
        assert!(is_native_app_domain(Some(
            "https://play.google.com/store/apps/details?id=app"
        )));
        assert!(!is_native_app_domain(Some("example.com")));
        assert!(!is_native_app_domain(None));
    }

    #[test]
    fn query_uses_verified_app_identity_and_error_semantics() {
        assert!(METRICS_SQL.contains("session_link"));
        assert!(METRICS_SQL.contains("HAVING count(DISTINCT user_id) = 1"));
        assert!(METRICS_SQL.contains("TelemetryDeck.Error.occurred"));
        assert!(!METRICS_SQL.contains("TelemetryDeck.Error.id"));
        assert!(METRICS_SQL.contains("FROM error_event ee"));
        assert!(METRICS_SQL.contains("e.tag = 'test'"));
        assert!(METRICS_SQL.contains("greatest($3, COALESCE(w.reset_at"));
        assert!(METRICS_SQL.contains("ee.created_at > COALESCE(w.reset_at"));
        assert!(PERSONAL_WEBSITES_SQL.contains("AND team_id IS NULL"));
        assert!(TEAM_WEBSITES_SQL.contains("AND team_id = $2"));
    }

    #[test]
    fn response_preserves_request_order_and_fills_missing_metrics() {
        let first = request().website_ids[0];
        let second = request().website_ids[1];
        let body = response_data(
            &[first, second],
            &HashMap::from([(
                second,
                CardMetric {
                    users: 7,
                    errors: 2,
                },
            )]),
        );

        assert_eq!(body["data"][0]["websiteId"], json!(first));
        assert_eq!(body["data"][0]["users"], 0);
        assert_eq!(body["data"][1]["websiteId"], json!(second));
        assert_eq!(body["data"][1]["users"], 7);
        assert_eq!(body["data"][1]["errors"], 2);
    }
}
