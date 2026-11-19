use crate::{
    access::{website_pool, Access},
    error_response, json_response, status_response, AppState,
};
use axum::{
    extract::{MatchedPath, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use uuid::Uuid;

pub(super) fn register_routes(router: Router<AppState>) -> Router<AppState> {
    let mut router = router.route(
        "/api/websites/{website_id}/errors/{issue_id}",
        get(read).patch(update),
    );
    for path in [
        "/api/websites/{website_id}/errors",
        "/api/websites/{website_id}/errors/stats",
        "/api/websites/{website_id}/errors/values",
        "/api/websites/{website_id}/errors/{issue_id}/values",
        "/api/websites/{website_id}/errors/{issue_id}/events",
        "/api/websites/{website_id}/errors/{issue_id}/events/{event_id}",
    ] {
        router = router.route(path, get(read));
    }
    router
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Filters {
    start_at: Option<i64>,
    end_at: Option<i64>,
    status: Option<String>,
    release: Option<String>,
    environment: Option<String>,
    browser: Option<String>,
    url_path: Option<String>,
    search: Option<String>,
    page: Option<i64>,
    page_size: Option<i64>,
    #[serde(rename = "type")]
    kind: Option<String>,
    value: Option<String>,
}

fn valid_status(s: &str) -> bool {
    matches!(s, "ignored" | "resolved" | "unresolved")
}
const EVENTS: &str = r#"id, "createdAt", name, message, handled, release, environment, browser, os, device, "urlPath","sessionId","visitId""#;
const ISSUE: &str = r#"i.issue_id AS id,i.website_id AS "websiteId",i.fingerprint,i.grouping_version AS "groupingVersion",i.title,i.status,i.resolved_at AS "resolvedAt",i.created_at AS "createdAt""#;
const STATS: &str = r#"SELECT count(*) AS occurrences,count(distinct "visitId") AS visits,count(distinct issue_id) AS issues,min("createdAt") AS "firstSeen",max("createdAt") AS "lastSeen" FROM f"#;

async fn read(
    State(state): State<AppState>,
    matched: MatchedPath,
    Path(ids): Path<HashMap<String, Uuid>>,
    Query(q): Query<Filters>,
    headers: HeaderMap,
) -> Response {
    let website = ids["website_id"];
    let issue = ids.get("issue_id");
    let event = ids.get("event_id");
    let pool = match website_pool(&state, &headers, website, Access::Read).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let (page, size) = (q.page.unwrap_or(1), q.page_size.unwrap_or(25));
    let (start, end) = if event.is_some() {
        (0, 4102444800000)
    } else if let (Some(a), Some(b)) = (q.start_at, q.end_at) {
        (a, b)
    } else {
        return error_response(StatusCode::BAD_REQUEST, "Missing date range");
    };
    if start < 0
        || end < start
        || end > 4102444800000
        || !(1..=10000).contains(&page)
        || !(1..=100).contains(&size)
        || q.status.as_deref().is_some_and(|s| !valid_status(s))
        || [
            (&q.release, 100),
            (&q.environment, 50),
            (&q.browser, 50),
            (&q.url_path, 500),
            (&q.search, 200),
            (&q.value, 500),
        ]
        .iter()
        .any(|(s, max)| s.as_ref().is_some_and(|s| s.chars().count() > *max))
    {
        return error_response(StatusCode::BAD_REQUEST, "Invalid error query");
    }
    if let Some(id) = issue {
        match sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM error_issue WHERE website_id=$1 AND issue_id=$2)",
        )
        .bind(website)
        .bind(id)
        .fetch_one(&pool)
        .await
        {
            Ok(true) => (),
            Ok(false) => return status_response(StatusCode::NOT_FOUND),
            Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        }
    }
    let path = matched.as_str();
    let dimension = if path.ends_with("/values") {
        match q.kind.as_deref() {
            Some("release") => "release",
            Some("environment") => "environment",
            Some("browser") => "browser",
            Some("urlPath") => "\"urlPath\"",
            _ => return error_response(StatusCode::BAD_REQUEST, "Invalid error dimension"),
        }
    } else {
        ""
    };
    // Keep pagination and totals on the same filtered snapshot; all request data is bound.
    let base = r#"WITH opts AS (
        SELECT $1::uuid w,$2::uuid issue,$3::uuid event,to_timestamp($4::bigint/1000.0) a,to_timestamp($5::bigint/1000.0) b,
               $6::text status,$7::text release,$8::text env,$9::text browser,$10::text path,$11::text search,$12::text val,$13::bigint pg,$14::bigint size
        ), f AS (
        SELECT e.event_id id,e.issue_id,e.created_at AS "createdAt",e.visit_id AS "visitId",e.session_id AS "sessionId",
               e.name,e.message,e.handled,e.release,e.environment,e.browser,e.os,e.device,e.url_path AS "urlPath",e.frames,e.tags,e.stack
        FROM opts o JOIN error_event e ON e.website_id=o.w JOIN website w ON w.website_id=e.website_id
        JOIN error_issue i ON i.website_id=e.website_id AND i.issue_id=e.issue_id
        WHERE (o.issue IS NULL OR e.issue_id=o.issue) AND (o.event IS NULL OR e.event_id=o.event)
        AND e.created_at>COALESCE(w.reset_at,'epoch'::timestamptz) AND e.created_at BETWEEN o.a AND o.b
        AND (o.status IS NULL OR o.status=i.status) AND (o.release IS NULL OR o.release=e.release)
        AND (o.env IS NULL OR o.env=e.environment) AND (o.browser IS NULL OR o.browser=e.browser)
        AND (o.path IS NULL OR o.path=e.url_path) AND (o.search IS NULL OR position(lower(o.search) IN lower(e.name||': '||e.message))>0)
        )"#;
    let paged = |query: String, order: &str| {
        format!(
            r#", rows AS ({query}), subset AS (SELECT * FROM rows ORDER BY {order} LIMIT (SELECT size FROM opts) OFFSET (SELECT (pg-1)*size FROM opts))
        SELECT jsonb_build_object('count',(SELECT count(*) FROM rows),'page',pg,'pageSize',size,'data',COALESCE((SELECT jsonb_agg(to_jsonb(subset) ORDER BY {order}) FROM subset),'[]'::jsonb))::text FROM opts"#
        )
    };
    let tail = if !dimension.is_empty() {
        format!(
            r#" SELECT COALESCE(jsonb_agg(to_jsonb(sub) ORDER BY count DESC,value),'[]'::jsonb)::text FROM (SELECT {dimension} value,count(*) count FROM f WHERE {dimension}<>'' AND ((SELECT val FROM opts) IS NULL OR position(lower((SELECT val FROM opts)) IN lower({dimension}))>0) GROUP BY 1 ORDER BY 2 DESC,1 LIMIT 100) sub"#
        )
    } else if event.is_some() {
        format!(
            r#" SELECT to_jsonb(sub)::text FROM (SELECT {EVENTS},frames,tags,stack,(SELECT r.visit_id FROM session_replay r WHERE r.website_id=(SELECT w FROM opts) AND r.session_id=f."sessionId" AND r.visit_id=f."visitId" AND f."createdAt" BETWEEN r.started_at AND r.ended_at LIMIT 1) AS "replayId" FROM f) sub"#
        )
    } else if path.ends_with("/events") {
        paged(format!("SELECT {EVENTS} FROM f"), r#""createdAt" DESC,id"#)
    } else if issue.is_some() || path.ends_with("/stats") {
        let item = if issue.is_some() {
            format!(
                r#"(SELECT to_jsonb(sub) FROM (SELECT {ISSUE} FROM error_issue i WHERE i.website_id=(SELECT w FROM opts) AND i.issue_id=(SELECT issue FROM opts)) sub) || "#
            )
        } else {
            String::new()
        };
        format!(
            r#", totals AS ({STATS}), series AS (SELECT to_char("createdAt" AT TIME ZONE 'UTC','YYYY-MM-DD') date,count(*) occurrences FROM f GROUP BY 1)
            SELECT ({item}to_jsonb(totals) || jsonb_build_object('series',COALESCE((SELECT jsonb_agg(to_jsonb(series) ORDER BY date) FROM series),'[]'::jsonb)))::text FROM totals"#
        )
    } else {
        paged(
            format!(
                r#"SELECT {ISSUE},g.occurrences,g.visits,g."firstSeen",g."lastSeen" FROM error_issue i JOIN
            (SELECT issue_id,count(*) occurrences,count(DISTINCT "visitId") visits,min("createdAt") AS "firstSeen",max("createdAt") AS "lastSeen" FROM f GROUP BY issue_id) g USING(issue_id) WHERE i.website_id=(SELECT w FROM opts)"#
            ),
            r#""lastSeen" DESC,id"#,
        )
    };
    let filter = |k: &str, v: Option<String>| {
        if dimension.trim_matches('"') == k {
            None
        } else {
            v.filter(|s| !s.is_empty())
        }
    };
    let sql = format!("{base}{tail}");
    let data = sqlx::query_scalar::<_, String>(&sql)
        .bind(website)
        .bind(issue)
        .bind(event)
        .bind(start)
        .bind(end)
        .bind(q.status)
        .bind(filter("release", q.release))
        .bind(filter("environment", q.environment))
        .bind(filter("browser", q.browser))
        .bind(filter("urlPath", q.url_path))
        .bind(q.search)
        .bind(q.value)
        .bind(page)
        .bind(size)
        .fetch_optional(&pool)
        .await;
    match data {
        Ok(Some(s)) => match serde_json::from_str::<Value>(&s) {
            Ok(mut v) => {
                if path.ends_with("/{issue_id}") {
                    v["canUpdate"] = json!(website_pool(&state, &headers, website, Access::Write)
                        .await
                        .is_ok());
                }
                json_response(v)
            }
            Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        },
        Ok(None) => status_response(StatusCode::NOT_FOUND),
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn update(
    State(state): State<AppState>,
    Path((website, issue)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let pool = match website_pool(&state, &headers, website, Access::Write).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some(status) = body["status"].as_str().filter(|s| valid_status(s)) else {
        return error_response(StatusCode::BAD_REQUEST, "Invalid issue status");
    };
    match sqlx::query("UPDATE error_issue SET status=$3,resolved_at=CASE WHEN $3='resolved' THEN now() END WHERE website_id=$1 AND issue_id=$2")
        .bind(website).bind(issue).bind(status).execute(&pool).await {
        Ok(r) if r.rows_affected()==1=>json_response(json!({"ok":true})), Ok(_)=>status_response(StatusCode::NOT_FOUND),Err(e)=>error_response(StatusCode::INTERNAL_SERVER_ERROR,e.to_string())
    }
}
