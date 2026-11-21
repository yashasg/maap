use crate::{
    access::{website_section_pool, ShareDataVisibility, ShareSection},
    error_response, json_response,
    lifecycle::{clamp_to_activity_floor, website_activity_floor},
    AppState,
};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::get,
    Router,
};
use chrono::{DateTime, Utc};
use serde_json::json;
use sqlx::Row;
use std::collections::HashMap;
use uuid::Uuid;

const EVENT_TYPES_SQL: &str = r#"
    SELECT e.event_name AS name,
           count(DISTINCT NULLIF(s.distinct_id, ''))::bigint AS users,
           count(*)::bigint AS events
    FROM website_event e
    LEFT JOIN session s
      ON s.website_id = e.website_id
     AND s.session_id = e.session_id
    WHERE e.website_id = $1
      AND e.created_at BETWEEN $2 AND $3
      AND e.event_type = 2
      AND e.event_name IS NOT NULL
      AND (($4 AND e.tag = 'test') OR (NOT $4 AND COALESCE(e.tag, '') != 'test'))
    GROUP BY e.event_name
    ORDER BY events DESC, lower(e.event_name), e.event_name
"#;

pub(crate) fn register_event_type_routes(router: Router<AppState>) -> Router<AppState> {
    router.route(
        "/api/websites/{website_id}/app-reports/event-types",
        get(event_types),
    )
}

fn date_param(params: &HashMap<String, String>, key: &str) -> Option<DateTime<Utc>> {
    params
        .get(key)?
        .parse::<i64>()
        .ok()
        .and_then(DateTime::from_timestamp_millis)
}

fn query_scope(
    params: &HashMap<String, String>,
) -> Result<(DateTime<Utc>, DateTime<Utc>, bool), Response> {
    let start = date_param(params, "startAt");
    let end = date_param(params, "endAt");
    let test_mode = match params.get("testMode").map(String::as_str) {
        None | Some("false") => false,
        Some("true") => true,
        Some(_) => {
            return Err(error_response(
                StatusCode::BAD_REQUEST,
                "Invalid Test Mode value",
            ));
        }
    };

    match (start, end) {
        (Some(start), Some(end)) if start <= end => Ok((start, end, test_mode)),
        (Some(_), Some(_)) => Err(error_response(
            StatusCode::BAD_REQUEST,
            "Start date must not be after end date",
        )),
        _ => Err(error_response(
            StatusCode::BAD_REQUEST,
            "A valid start and end date are required",
        )),
    }
}

async fn event_types(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match website_section_pool(
        &state,
        &headers,
        website_id,
        &[ShareSection::Events],
        ShareDataVisibility::Aggregate,
    )
    .await
    {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (requested_start, end, test_mode) = match query_scope(&params) {
        Ok(scope) => scope,
        Err(response) => return response,
    };
    let minimum = match website_activity_floor(&pool, website_id).await {
        Ok(minimum) => minimum,
        Err(response) => return response,
    };
    let start = clamp_to_activity_floor(requested_start, minimum);

    match sqlx::query(EVENT_TYPES_SQL)
        .bind(website_id)
        .bind(start)
        .bind(end)
        .bind(test_mode)
        .fetch_all(&pool)
        .await
    {
        Ok(rows) => json_response(json!({
            "data": rows.into_iter().map(|row| json!({
                "name": row.get::<String, _>("name"),
                "users": row.get::<i64, _>("users"),
                "events": row.get::<i64, _>("events"),
            })).collect::<Vec<_>>()
        })),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_sdk_identity_instead_of_visitor_sessions() {
        assert!(EVENT_TYPES_SQL.contains("count(DISTINCT NULLIF(s.distinct_id, ''))"));
        assert!(!EVENT_TYPES_SQL.contains("count(DISTINCT e.session_id)"));
    }

    #[test]
    fn separates_release_and_test_mode_events() {
        assert!(EVENT_TYPES_SQL.contains("e.tag = 'test'"));
        assert!(EVENT_TYPES_SQL.contains("COALESCE(e.tag, '') != 'test'"));
    }

    #[test]
    fn validates_range_and_test_mode() {
        let params = HashMap::from([
            ("startAt".to_string(), "1000".to_string()),
            ("endAt".to_string(), "2000".to_string()),
            ("testMode".to_string(), "true".to_string()),
        ]);
        let (start, end, test_mode) = query_scope(&params).expect("valid scope");
        assert!(start < end);
        assert!(test_mode);
    }

    #[test]
    fn reset_floor_can_make_the_event_range_empty() {
        let start = DateTime::from_timestamp_millis(1_000).unwrap();
        let end = DateTime::from_timestamp_millis(2_000).unwrap();
        let reset = DateTime::from_timestamp_millis(3_000).unwrap();

        assert!(clamp_to_activity_floor(start, Some(reset)) > end);
    }
}
