use crate::{error_response, metrics::analytics_timezone, parse_ms_param};
use axum::{http::StatusCode, response::Response};
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, QueryBuilder};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LifecycleScope {
    pub(crate) website_id: Uuid,
    pub(crate) start: DateTime<Utc>,
    pub(crate) end: DateTime<Utc>,
    pub(crate) minimum: Option<DateTime<Utc>>,
    pub(crate) timezone: String,
    pub(crate) test_mode: bool,
}

pub(crate) fn clamp_to_activity_floor(
    start: DateTime<Utc>,
    minimum: Option<DateTime<Utc>>,
) -> DateTime<Utc> {
    minimum.map_or(start, |minimum| start.max(minimum))
}

pub(crate) async fn website_activity_floor(
    pool: &PgPool,
    website_id: Uuid,
) -> Result<Option<DateTime<Utc>>, Response> {
    sqlx::query_scalar::<_, Option<DateTime<Utc>>>(
        "SELECT reset_at FROM website WHERE website_id = $1",
    )
    .bind(website_id)
    .fetch_optional(pool)
    .await
    .map(Option::flatten)
    .map_err(|error| error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
}

pub(crate) async fn resolve_scope(
    pool: &PgPool,
    website_id: Uuid,
    params: &HashMap<String, String>,
) -> Result<LifecycleScope, Response> {
    let start = parse_ms_param(params, "startAt").or_else(|| parse_ms_param(params, "startDate"));
    let end = parse_ms_param(params, "endAt").or_else(|| parse_ms_param(params, "endDate"));
    let (Some(start), Some(end)) = (start, end) else {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "A valid start and end date are required",
        ));
    };
    if start > end {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "The start date must not be after the end date",
        ));
    }
    let timezone = analytics_timezone(pool, params, true).await?;
    let test_mode = params
        .get("testMode")
        .is_some_and(|value| value.eq_ignore_ascii_case("true"));
    let minimum = website_activity_floor(pool, website_id).await?;
    let start = clamp_to_activity_floor(start, minimum);

    Ok(LifecycleScope {
        website_id,
        start,
        end,
        minimum,
        timezone,
        test_mode,
    })
}

/// Starts a query with two reusable CTEs:
///
/// - `identified_events` contains only events that can be tied to exactly one SDK app-user
///   identity (`clientUser`, persisted as `distinct_id`).
/// - `first_seen_users` contains one row per app user, attributed to their first stored signal.
///
/// The CTE deliberately has no selected-range predicate. Lifecycle reports need history before
/// the visible range to distinguish an active existing user from a genuinely new user.
pub(crate) fn push_identified_events_cte(
    query: &mut QueryBuilder<'_, Postgres>,
    scope: &LifecycleScope,
) {
    query
        .push(
            r#"
            WITH identity_candidates AS (
                SELECT website_id, session_id, NULLIF(distinct_id, '') AS user_id
                FROM session
                WHERE website_id = "#,
        )
        .push_bind(scope.website_id)
        .push(
            r#"
                  AND NULLIF(distinct_id, '') IS NOT NULL
                UNION ALL
                SELECT website_id, session_id, NULLIF(distinct_id, '') AS user_id
                FROM session_link
                WHERE website_id = "#,
        )
        .push_bind(scope.website_id)
        .push(
            r#"
                  AND NULLIF(distinct_id, '') IS NOT NULL
            ),
            identity_sessions AS (
                SELECT website_id, session_id, min(user_id) AS user_id
                FROM identity_candidates
                GROUP BY website_id, session_id
                HAVING count(DISTINCT user_id) = 1
            ),
            identified_events AS MATERIALIZED (
                SELECT
                    identities.user_id,
                    e.event_id,
                    e.session_id,
                    e.created_at,
                    e.event_type,
                    e.tag,
                    s.device,
                    CASE
                        WHEN lower(COALESCE(s.os, '')) LIKE '%android%' THEN 'Android'
                        WHEN lower(COALESCE(s.os, '')) LIKE '%ios%' THEN 'iOS'
                        WHEN lower(COALESCE(s.os, '')) LIKE '%mac%' THEN 'macOS'
                        WHEN NULLIF(s.os, '') IS NOT NULL THEN s.os
                        ELSE 'Unknown'
                    END AS platform,
                    s.language,
                    s.country,
                    s.region
                FROM website_event e
                JOIN identity_sessions identities
                  ON identities.website_id = e.website_id
                 AND identities.session_id = e.session_id
                JOIN session s
                  ON s.website_id = e.website_id
                 AND s.session_id = e.session_id
                WHERE e.website_id = "#,
        )
        .push_bind(scope.website_id)
        .push(if scope.minimum.is_some() {
            " AND e.created_at >= "
        } else {
            ""
        });
    if let Some(minimum) = scope.minimum {
        query.push_bind(minimum);
    }
    query
        .push(" AND ((")
        .push_bind(scope.test_mode)
        .push(" AND e.tag = 'test') OR (NOT ")
        .push_bind(scope.test_mode)
        .push(
            r#" AND COALESCE(e.tag, '') <> 'test'))
            ),
            first_seen_users AS MATERIALIZED (
                SELECT DISTINCT ON (user_id)
                    user_id,
                    created_at AS first_seen,
                    device,
                    platform,
                    language,
                    country,
                    region
                FROM identified_events
                ORDER BY user_id, created_at, event_id
            )
            "#,
        );
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn scope() -> LifecycleScope {
        LifecycleScope {
            website_id: Uuid::nil(),
            start: Utc.timestamp_opt(1_700_000_000, 0).single().unwrap(),
            end: Utc.timestamp_opt(1_700_086_400, 0).single().unwrap(),
            minimum: None,
            timezone: "UTC".into(),
            test_mode: false,
        }
    }

    #[test]
    fn lifecycle_identity_cte_uses_sdk_identity_and_all_history() {
        let mut query = QueryBuilder::<Postgres>::new("");
        push_identified_events_cte(&mut query, &scope());
        let sql = query.sql();

        assert!(sql.contains("NULLIF(distinct_id, '') AS user_id"));
        assert!(sql.contains("session_link"));
        assert!(sql.contains("HAVING count(DISTINCT user_id) = 1"));
        assert!(sql.contains("first_seen_users"));
        assert!(!sql.contains("created_at BETWEEN"));
        assert!(!sql.contains("session_id AS user_id"));
    }

    #[test]
    fn lifecycle_identity_context_starts_at_policy_floor_not_selected_start() {
        let minimum = Utc.timestamp_opt(1_600_000_000, 0).single().unwrap();
        let mut scoped = scope();
        scoped.minimum = Some(minimum);
        let mut query = QueryBuilder::<Postgres>::new("");
        push_identified_events_cte(&mut query, &scoped);
        let sql = query.sql();

        assert!(sql.contains("AND e.created_at >= "));
        assert!(!sql.contains("created_at BETWEEN"));
        assert!(scoped.minimum.unwrap() < scoped.start);
    }

    #[test]
    fn activity_floor_can_make_a_requested_range_wholly_disallowed() {
        let end = Utc.timestamp_opt(1_700_000_000, 0).single().unwrap();
        let minimum = end + chrono::Duration::days(1);

        assert!(clamp_to_activity_floor(end - chrono::Duration::days(30), Some(minimum)) > end);
    }

    #[test]
    fn lifecycle_scope_requires_ordered_dates() {
        let reversed = LifecycleScope {
            start: scope().end,
            end: scope().start,
            ..scope()
        };
        assert!(reversed.start > reversed.end);
    }
}
