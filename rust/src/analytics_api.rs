use super::{
    check_bearer_auth, error_response, json_response, page_params, parse_ms_param,
    with_rust_header, AppState,
};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub fn register_analytics_routes(mut router: Router<AppState>) -> Router<AppState> {
    router = router
        .route("/api/websites/{website_id}/pageviews", get(pageviews))
        .route("/api/websites/{website_id}/active", get(active))
        .route("/api/realtime/{website_id}", get(realtime))
        .route("/api/websites/{website_id}/daterange", get(date_range))
        .route("/api/websites/{website_id}/values", get(values))
        .route(
            "/api/websites/{website_id}/metrics/expanded",
            get(expanded_metrics),
        )
        .route("/api/websites/{website_id}/events", get(events))
        .route(
            "/api/websites/{website_id}/events/series",
            get(event_series),
        )
        .route("/api/websites/{website_id}/events/stats", get(event_stats))
        .route("/api/websites/{website_id}/event-data", get(event_data))
        .route(
            "/api/websites/{website_id}/event-data/stats",
            get(event_data_stats),
        )
        .route(
            "/api/websites/{website_id}/event-data/events",
            get(event_data_events),
        )
        .route(
            "/api/websites/{website_id}/event-data/fields",
            get(event_data_fields),
        )
        .route(
            "/api/websites/{website_id}/event-data/properties",
            get(event_data_properties),
        )
        .route(
            "/api/websites/{website_id}/event-data/values",
            get(event_data_values),
        )
        .route(
            "/api/websites/{website_id}/event-data/{event_id}",
            get(event_data_by_id),
        )
        .route("/api/websites/{website_id}/sessions", get(sessions))
        .route(
            "/api/websites/{website_id}/sessions/stats",
            get(session_stats),
        )
        .route(
            "/api/websites/{website_id}/sessions/weekly",
            get(session_weekly),
        )
        .route(
            "/api/websites/{website_id}/sessions/{session_id}",
            get(session_detail).delete(delete_session),
        )
        .route(
            "/api/websites/{website_id}/sessions/{session_id}/activity",
            get(session_activity),
        )
        .route(
            "/api/websites/{website_id}/sessions/{session_id}/properties",
            get(session_properties),
        )
        .route(
            "/api/websites/{website_id}/session-data/properties",
            get(session_data_properties),
        )
        .route(
            "/api/websites/{website_id}/session-data/values",
            get(session_data_values),
        );

    for path in [
        "/api/websites/{website_id}/utm/metrics",
        "/api/websites/{website_id}/breakdown",
        "/api/websites/{website_id}/funnels/stats",
        "/api/websites/{website_id}/funnels/{report_id}/stats",
        "/api/websites/{website_id}/goals/stats",
        "/api/websites/{website_id}/goals/{report_id}/stats",
        "/api/websites/{website_id}/retention",
        "/api/websites/{website_id}/revenue/metrics",
        "/api/websites/{website_id}/revenue/chart",
        "/api/websites/{website_id}/journeys",
        "/api/websites/{website_id}/attribution",
        "/api/websites/{website_id}/performance/metrics",
        "/api/websites/{website_id}/performance/chart",
        "/api/websites/{website_id}/errors/values",
        "/api/websites/{website_id}/errors/{issue_id}/values",
        "/api/websites/{website_id}/event-data-pivot/array-series",
        "/api/websites/{website_id}/event-data-pivot/date-series",
        "/api/websites/{website_id}/event-data-pivot/numeric-series",
        "/api/websites/{website_id}/event-data-pivot/property-series",
        "/api/websites/{website_id}/session-data/array-series",
        "/api/websites/{website_id}/session-data/date-series",
        "/api/websites/{website_id}/session-data/numeric-series",
        "/api/websites/{website_id}/session-data/property-series",
        "/api/websites/{website_id}/session-data/stats",
    ] {
        router = router.route(path, get(empty_array));
    }

    for path in [
        "/api/websites/{website_id}/funnels",
        "/api/websites/{website_id}/goals",
        "/api/websites/{website_id}/sessions/{session_id}/replays",
        "/api/websites/{website_id}/revenue/sessions",
        "/api/websites/{website_id}/replays",
        "/api/websites/{website_id}/replays/saved",
        "/api/websites/{website_id}/errors",
        "/api/websites/{website_id}/errors/{issue_id}/events",
        "/api/websites/{website_id}/event-data-pivot",
        "/api/websites/{website_id}/session-data-pivot",
    ] {
        router = router.route(path, get(empty_page));
    }

    for path in [
        "/api/websites/{website_id}/funnels/{report_id}",
        "/api/websites/{website_id}/goals/{report_id}",
        "/api/websites/{website_id}/errors/{issue_id}/events/{event_id}",
        "/api/websites/{website_id}/event-data-pivot/numeric-stats",
        "/api/websites/{website_id}/session-data/numeric-stats",
    ] {
        router = router.route(path, get(empty_object));
    }

    router
        .route("/api/websites/{website_id}/revenue/stats", get(revenue_stats))
        .route(
            "/api/websites/{website_id}/performance/stats",
            get(performance_stats),
        )
        .route(
            "/api/websites/{website_id}/replays/saved/{replay_id}",
            get(empty_object),
        )
        .route(
            "/api/websites/{website_id}/replays/{replay_id}",
            get(empty_object),
        )
        .route(
            "/api/websites/{website_id}/errors/stats",
            get(empty_error_stats),
        )
        .route(
            "/api/websites/{website_id}/errors/{issue_id}",
            get(empty_error_stats),
        )
}

async fn authorized_pool(state: &AppState, headers: &HeaderMap) -> Result<PgPool, Response> {
    let Some(pool) = state.db.clone() else {
        return Err(error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "Database unavailable",
        ));
    };
    if check_bearer_auth(&pool, headers, "/api/websites")
        .await
        .is_none()
    {
        return Err(with_rust_header(StatusCode::UNAUTHORIZED.into_response()));
    }
    Ok(pool)
}

fn date_bounds(params: &HashMap<String, String>) -> (DateTime<Utc>, DateTime<Utc>) {
    let end = parse_ms_param(params, "endAt")
        .or_else(|| parse_ms_param(params, "endDate"))
        .unwrap_or_else(Utc::now);
    let start = parse_ms_param(params, "startAt")
        .or_else(|| parse_ms_param(params, "startDate"))
        .unwrap_or_else(|| end - Duration::days(1));
    if start <= end {
        (start, end)
    } else {
        (end, start)
    }
}

fn is_test_mode(params: &HashMap<String, String>) -> bool {
    params
        .get("testMode")
        .is_some_and(|value| value.eq_ignore_ascii_case("true"))
        || params
            .get("tag")
            .is_some_and(|value| value == "test" || value == "eq.test")
}

fn unit(
    params: &HashMap<String, String>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> &'static str {
    match params.get("unit").map(String::as_str) {
        Some("minute") => "minute",
        Some("hour") => "hour",
        Some("day") => "day",
        Some("month") => "month",
        Some("year") => "year",
        _ if end - start <= Duration::days(2) => "hour",
        _ => "day",
    }
}

fn decimal_value(value: Option<Decimal>) -> Value {
    value
        .and_then(|number| number.to_string().parse::<f64>().ok())
        .map(Value::from)
        .unwrap_or(Value::Null)
}

async fn load_series(
    pool: &PgPool,
    website_id: Uuid,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    bucket: &str,
    test_mode: bool,
) -> Result<(Vec<Value>, Vec<Value>), sqlx::Error> {
    let sql = format!(
        r#"
        SELECT
          to_char(date_trunc('{bucket}', created_at AT TIME ZONE 'UTC'), 'YYYY-MM-DD HH24:MI:SS') AS x,
          count(*) FILTER (WHERE event_type NOT IN (2, 5))::bigint AS pageviews,
          count(DISTINCT session_id) FILTER (WHERE event_type NOT IN (2, 5))::bigint AS sessions
        FROM website_event
        WHERE website_id = $1
          AND created_at BETWEEN $2 AND $3
          AND (($4 AND tag = 'test') OR (NOT $4 AND COALESCE(tag, '') != 'test'))
        GROUP BY 1
        ORDER BY 1
        "#
    );
    let rows = sqlx::query(&sql)
        .bind(website_id)
        .bind(start)
        .bind(end)
        .bind(test_mode)
        .fetch_all(pool)
        .await?;
    let mut pageviews = Vec::with_capacity(rows.len());
    let mut sessions = Vec::with_capacity(rows.len());
    for row in rows.into_iter().rev() {
        let x: String = row.get("x");
        pageviews.push(json!({ "x": x, "y": row.get::<i64, _>("pageviews") }));
        sessions.push(json!({ "x": x, "y": row.get::<i64, _>("sessions") }));
    }
    Ok((pageviews, sessions))
}

async fn pageviews(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match authorized_pool(&state, &headers).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (start, end) = date_bounds(&params);
    let bucket = unit(&params, start, end);
    let test_mode = is_test_mode(&params);
    let Ok((pageviews, sessions)) =
        load_series(&pool, website_id, start, end, bucket, test_mode).await
    else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unable to query pageviews",
        );
    };
    let mut body = json!({ "pageviews": pageviews, "sessions": sessions });
    if params.contains_key("compare") {
        let span = end - start;
        let compare_end = start - Duration::milliseconds(1);
        let compare_start = compare_end - span;
        match load_series(
            &pool,
            website_id,
            compare_start,
            compare_end,
            bucket,
            test_mode,
        )
        .await
        {
            Ok((compare_pageviews, compare_sessions)) => {
                body = json!({
                    "pageviews": body["pageviews"],
                    "sessions": body["sessions"],
                    "startDate": start,
                    "endDate": end,
                    "compare": {
                        "pageviews": compare_pageviews,
                        "sessions": compare_sessions,
                        "startDate": compare_start,
                        "endDate": compare_end
                    }
                });
            }
            Err(error) => {
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
            }
        }
    }
    json_response(body)
}

async fn active(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match authorized_pool(&state, &headers).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    match sqlx::query(
        r#"
        SELECT count(DISTINCT session_id)::bigint AS visitors
        FROM website_event
        WHERE website_id = $1
          AND created_at >= NOW() - INTERVAL '5 minutes'
          AND (($2 AND tag = 'test') OR (NOT $2 AND COALESCE(tag, '') != 'test'))
        "#,
    )
    .bind(website_id)
    .bind(is_test_mode(&params))
    .fetch_one(&pool)
    .await
    {
        Ok(row) => json_response(json!({ "visitors": row.get::<i64, _>("visitors") })),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn realtime(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match authorized_pool(&state, &headers).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let end = Utc::now();
    let start = end - Duration::minutes(30);
    let test_mode = is_test_mode(&params);
    let rows = match sqlx::query(
        r#"
        SELECT e.session_id, e.event_name, e.created_at, s.browser, s.os, s.device, s.country,
               e.url_path, e.referrer_domain, e.hostname, e.event_type
        FROM website_event e
        JOIN session s ON s.session_id = e.session_id AND s.website_id = e.website_id
        WHERE e.website_id = $1
          AND e.created_at BETWEEN $2 AND $3
          AND e.event_type != 5
          AND (($4 AND e.tag = 'test') OR (NOT $4 AND COALESCE(e.tag, '') != 'test'))
        ORDER BY e.created_at DESC
        LIMIT 100
        "#,
    )
    .bind(website_id)
    .bind(start)
    .bind(end)
    .bind(test_mode)
    .fetch_all(&pool)
    .await
    {
        Ok(rows) => rows,
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    };
    let (views, visitors) = match load_series(&pool, website_id, start, end, "minute", test_mode)
        .await
    {
        Ok(series) => series,
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    };

    let mut countries: HashMap<String, i64> = HashMap::new();
    let mut urls: HashMap<String, i64> = HashMap::new();
    let mut referrers: HashMap<String, i64> = HashMap::new();
    let mut seen = HashSet::new();
    let mut activity = Vec::new();
    let mut custom_events = 0i64;

    for row in rows {
        let session_id: Uuid = row.get("session_id");
        let event_name: Option<String> = row.get("event_name");
        let country: Option<String> = row.get("country");
        let url_path: String = row.get("url_path");
        let referrer: Option<String> = row.get("referrer_domain");
        let base = json!({
            "sessionId": session_id,
            "eventName": event_name,
            "createdAt": row.get::<Option<DateTime<Utc>>, _>("created_at"),
            "browser": row.get::<Option<String>, _>("browser"),
            "os": row.get::<Option<String>, _>("os"),
            "device": row.get::<Option<String>, _>("device"),
            "country": country,
            "urlPath": url_path,
            "referrerDomain": referrer,
            "hostname": row.get::<Option<String>, _>("hostname")
        });
        if let Some(country) = country.filter(|value| !value.is_empty()) {
            if seen.insert(session_id) {
                *countries.entry(country).or_default() += 1;
                let mut session_event = base.clone();
                session_event["__type"] = json!("session");
                activity.push(session_event);
            }
        } else if seen.insert(session_id) {
            let mut session_event = base.clone();
            session_event["__type"] = json!("session");
            activity.push(session_event);
        }
        *urls.entry(url_path).or_default() += 1;
        if let Some(referrer) = referrer.filter(|value| !value.is_empty()) {
            *referrers.entry(referrer).or_default() += 1;
        }
        if event_name.is_some() {
            custom_events += 1;
        }
        let mut event = base;
        event["__type"] = json!(if event_name.is_some() {
            "event"
        } else {
            "pageview"
        });
        activity.push(event);
    }
    activity.reverse();

    let view_total = views
        .iter()
        .filter_map(|item| item["y"].as_i64())
        .sum::<i64>();
    let visitor_total = visitors
        .iter()
        .filter_map(|item| item["y"].as_i64())
        .sum::<i64>();
    json_response(json!({
        "countries": countries,
        "urls": urls,
        "referrers": referrers,
        "events": activity,
        "series": { "views": views, "visitors": visitors },
        "totals": {
            "views": view_total,
            "visitors": visitor_total,
            "events": custom_events,
            "countries": countries.len()
        },
        "timestamp": Utc::now().timestamp_millis()
    }))
}

async fn date_range(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    headers: HeaderMap,
) -> Response {
    let pool = match authorized_pool(&state, &headers).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    match sqlx::query(
        "SELECT min(created_at) AS min_date, max(created_at) AS max_date FROM website_event WHERE website_id = $1",
    )
    .bind(website_id)
    .fetch_one(&pool)
    .await
    {
        Ok(row) => json_response(json!({
            "minDate": row.get::<Option<DateTime<Utc>>, _>("min_date"),
            "maxDate": row.get::<Option<DateTime<Utc>>, _>("max_date")
        })),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

fn value_column(kind: &str) -> Option<(&'static str, bool)> {
    Some(match kind {
        "path" | "entry" | "exit" => ("url_path", false),
        "fullPath" => ("url_path", false),
        "referrer" | "domain" => ("referrer_domain", false),
        "title" => ("page_title", false),
        "query" => ("url_query", false),
        "event" => ("event_name", false),
        "tag" => ("tag", false),
        "hostname" => ("hostname", false),
        "utmSource" => ("utm_source", false),
        "utmMedium" => ("utm_medium", false),
        "utmCampaign" => ("utm_campaign", false),
        "utmContent" => ("utm_content", false),
        "utmTerm" => ("utm_term", false),
        "browser" => ("browser", true),
        "os" => ("os", true),
        "device" => ("device", true),
        "screen" => ("screen", true),
        "language" => ("language", true),
        "country" => ("country", true),
        "city" => ("city", true),
        "region" => ("region", true),
        "distinctId" => ("distinct_id", true),
        _ => return None,
    })
}

async fn values(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match authorized_pool(&state, &headers).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let Some((column, session_column)) = params
        .get("type")
        .and_then(|kind| value_column(kind.as_str()))
    else {
        return error_response(StatusCode::BAD_REQUEST, "Invalid value type");
    };
    let (start, end) = date_bounds(&params);
    let search = params
        .get("search")
        .map(|value| format!("%{value}%"))
        .unwrap_or_else(|| "%".to_string());
    let sql = if session_column {
        format!(
            r#"
            SELECT {column}::text AS value
            FROM session
            WHERE website_id = $1 AND created_at BETWEEN $2 AND $3
              AND {column} IS NOT NULL AND {column}::text ILIKE $4
            GROUP BY {column}
            ORDER BY count(*) DESC
            LIMIT 10
            "#
        )
    } else {
        format!(
            r#"
            SELECT {column}::text AS value
            FROM website_event
            WHERE website_id = $1 AND created_at BETWEEN $2 AND $3
              AND {column} IS NOT NULL AND {column}::text ILIKE $4
            GROUP BY {column}
            ORDER BY count(*) DESC
            LIMIT 10
            "#
        )
    };
    match sqlx::query(&sql)
        .bind(website_id)
        .bind(start)
        .bind(end)
        .bind(search)
        .fetch_all(&pool)
        .await
    {
        Ok(rows) => json_response(Value::Array(
            rows.into_iter()
                .map(|row| json!({ "value": row.get::<String, _>("value") }))
                .collect(),
        )),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

fn expanded_column(kind: &str) -> Option<(&'static str, bool, bool)> {
    Some(match kind {
        "path" | "entry" | "exit" => ("e.url_path", false, false),
        "fullPath" => (
            "CASE WHEN COALESCE(e.url_query, '') != '' THEN e.url_path || '?' || e.url_query ELSE e.url_path END",
            false,
            false,
        ),
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
        "channel" => (
            "CASE WHEN COALESCE(e.referrer_domain, '') = '' THEN 'direct' ELSE 'referral' END",
            false,
            false,
        ),
        _ => return None,
    })
}

async fn expanded_metrics(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match authorized_pool(&state, &headers).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let Some((column, join_session, custom_only)) = params
        .get("type")
        .and_then(|kind| expanded_column(kind.as_str()))
    else {
        return error_response(StatusCode::BAD_REQUEST, "Invalid metric type");
    };
    let (start, end) = date_bounds(&params);
    let limit = params
        .get("limit")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(500)
        .clamp(1, 1000);
    let offset = params
        .get("offset")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0)
        .max(0);
    let search = params
        .get("search")
        .map(|value| format!("%{value}%"))
        .unwrap_or_else(|| "%".to_string());
    let join = if join_session {
        "JOIN session s ON s.session_id = e.session_id AND s.website_id = e.website_id"
    } else {
        "LEFT JOIN session s ON s.session_id = e.session_id AND s.website_id = e.website_id"
    };
    let event_filter = if custom_only {
        "e.event_type = 2"
    } else {
        "e.event_type NOT IN (2, 5)"
    };
    let sql = format!(
        r#"
        SELECT name,
               sum(c)::bigint AS pageviews,
               count(DISTINCT session_id)::bigint AS visitors,
               count(DISTINCT visit_id)::bigint AS visits,
               sum(CASE WHEN c = 1 THEN 1 ELSE 0 END)::bigint AS bounces,
               COALESCE(sum(EXTRACT(EPOCH FROM (max_time - min_time))), 0)::bigint AS totaltime
        FROM (
          SELECT ({column})::text AS name, e.session_id, e.visit_id, count(*)::bigint AS c,
                 min(e.created_at) AS min_time, max(e.created_at) AS max_time
          FROM website_event e
          {join}
          WHERE e.website_id = $1
            AND e.created_at BETWEEN $2 AND $3
            AND {event_filter}
            AND (($4 AND e.tag = 'test') OR (NOT $4 AND COALESCE(e.tag, '') != 'test'))
          GROUP BY ({column})::text, e.session_id, e.visit_id
        ) metrics
        WHERE COALESCE(name, '') != '' AND name ILIKE $5
        GROUP BY name
        ORDER BY visitors DESC, visits DESC
        LIMIT $6 OFFSET $7
        "#
    );
    match sqlx::query(&sql)
        .bind(website_id)
        .bind(start)
        .bind(end)
        .bind(is_test_mode(&params))
        .bind(search)
        .bind(limit)
        .bind(offset)
        .fetch_all(&pool)
        .await
    {
        Ok(rows) => json_response(Value::Array(
            rows.into_iter()
                .map(|row| {
                    json!({
                        "name": row.get::<String, _>("name"),
                        "pageviews": row.get::<i64, _>("pageviews"),
                        "visitors": row.get::<i64, _>("visitors"),
                        "visits": row.get::<i64, _>("visits"),
                        "bounces": row.get::<i64, _>("bounces"),
                        "totaltime": row.get::<i64, _>("totaltime")
                    })
                })
                .collect(),
        )),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn events(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match authorized_pool(&state, &headers).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (start, end) = date_bounds(&params);
    let (page, page_size, offset) = page_params(&params);
    let search = params
        .get("search")
        .map(|value| format!("%{value}%"))
        .unwrap_or_else(|| "%".to_string());
    let test_mode = is_test_mode(&params);
    let count = match sqlx::query(
        r#"
        SELECT count(*)::bigint AS count
        FROM website_event e
        WHERE e.website_id = $1 AND e.created_at BETWEEN $2 AND $3 AND e.event_type != 5
          AND (($4 AND e.tag = 'test') OR (NOT $4 AND COALESCE(e.tag, '') != 'test'))
          AND ((e.event_type = 2 AND COALESCE(e.event_name, '') ILIKE $5)
            OR (e.event_type != 2 AND e.url_path ILIKE $5))
        "#,
    )
    .bind(website_id)
    .bind(start)
    .bind(end)
    .bind(test_mode)
    .bind(&search)
    .fetch_one(&pool)
    .await
    {
        Ok(row) => row.get::<i64, _>("count"),
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    };
    let rows = match sqlx::query(
        r#"
        SELECT e.event_id, e.website_id, e.session_id, e.visit_id, e.created_at, e.hostname,
               e.url_path, e.url_query, e.referrer_path, e.referrer_query, e.referrer_domain,
               s.country, s.city, s.device, s.os, s.browser, e.page_title, e.event_type,
               e.event_name, EXISTS(
                 SELECT 1 FROM event_data d
                 WHERE d.website_event_id = e.event_id AND d.website_id = e.website_id
               ) AS has_data
        FROM website_event e
        LEFT JOIN session s ON s.session_id = e.session_id AND s.website_id = e.website_id
        WHERE e.website_id = $1 AND e.created_at BETWEEN $2 AND $3 AND e.event_type != 5
          AND (($4 AND e.tag = 'test') OR (NOT $4 AND COALESCE(e.tag, '') != 'test'))
          AND ((e.event_type = 2 AND COALESCE(e.event_name, '') ILIKE $5)
            OR (e.event_type != 2 AND e.url_path ILIKE $5))
        ORDER BY e.created_at DESC
        LIMIT $6 OFFSET $7
        "#,
    )
    .bind(website_id)
    .bind(start)
    .bind(end)
    .bind(test_mode)
    .bind(search)
    .bind(page_size)
    .bind(offset)
    .fetch_all(&pool)
    .await
    {
        Ok(rows) => rows,
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    };
    let data = rows
        .into_iter()
        .map(|row| {
            json!({
                "id": row.get::<Uuid, _>("event_id"),
                "websiteId": row.get::<Uuid, _>("website_id"),
                "sessionId": row.get::<Uuid, _>("session_id"),
                "visitId": row.get::<Uuid, _>("visit_id"),
                "createdAt": row.get::<Option<DateTime<Utc>>, _>("created_at"),
                "hostname": row.get::<Option<String>, _>("hostname"),
                "urlPath": row.get::<String, _>("url_path"),
                "urlQuery": row.get::<Option<String>, _>("url_query"),
                "referrerPath": row.get::<Option<String>, _>("referrer_path"),
                "referrerQuery": row.get::<Option<String>, _>("referrer_query"),
                "referrerDomain": row.get::<Option<String>, _>("referrer_domain"),
                "country": row.get::<Option<String>, _>("country"),
                "city": row.get::<Option<String>, _>("city"),
                "device": row.get::<Option<String>, _>("device"),
                "os": row.get::<Option<String>, _>("os"),
                "browser": row.get::<Option<String>, _>("browser"),
                "pageTitle": row.get::<Option<String>, _>("page_title"),
                "eventType": row.get::<i32, _>("event_type"),
                "eventName": row.get::<Option<String>, _>("event_name"),
                "hasData": row.get::<bool, _>("has_data")
            })
        })
        .collect::<Vec<_>>();
    json_response(json!({ "data": data, "count": count, "page": page, "pageSize": page_size }))
}

async fn event_series(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match authorized_pool(&state, &headers).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (start, end) = date_bounds(&params);
    let bucket = unit(&params, start, end);
    let limit = params
        .get("limit")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(500)
        .clamp(1, 1000);
    let sql = format!(
        r#"
        SELECT event_name AS x,
               to_char(date_trunc('{bucket}', created_at AT TIME ZONE 'UTC'), 'YYYY-MM-DD HH24:MI:SS') AS t,
               count(*)::bigint AS y
        FROM website_event
        WHERE website_id = $1 AND created_at BETWEEN $2 AND $3 AND event_type = 2
          AND (($4 AND tag = 'test') OR (NOT $4 AND COALESCE(tag, '') != 'test'))
          AND event_name IN (
            SELECT event_name FROM website_event
            WHERE website_id = $1 AND created_at BETWEEN $2 AND $3 AND event_type = 2
            GROUP BY event_name ORDER BY count(*) DESC LIMIT $5
          )
        GROUP BY 1, 2 ORDER BY 2
        "#
    );
    match sqlx::query(&sql)
        .bind(website_id)
        .bind(start)
        .bind(end)
        .bind(is_test_mode(&params))
        .bind(limit)
        .fetch_all(&pool)
        .await
    {
        Ok(rows) => json_response(Value::Array(
            rows.into_iter()
                .map(|row| {
                    json!({
                        "x": row.get::<Option<String>, _>("x"),
                        "t": row.get::<String, _>("t"),
                        "y": row.get::<i64, _>("y")
                    })
                })
                .collect(),
        )),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn event_stat_values(
    pool: &PgPool,
    website_id: Uuid,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    test_mode: bool,
) -> Result<Value, sqlx::Error> {
    let row = sqlx::query(
        r#"
        SELECT count(*) FILTER (WHERE event_type = 2)::bigint AS events,
               count(DISTINCT session_id) FILTER (WHERE event_type = 2)::bigint AS visitors,
               count(DISTINCT visit_id) FILTER (WHERE event_type = 2)::bigint AS visits,
               count(DISTINCT event_name) FILTER (WHERE event_type = 2)::bigint AS unique_events
        FROM website_event
        WHERE website_id = $1 AND created_at BETWEEN $2 AND $3
          AND (($4 AND tag = 'test') OR (NOT $4 AND COALESCE(tag, '') != 'test'))
        "#,
    )
    .bind(website_id)
    .bind(start)
    .bind(end)
    .bind(test_mode)
    .fetch_one(pool)
    .await?;
    Ok(json!({
        "events": row.get::<i64, _>("events"),
        "visitors": row.get::<i64, _>("visitors"),
        "visits": row.get::<i64, _>("visits"),
        "uniqueEvents": row.get::<i64, _>("unique_events")
    }))
}

async fn event_stats(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match authorized_pool(&state, &headers).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (start, end) = date_bounds(&params);
    let span = end - start;
    let compare_end = start - Duration::milliseconds(1);
    let compare_start = compare_end - span;
    let test_mode = is_test_mode(&params);
    match (
        event_stat_values(&pool, website_id, start, end, test_mode).await,
        event_stat_values(&pool, website_id, compare_start, compare_end, test_mode).await,
    ) {
        (Ok(mut data), Ok(comparison)) => {
            data["comparison"] = comparison;
            json_response(json!({ "data": data }))
        }
        (Err(error), _) | (_, Err(error)) => {
            error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
        }
    }
}

fn event_data_json(row: &sqlx::postgres::PgRow) -> Value {
    json!({
        "websiteId": row.get::<Uuid, _>("website_id"),
        "eventId": row.get::<Uuid, _>("website_event_id"),
        "eventName": row.get::<Option<String>, _>("event_name"),
        "dataKey": row.get::<String, _>("data_key"),
        "stringValue": row.get::<Option<String>, _>("string_value"),
        "numberValue": decimal_value(row.get::<Option<Decimal>, _>("number_value")),
        "dateValue": row.get::<Option<DateTime<Utc>>, _>("date_value"),
        "dataType": row.get::<i32, _>("data_type"),
        "createdAt": row.get::<Option<DateTime<Utc>>, _>("created_at")
    })
}

async fn event_data(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match authorized_pool(&state, &headers).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (start, end) = date_bounds(&params);
    let (page, page_size, offset) = page_params(&params);
    let test_mode = is_test_mode(&params);
    let count = match sqlx::query(
        r#"
        SELECT count(DISTINCT d.website_event_id)::bigint AS count
        FROM event_data d
        JOIN website_event e ON e.event_id = d.website_event_id AND e.website_id = d.website_id
        WHERE d.website_id = $1 AND d.created_at BETWEEN $2 AND $3
          AND (($4 AND e.tag = 'test') OR (NOT $4 AND COALESCE(e.tag, '') != 'test'))
        "#,
    )
    .bind(website_id)
    .bind(start)
    .bind(end)
    .bind(test_mode)
    .fetch_one(&pool)
    .await
    {
        Ok(row) => row.get::<i64, _>("count"),
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    };
    let rows = match sqlx::query(
        r#"
        WITH selected AS (
          SELECT d.website_event_id, max(d.created_at) AS created_at
          FROM event_data d
          JOIN website_event e ON e.event_id = d.website_event_id AND e.website_id = d.website_id
          WHERE d.website_id = $1 AND d.created_at BETWEEN $2 AND $3
            AND (($4 AND e.tag = 'test') OR (NOT $4 AND COALESCE(e.tag, '') != 'test'))
          GROUP BY d.website_event_id
          ORDER BY max(d.created_at) DESC
          LIMIT $5 OFFSET $6
        )
        SELECT d.website_id, d.website_event_id, e.event_name, d.data_key, d.string_value,
               d.number_value, d.date_value, d.data_type, d.created_at
        FROM event_data d
        JOIN website_event e ON e.event_id = d.website_event_id AND e.website_id = d.website_id
        JOIN selected ON selected.website_event_id = d.website_event_id
        WHERE d.website_id = $1 AND d.created_at BETWEEN $2 AND $3
        ORDER BY d.created_at DESC
        "#,
    )
    .bind(website_id)
    .bind(start)
    .bind(end)
    .bind(test_mode)
    .bind(page_size)
    .bind(offset)
    .fetch_all(&pool)
    .await
    {
        Ok(rows) => rows,
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    };
    json_response(json!({
        "data": rows.iter().map(event_data_json).collect::<Vec<_>>(),
        "count": count,
        "page": page,
        "pageSize": page_size
    }))
}

async fn event_data_stats(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match authorized_pool(&state, &headers).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (start, end) = date_bounds(&params);
    match sqlx::query(
        r#"
        SELECT count(DISTINCT d.website_event_id)::bigint AS events,
               count(DISTINCT d.data_key)::bigint AS properties,
               count(*)::bigint AS records
        FROM event_data d
        JOIN website_event e ON e.event_id = d.website_event_id AND e.website_id = d.website_id
        WHERE d.website_id = $1 AND d.created_at BETWEEN $2 AND $3
          AND (($4 AND e.tag = 'test') OR (NOT $4 AND COALESCE(e.tag, '') != 'test'))
        "#,
    )
    .bind(website_id)
    .bind(start)
    .bind(end)
    .bind(is_test_mode(&params))
    .fetch_one(&pool)
    .await
    {
        Ok(row) => json_response(json!({
            "events": row.get::<i64, _>("events"),
            "properties": row.get::<i64, _>("properties"),
            "records": row.get::<i64, _>("records")
        })),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn event_data_events(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match authorized_pool(&state, &headers).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (start, end) = date_bounds(&params);
    let event = params.get("event").cloned();
    match sqlx::query(
        r#"
        SELECT e.event_name, d.data_key, d.data_type, d.string_value, count(*)::bigint AS total
        FROM event_data d
        JOIN website_event e ON e.event_id = d.website_event_id AND e.website_id = d.website_id
        WHERE d.website_id = $1 AND d.created_at BETWEEN $2 AND $3
          AND ($4::text IS NULL OR e.event_name = $4)
          AND (($5 AND e.tag = 'test') OR (NOT $5 AND COALESCE(e.tag, '') != 'test'))
        GROUP BY e.event_name, d.data_key, d.data_type, d.string_value
        ORDER BY 1, 2, 3, 5 DESC
        LIMIT 500
        "#,
    )
    .bind(website_id)
    .bind(start)
    .bind(end)
    .bind(event)
    .bind(is_test_mode(&params))
    .fetch_all(&pool)
    .await
    {
        Ok(rows) => json_response(Value::Array(
            rows.into_iter()
                .map(|row| {
                    json!({
                        "eventName": row.get::<Option<String>, _>("event_name"),
                        "propertyName": row.get::<String, _>("data_key"),
                        "dataType": row.get::<i32, _>("data_type"),
                        "propertyValue": row.get::<Option<String>, _>("string_value"),
                        "total": row.get::<i64, _>("total")
                    })
                })
                .collect(),
        )),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn event_data_fields(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match authorized_pool(&state, &headers).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (start, end) = date_bounds(&params);
    let event_name = params.get("eventName").cloned();
    match sqlx::query(
        r#"
        SELECT d.data_key, d.data_type, count(*)::bigint AS total
        FROM event_data d
        JOIN website_event e ON e.event_id = d.website_event_id AND e.website_id = d.website_id
        WHERE d.website_id = $1 AND d.created_at BETWEEN $2 AND $3
          AND ($4::text IS NULL OR e.event_name = $4)
          AND (($5 AND e.tag = 'test') OR (NOT $5 AND COALESCE(e.tag, '') != 'test'))
        GROUP BY d.data_key, d.data_type
        ORDER BY total DESC, d.data_key
        "#,
    )
    .bind(website_id)
    .bind(start)
    .bind(end)
    .bind(event_name)
    .bind(is_test_mode(&params))
    .fetch_all(&pool)
    .await
    {
        Ok(rows) => json_response(Value::Array(
            rows.into_iter()
                .map(|row| {
                    json!({
                        "propertyName": row.get::<String, _>("data_key"),
                        "dataType": row.get::<i32, _>("data_type"),
                        "total": row.get::<i64, _>("total")
                    })
                })
                .collect(),
        )),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn event_data_properties(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match authorized_pool(&state, &headers).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (start, end) = date_bounds(&params);
    match sqlx::query(
        r#"
        SELECT e.event_name, d.data_key, d.data_type, count(*)::bigint AS total
        FROM event_data d
        JOIN website_event e ON e.event_id = d.website_event_id AND e.website_id = d.website_id
        WHERE d.website_id = $1 AND d.created_at BETWEEN $2 AND $3
          AND (($4 AND e.tag = 'test') OR (NOT $4 AND COALESCE(e.tag, '') != 'test'))
        GROUP BY e.event_name, d.data_key, d.data_type
        ORDER BY total DESC
        LIMIT 500
        "#,
    )
    .bind(website_id)
    .bind(start)
    .bind(end)
    .bind(is_test_mode(&params))
    .fetch_all(&pool)
    .await
    {
        Ok(rows) => json_response(Value::Array(
            rows.into_iter()
                .map(|row| {
                    json!({
                        "eventName": row.get::<Option<String>, _>("event_name"),
                        "propertyName": row.get::<String, _>("data_key"),
                        "dataType": row.get::<i32, _>("data_type"),
                        "total": row.get::<i64, _>("total")
                    })
                })
                .collect(),
        )),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn event_data_values(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match authorized_pool(&state, &headers).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let Some(property_name) = params.get("propertyName") else {
        return error_response(StatusCode::BAD_REQUEST, "propertyName is required");
    };
    let (start, end) = date_bounds(&params);
    let event_name = params.get("eventName").cloned();
    let data_type = params
        .get("dataType")
        .and_then(|value| value.parse::<i32>().ok());
    match sqlx::query(
        r#"
        SELECT CASE
                 WHEN d.data_type = 2 THEN replace(d.number_value::text, '.0000', '')
                 WHEN d.data_type = 4 THEN to_char(date_trunc('hour', d.date_value), 'YYYY-MM-DD HH24:MI:SS')
                 ELSE d.string_value
               END AS value,
               count(*)::bigint AS total
        FROM event_data d
        JOIN website_event e ON e.event_id = d.website_event_id AND e.website_id = d.website_id
        WHERE d.website_id = $1 AND d.created_at BETWEEN $2 AND $3 AND d.data_key = $4
          AND ($5::text IS NULL OR e.event_name = $5)
          AND ($6::int IS NULL OR d.data_type = $6)
          AND (($7 AND e.tag = 'test') OR (NOT $7 AND COALESCE(e.tag, '') != 'test'))
        GROUP BY value
        ORDER BY total DESC
        LIMIT 100
        "#,
    )
    .bind(website_id)
    .bind(start)
    .bind(end)
    .bind(property_name)
    .bind(event_name)
    .bind(data_type)
    .bind(is_test_mode(&params))
    .fetch_all(&pool)
    .await
    {
        Ok(rows) => json_response(Value::Array(
            rows.into_iter()
                .map(|row| {
                    json!({
                        "value": row.get::<Option<String>, _>("value"),
                        "total": row.get::<i64, _>("total")
                    })
                })
                .collect(),
        )),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn event_data_by_id(
    State(state): State<AppState>,
    Path((website_id, event_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Response {
    let pool = match authorized_pool(&state, &headers).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    match sqlx::query(
        r#"
        SELECT d.website_id, d.website_event_id, e.event_name, d.data_key, d.string_value,
               d.number_value, d.date_value, d.data_type, d.created_at
        FROM event_data d
        JOIN website_event e ON e.event_id = d.website_event_id AND e.website_id = d.website_id
        WHERE d.website_id = $1 AND d.website_event_id = $2
        ORDER BY d.data_key
        "#,
    )
    .bind(website_id)
    .bind(event_id)
    .fetch_all(&pool)
    .await
    {
        Ok(rows) => json_response(Value::Array(rows.iter().map(event_data_json).collect())),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn sessions(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match authorized_pool(&state, &headers).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (start, end) = date_bounds(&params);
    let (page, page_size, offset) = page_params(&params);
    let search = params
        .get("search")
        .map(|value| format!("%{value}%"))
        .unwrap_or_else(|| "%".to_string());
    let test_mode = is_test_mode(&params);
    let count = match sqlx::query(
        r#"
        SELECT count(DISTINCT e.session_id)::bigint AS count
        FROM website_event e
        JOIN session s ON s.session_id = e.session_id AND s.website_id = e.website_id
        WHERE e.website_id = $1 AND e.created_at BETWEEN $2 AND $3 AND e.event_type != 5
          AND (($4 AND e.tag = 'test') OR (NOT $4 AND COALESCE(e.tag, '') != 'test'))
          AND (COALESCE(s.distinct_id, '') ILIKE $5 OR COALESCE(s.city, '') ILIKE $5
            OR COALESCE(s.browser, '') ILIKE $5 OR COALESCE(s.os, '') ILIKE $5
            OR COALESCE(s.device, '') ILIKE $5)
        "#,
    )
    .bind(website_id)
    .bind(start)
    .bind(end)
    .bind(test_mode)
    .bind(&search)
    .fetch_one(&pool)
    .await
    {
        Ok(row) => row.get::<i64, _>("count"),
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    };
    let rows = match sqlx::query(
        r#"
        SELECT s.session_id, s.website_id, max(e.hostname) AS hostname, s.browser, s.os, s.device,
               s.screen, s.language, s.country, s.region, s.city, min(e.created_at) AS first_at,
               max(e.created_at) AS last_at, count(DISTINCT e.visit_id)::bigint AS visits,
               count(*) FILTER (WHERE e.event_type = 1)::bigint AS views,
               count(*) FILTER (WHERE e.event_type = 2)::bigint AS events,
               max(e.created_at) AS created_at
        FROM website_event e
        JOIN session s ON s.session_id = e.session_id AND s.website_id = e.website_id
        WHERE e.website_id = $1 AND e.created_at BETWEEN $2 AND $3 AND e.event_type != 5
          AND (($4 AND e.tag = 'test') OR (NOT $4 AND COALESCE(e.tag, '') != 'test'))
          AND (COALESCE(s.distinct_id, '') ILIKE $5 OR COALESCE(s.city, '') ILIKE $5
            OR COALESCE(s.browser, '') ILIKE $5 OR COALESCE(s.os, '') ILIKE $5
            OR COALESCE(s.device, '') ILIKE $5)
        GROUP BY s.session_id, s.website_id, s.browser, s.os, s.device, s.screen,
                 s.language, s.country, s.region, s.city
        ORDER BY max(e.created_at) DESC, s.session_id
        LIMIT $6 OFFSET $7
        "#,
    )
    .bind(website_id)
    .bind(start)
    .bind(end)
    .bind(test_mode)
    .bind(search)
    .bind(page_size)
    .bind(offset)
    .fetch_all(&pool)
    .await
    {
        Ok(rows) => rows,
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    };
    let data = rows
        .into_iter()
        .map(|row| {
            json!({
                "id": row.get::<Uuid, _>("session_id"),
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
                "firstAt": row.get::<Option<DateTime<Utc>>, _>("first_at"),
                "lastAt": row.get::<Option<DateTime<Utc>>, _>("last_at"),
                "visits": row.get::<i64, _>("visits"),
                "views": row.get::<i64, _>("views"),
                "events": row.get::<i64, _>("events"),
                "createdAt": row.get::<Option<DateTime<Utc>>, _>("created_at")
            })
        })
        .collect::<Vec<_>>();
    json_response(json!({ "data": data, "count": count, "page": page, "pageSize": page_size }))
}

async fn session_stats(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match authorized_pool(&state, &headers).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (start, end) = date_bounds(&params);
    match sqlx::query(
        r#"
        SELECT count(*) FILTER (WHERE e.event_type = 1)::bigint AS pageviews,
               count(DISTINCT e.session_id)::bigint AS visitors,
               count(DISTINCT e.visit_id)::bigint AS visits,
               count(DISTINCT s.country)::bigint AS countries,
               count(*) FILTER (WHERE e.event_type = 2)::bigint AS events
        FROM website_event e
        JOIN session s ON s.session_id = e.session_id AND s.website_id = e.website_id
        WHERE e.website_id = $1 AND e.created_at BETWEEN $2 AND $3 AND e.event_type != 5
          AND (($4 AND e.tag = 'test') OR (NOT $4 AND COALESCE(e.tag, '') != 'test'))
        "#,
    )
    .bind(website_id)
    .bind(start)
    .bind(end)
    .bind(is_test_mode(&params))
    .fetch_one(&pool)
    .await
    {
        Ok(row) => json_response(json!({
            "pageviews": { "value": row.get::<i64, _>("pageviews") },
            "visitors": { "value": row.get::<i64, _>("visitors") },
            "visits": { "value": row.get::<i64, _>("visits") },
            "countries": { "value": row.get::<i64, _>("countries") },
            "events": { "value": row.get::<i64, _>("events") }
        })),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn session_weekly(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match authorized_pool(&state, &headers).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (start, end) = date_bounds(&params);
    let rows = match sqlx::query(
        r#"
        SELECT extract(dow FROM created_at AT TIME ZONE 'UTC')::int AS day,
               extract(hour FROM created_at AT TIME ZONE 'UTC')::int AS hour,
               count(DISTINCT session_id)::bigint AS value
        FROM website_event
        WHERE website_id = $1 AND created_at BETWEEN $2 AND $3 AND event_type NOT IN (2, 5)
          AND (($4 AND tag = 'test') OR (NOT $4 AND COALESCE(tag, '') != 'test'))
        GROUP BY 1, 2
        "#,
    )
    .bind(website_id)
    .bind(start)
    .bind(end)
    .bind(is_test_mode(&params))
    .fetch_all(&pool)
    .await
    {
        Ok(rows) => rows,
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    };
    let mut weekly = vec![vec![0i64; 24]; 7];
    for row in rows {
        let day = row.get::<i32, _>("day") as usize;
        let hour = row.get::<i32, _>("hour") as usize;
        if day < 7 && hour < 24 {
            weekly[day][hour] = row.get::<i64, _>("value");
        }
    }
    json_response(json!(weekly))
}

async fn session_detail(
    State(state): State<AppState>,
    Path((website_id, session_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Response {
    let pool = match authorized_pool(&state, &headers).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let row = match sqlx::query(
        r#"
        WITH visit_stats AS (
          SELECT visit_id, min(created_at) AS min_at, max(created_at) AS max_at,
                 count(*) FILTER (WHERE event_type = 1)::bigint AS views,
                 count(*) FILTER (WHERE event_type = 2)::bigint AS events
          FROM website_event
          WHERE website_id = $1 AND session_id = $2 AND event_type != 5
          GROUP BY visit_id
        )
        SELECT s.session_id, s.website_id, s.distinct_id, s.browser, s.os, s.device, s.screen,
               s.language, s.country, s.region, s.city, min(v.min_at) AS first_at,
               max(v.max_at) AS last_at, count(*)::bigint AS visits,
               COALESCE(sum(v.views), 0)::bigint AS views,
               COALESCE(sum(v.events), 0)::bigint AS events,
               COALESCE(sum(EXTRACT(EPOCH FROM (v.max_at - v.min_at))), 0)::bigint AS totaltime
        FROM session s
        JOIN visit_stats v ON true
        WHERE s.website_id = $1 AND s.session_id = $2
        GROUP BY s.session_id, s.website_id, s.distinct_id, s.browser, s.os, s.device, s.screen,
                 s.language, s.country, s.region, s.city
        "#,
    )
    .bind(website_id)
    .bind(session_id)
    .fetch_optional(&pool)
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => return with_rust_header(StatusCode::NOT_FOUND.into_response()),
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    };
    let distinct_id: Option<String> = row.get("distinct_id");
    let mut distinct_ids = Vec::new();
    if let Ok(rows) = sqlx::query(
        "SELECT DISTINCT distinct_id FROM session_link WHERE website_id = $1 AND session_id = $2 ORDER BY distinct_id",
    )
    .bind(website_id)
    .bind(session_id)
    .fetch_all(&pool)
    .await
    {
        distinct_ids.extend(rows.into_iter().map(|item| item.get::<String, _>("distinct_id")));
    }
    if distinct_ids.is_empty() {
        if let Some(value) = distinct_id.clone() {
            distinct_ids.push(value);
        }
    }
    let stitched_session_count = if distinct_ids.len() == 1 {
        sqlx::query(
            "SELECT count(DISTINCT session_id)::bigint AS count FROM session_link WHERE website_id = $1 AND distinct_id = $2",
        )
        .bind(website_id)
        .bind(&distinct_ids[0])
        .fetch_optional(&pool)
        .await
        .ok()
        .flatten()
        .map(|item| item.get::<i64, _>("count").max(1))
        .unwrap_or(1)
    } else {
        1
    };
    json_response(json!({
        "id": row.get::<Uuid, _>("session_id"),
        "websiteId": row.get::<Uuid, _>("website_id"),
        "distinctId": if distinct_ids.len() == 1 { json!(distinct_ids[0]) } else { Value::Null },
        "browser": row.get::<Option<String>, _>("browser"),
        "os": row.get::<Option<String>, _>("os"),
        "device": row.get::<Option<String>, _>("device"),
        "screen": row.get::<Option<String>, _>("screen"),
        "language": row.get::<Option<String>, _>("language"),
        "country": row.get::<Option<String>, _>("country"),
        "region": row.get::<Option<String>, _>("region"),
        "city": row.get::<Option<String>, _>("city"),
        "firstAt": row.get::<Option<DateTime<Utc>>, _>("first_at"),
        "lastAt": row.get::<Option<DateTime<Utc>>, _>("last_at"),
        "visits": row.get::<i64, _>("visits"),
        "views": row.get::<i64, _>("views"),
        "events": row.get::<i64, _>("events"),
        "totaltime": row.get::<i64, _>("totaltime"),
        "canDelete": true,
        "distinctIds": distinct_ids,
        "stitchedSessionCount": stitched_session_count
    }))
}

async fn delete_session(
    State(state): State<AppState>,
    Path((website_id, session_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Response {
    let pool = match authorized_pool(&state, &headers).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let mut transaction = match pool.begin().await {
        Ok(transaction) => transaction,
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    };
    for statement in [
        "DELETE FROM event_data WHERE website_id = $1 AND website_event_id IN (SELECT event_id FROM website_event WHERE website_id = $1 AND session_id = $2)",
        "DELETE FROM session_data WHERE website_id = $1 AND session_id = $2",
        "DELETE FROM revenue WHERE website_id = $1 AND session_id = $2",
        "DELETE FROM session_link WHERE website_id = $1 AND session_id = $2",
        "DELETE FROM website_event WHERE website_id = $1 AND session_id = $2",
    ] {
        if let Err(error) = sqlx::query(statement)
            .bind(website_id)
            .bind(session_id)
            .execute(&mut *transaction)
            .await
        {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string());
        }
    }
    let result = match sqlx::query("DELETE FROM session WHERE website_id = $1 AND session_id = $2")
        .bind(website_id)
        .bind(session_id)
        .execute(&mut *transaction)
        .await
    {
        Ok(result) => result,
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    };
    if let Err(error) = transaction.commit().await {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string());
    }
    if result.rows_affected() == 0 {
        with_rust_header(StatusCode::NOT_FOUND.into_response())
    } else {
        json_response(json!({ "ok": true }))
    }
}

async fn session_activity(
    State(state): State<AppState>,
    Path((website_id, session_id)): Path<(Uuid, Uuid)>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match authorized_pool(&state, &headers).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (start, end) = date_bounds(&params);
    match sqlx::query(
        r#"
        SELECT e.created_at, e.url_path, e.url_query, e.referrer_domain, e.event_id,
               e.event_type, e.event_name, e.visit_id, e.hostname,
               EXISTS(SELECT 1 FROM event_data d WHERE d.website_id = e.website_id
                 AND d.website_event_id = e.event_id AND d.created_at BETWEEN $3 AND $4) AS has_data
        FROM website_event e
        WHERE e.website_id = $1 AND e.session_id = $2 AND e.created_at BETWEEN $3 AND $4
          AND e.event_type != 5
        ORDER BY e.created_at DESC
        LIMIT 500
        "#,
    )
    .bind(website_id)
    .bind(session_id)
    .bind(start)
    .bind(end)
    .fetch_all(&pool)
    .await
    {
        Ok(rows) => json_response(Value::Array(
            rows.into_iter()
                .map(|row| {
                    json!({
                        "createdAt": row.get::<Option<DateTime<Utc>>, _>("created_at"),
                        "urlPath": row.get::<String, _>("url_path"),
                        "urlQuery": row.get::<Option<String>, _>("url_query"),
                        "referrerDomain": row.get::<Option<String>, _>("referrer_domain"),
                        "eventId": row.get::<Uuid, _>("event_id"),
                        "eventType": row.get::<i32, _>("event_type"),
                        "eventName": row.get::<Option<String>, _>("event_name"),
                        "visitId": row.get::<Uuid, _>("visit_id"),
                        "hostname": row.get::<Option<String>, _>("hostname"),
                        "hasData": row.get::<bool, _>("has_data")
                    })
                })
                .collect(),
        )),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

fn session_data_json(row: &sqlx::postgres::PgRow) -> Value {
    json!({
        "websiteId": row.get::<Uuid, _>("website_id"),
        "sessionId": row.get::<Uuid, _>("session_id"),
        "dataKey": row.get::<String, _>("data_key"),
        "dataType": row.get::<i32, _>("data_type"),
        "stringValue": row.get::<Option<String>, _>("string_value").map(|value| value.replace(".0000", "")),
        "numberValue": decimal_value(row.get::<Option<Decimal>, _>("number_value")),
        "dateValue": row.get::<Option<DateTime<Utc>>, _>("date_value"),
        "createdAt": row.get::<Option<DateTime<Utc>>, _>("created_at")
    })
}

async fn session_properties(
    State(state): State<AppState>,
    Path((website_id, session_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Response {
    let pool = match authorized_pool(&state, &headers).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    match sqlx::query(
        r#"
        SELECT website_id, session_id, data_key, data_type, string_value, number_value,
               date_value, created_at
        FROM session_data
        WHERE website_id = $1 AND session_id = $2
        ORDER BY data_key
        "#,
    )
    .bind(website_id)
    .bind(session_id)
    .fetch_all(&pool)
    .await
    {
        Ok(rows) => json_response(Value::Array(rows.iter().map(session_data_json).collect())),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn session_data_properties(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match authorized_pool(&state, &headers).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (start, end) = date_bounds(&params);
    let property_name = params.get("propertyName").cloned();
    match sqlx::query(
        r#"
        WITH filtered_sessions AS (
          SELECT DISTINCT session_id
          FROM website_event
          WHERE website_id = $1 AND created_at BETWEEN $2 AND $3 AND event_type != 5
            AND (($5 AND tag = 'test') OR (NOT $5 AND COALESCE(tag, '') != 'test'))
        )
        SELECT d.data_key, d.data_type, count(DISTINCT d.session_id)::bigint AS total
        FROM session_data d
        JOIN filtered_sessions f ON f.session_id = d.session_id
        WHERE d.website_id = $1 AND ($4::text IS NULL OR d.data_key = $4)
        GROUP BY d.data_key, d.data_type
        ORDER BY total DESC, d.data_key
        LIMIT 500
        "#,
    )
    .bind(website_id)
    .bind(start)
    .bind(end)
    .bind(property_name)
    .bind(is_test_mode(&params))
    .fetch_all(&pool)
    .await
    {
        Ok(rows) => json_response(Value::Array(
            rows.into_iter()
                .map(|row| {
                    json!({
                        "propertyName": row.get::<String, _>("data_key"),
                        "dataType": row.get::<i32, _>("data_type"),
                        "total": row.get::<i64, _>("total")
                    })
                })
                .collect(),
        )),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn session_data_values(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match authorized_pool(&state, &headers).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let Some(property_name) = params.get("propertyName") else {
        return error_response(StatusCode::BAD_REQUEST, "propertyName is required");
    };
    let (start, end) = date_bounds(&params);
    let data_type = params
        .get("dataType")
        .and_then(|value| value.parse::<i32>().ok());
    match sqlx::query(
        r#"
        WITH filtered_sessions AS (
          SELECT DISTINCT session_id
          FROM website_event
          WHERE website_id = $1 AND created_at BETWEEN $2 AND $3 AND event_type != 5
            AND (($6 AND tag = 'test') OR (NOT $6 AND COALESCE(tag, '') != 'test'))
        )
        SELECT CASE
                 WHEN d.data_type = 2 THEN replace(d.number_value::text, '.0000', '')
                 WHEN d.data_type = 4 THEN to_char(date_trunc('hour', d.date_value), 'YYYY-MM-DD HH24:MI:SS')
                 ELSE d.string_value
               END AS value,
               count(*)::bigint AS total
        FROM session_data d
        JOIN filtered_sessions f ON f.session_id = d.session_id
        WHERE d.website_id = $1 AND d.data_key = $4 AND ($5::int IS NULL OR d.data_type = $5)
        GROUP BY value
        ORDER BY total DESC
        LIMIT 500
        "#,
    )
    .bind(website_id)
    .bind(start)
    .bind(end)
    .bind(property_name)
    .bind(data_type)
    .bind(is_test_mode(&params))
    .fetch_all(&pool)
    .await
    {
        Ok(rows) => json_response(Value::Array(
            rows.into_iter()
                .map(|row| {
                    json!({
                        "value": row.get::<Option<String>, _>("value"),
                        "total": row.get::<i64, _>("total")
                    })
                })
                .collect(),
        )),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn stub_json(state: &AppState, headers: &HeaderMap, payload: Value) -> Response {
    match authorized_pool(state, headers).await {
        Ok(_) => json_response(payload),
        Err(response) => response,
    }
}

async fn empty_array(State(state): State<AppState>, headers: HeaderMap) -> Response {
    stub_json(&state, &headers, json!([])).await
}

async fn empty_object(State(state): State<AppState>, headers: HeaderMap) -> Response {
    stub_json(&state, &headers, json!({})).await
}

async fn empty_page(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let (page, page_size, _) = page_params(&params);
    stub_json(
        &state,
        &headers,
        json!({ "data": [], "count": 0, "page": page, "pageSize": page_size }),
    )
    .await
}

async fn revenue_stats(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let stats = json!({ "sum": 0, "count": 0, "average": 0, "unique_count": 0, "arpu": 0 });
    stub_json(
        &state,
        &headers,
        json!({ "sum": 0, "count": 0, "average": 0, "unique_count": 0, "arpu": 0, "comparison": stats }),
    )
    .await
}

async fn performance_stats(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let z = json!({ "p50": 0, "p75": 0, "p95": 0 });
    stub_json(
        &state,
        &headers,
        json!({ "lcp": z, "inp": z, "cls": z, "fcp": z, "ttfb": z, "count": 0 }),
    )
    .await
}

async fn empty_error_stats(State(state): State<AppState>, headers: HeaderMap) -> Response {
    stub_json(
        &state,
        &headers,
        json!({ "occurrences": 0, "visits": 0, "issues": 0, "firstSeen": null, "lastSeen": null, "series": [], "canUpdate": false }),
    )
    .await
}
