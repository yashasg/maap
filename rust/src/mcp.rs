use super::*;
use chrono::{NaiveDate, SecondsFormat};
mod tools;

const VERSION: &str = "2026-07-28";

fn tools() -> Vec<Value> {
    let website = json!({"type": "string", "format": "uuid"});
    let tool = |name, description, properties, required| {
        json!({
            "name": name, "description": description,
            "inputSchema": {"type":"object","properties":properties,"required":required,"additionalProperties":false},
            "annotations": {"readOnlyHint":true, "destructiveHint":false}
        })
    };
    let mut list = vec![
        tool(
            "list_websites",
            "List accessible websites.",
            json!({
                "search":{"type":"string"}, "page":{"type":"integer","minimum":1},
                "pageSize":{"type":"integer","minimum":1,"maximum":100}
            }),
            json!([]),
        ),
        tool(
            "get_website_stats",
            "Traffic totals for a website and the comparison period.",
            json!({
                "websiteId":website, "startAt":{"type":"string"}, "endAt":{"type":"string"},
                "compare":{"enum":["prev","yoy"]}, "timezone":{"type":"string"}, "filters":tools::stats_filters_schema()
            }),
            json!(["websiteId", "startAt"]),
        ),
    ];
    list.extend(tools::catalog());
    list
}

fn rpc(id: &Value, result: Value) -> Response {
    json_response(json!({"jsonrpc":"2.0","id":id,"result":result}))
}

fn rpc_error(id: &Value, code: i32, message: &str) -> Response {
    json_response(json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}}))
}

fn tool_result(id: &Value, value: Value, failed: bool) -> Response {
    rpc(
        id,
        json!({
            "content":[{"type":"text","text":value.to_string()}],
            "structuredContent":value, "isError":failed
        }),
    )
}

fn parse_date(value: &Value) -> Option<DateTime<Utc>> {
    let text = value.as_str()?;
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|d| d.to_utc())
        .or_else(|| {
            Some(
                NaiveDate::parse_from_str(text, "%Y-%m-%d")
                    .ok()?
                    .and_hms_opt(0, 0, 0)?
                    .and_utc(),
            )
        })
}

fn summary(row: &Value) -> Option<Value> {
    let (pageviews, visitors, visits, bounces, time) = (
        row["pageviews"].as_u64()?,
        row["visitors"].as_u64()?,
        row["visits"].as_u64()?,
        row["bounces"].as_u64()?,
        row["totaltime"].as_f64()?,
    );
    let per_visit = |n: f64| if visits == 0 { 0.0 } else { n / visits as f64 };
    Some(json!({
        "pageviews":pageviews,"visitors":visitors,"visits":visits,"bounces":bounces,
        "bounceRate": (per_visit(bounces.min(visits) as f64)*1000.0).round()/10.0,
        "averageVisitDurationSeconds":per_visit(time).round(),
        "viewsPerVisit":(per_visit(pageviews as f64)*100.0).round()/100.0
    }))
}

// Use the REST handlers themselves so the tool never gets a separate, less restricted database path.
async fn rest_body(res: Response) -> Result<Value, Value> {
    let status = res.status();
    if !status.is_success() {
        let code = match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => "access_denied",
            StatusCode::BAD_REQUEST => "invalid_input",
            StatusCode::NOT_FOUND => "not_found",
            _ => "api_error",
        };
        return Err(json!({"code":code,"status":status.as_u16(),"message":"API request failed"}));
    }
    let invalid = || json!({"code":"api_error","message":"Invalid API response"});
    let bytes = to_bytes(res.into_body(), 1024 * 1024)
        .await
        .map_err(|_| invalid())?;
    serde_json::from_slice(&bytes).map_err(|_| invalid())
}

async fn call(
    state: AppState,
    headers: HeaderMap,
    name: &str,
    args: &Map<String, Value>,
) -> Result<Value, Value> {
    let invalid = || json!({"code":"invalid_input","message":"Invalid tool arguments"});
    let mut query = HashMap::new();
    if name == "list_websites" {
        for (key, value) in args {
            let value = match key.as_str() {
                "page"
                    if value
                        .as_u64()
                        .is_some_and(|n| n > 0 && n <= i32::MAX as u64) =>
                {
                    value.to_string()
                }
                "pageSize" if value.as_u64().is_some_and(|n| (1..=100).contains(&n)) => {
                    value.to_string()
                }
                "search" => value.as_str().ok_or_else(invalid)?.to_owned(),
                _ => return Err(invalid()),
            };
            query.insert(key.clone(), value);
        }
        query.insert("includeTeams".into(), "true".into());
        let page =
            rest_body(dashboard_api::handle_websites(State(state), Query(query), headers).await)
                .await?;
        let websites: Vec<Value> = page["data"].as_array().ok_or_else(invalid)?.iter().map(|w| json!({
            "id":w["id"], "name":w["name"], "domain":w["domain"], "teamId":w["teamId"], "createdAt":w["createdAt"]
        })).collect();
        let (p, size, count) = (
            page["page"].as_u64().ok_or_else(invalid)?,
            page["pageSize"].as_u64().ok_or_else(invalid)?,
            page["count"].as_u64().ok_or_else(invalid)?,
        );
        return Ok(
            json!({"websites":websites,"page":p,"pageSize":size,"count":count,"hasMore":p.saturating_mul(size)<count}),
        );
    }
    if args.keys().any(|k| {
        !matches!(
            k.as_str(),
            "websiteId" | "startAt" | "endAt" | "compare" | "timezone" | "filters"
        )
    }) {
        return Err(invalid());
    }
    query.extend(tools::stats_filters(args)?);
    let wid = args
        .get("websiteId")
        .and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(invalid)?;
    let start = args
        .get("startAt")
        .and_then(parse_date)
        .ok_or_else(invalid)?;
    let end = match args.get("endAt") {
        Some(v) => parse_date(v).ok_or_else(invalid)?,
        None => Utc::now(),
    };
    if end <= start {
        return Err(invalid());
    }
    query.insert("startAt".into(), start.timestamp_millis().to_string());
    query.insert("endAt".into(), end.timestamp_millis().to_string());
    let compare = match args.get("compare").map(Value::as_str) {
        None | Some(Some("prev")) => "prev",
        Some(Some("yoy")) => "yoy",
        _ => return Err(invalid()),
    };
    query.insert("compare".into(), compare.into());
    if let Some(tz) = args.get("timezone") {
        query.insert(
            "timezone".into(),
            tz.as_str().ok_or_else(invalid)?.to_owned(),
        );
    }
    let stats = rest_body(
        metrics::handle_website_stats(State(state), Path(wid), Query(query), headers).await,
    )
    .await?;
    Ok(json!({
        "websiteId":wid, "range":{"startAt":start.to_rfc3339_opts(SecondsFormat::Millis,true),"endAt":end.to_rfc3339_opts(SecondsFormat::Millis,true)},
        "current":summary(&stats).ok_or_else(invalid)?, "previous":summary(&stats["comparison"]).ok_or_else(invalid)?, "compare":compare,
    }))
}

pub(super) async fn handle(state: AppState, api: Router, req: Request) -> Response {
    if env::var("MCP_ENABLED").as_deref() != Ok("1") {
        return status_response(StatusCode::NOT_FOUND);
    }
    if state.node_upstream.is_some() {
        return fallback_proxy(State(state), req).await;
    }
    if req.method() == axum::http::Method::OPTIONS {
        return (
            StatusCode::NO_CONTENT,
            [
                ("access-control-allow-origin", "*"),
                ("access-control-allow-methods", "GET, POST, DELETE, OPTIONS"),
                ("access-control-allow-headers", "*"),
                ("access-control-max-age", "86400"),
            ],
        )
            .into_response();
    }
    let headers = req.headers().clone();
    let api_key = auth::bearer_token(&headers).is_some_and(|t| t.starts_with("umami_"));
    let Some(pool) = &state.db else {
        return status_response(StatusCode::SERVICE_UNAVAILABLE);
    };
    if !api_key || check_bearer_auth(pool, &headers, "/mcp").await.is_none() {
        return with_rust_header(
            (
                StatusCode::UNAUTHORIZED,
                [
                    ("www-authenticate", r#"Bearer realm="Umami MCP""#),
                    ("cache-control", "no-store"),
                ],
                Json(
                    json!({"error":"invalid_token","error_description":"Valid API key required."}),
                ),
            )
                .into_response(),
        );
    }
    if req.method() != axum::http::Method::POST {
        return status_response(StatusCode::METHOD_NOT_ALLOWED);
    }
    let body = match to_bytes(req.into_body(), 1024 * 1024).await {
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(v) => v,
            Err(_) => return rpc_error(&Value::Null, -32700, "Parse error"),
        },
        Err(_) => return status_response(StatusCode::PAYLOAD_TOO_LARGE),
    };
    let id = &body["id"];
    if body["jsonrpc"] != "2.0" {
        return rpc_error(id, -32600, "Invalid request");
    }
    match body["method"].as_str() {
        Some("initialize") => {
            let version = match body["params"]["protocolVersion"].as_str() {
                Some("2025-11-25") => "2025-11-25",
                _ => VERSION,
            };
            rpc(
                id,
                json!({"protocolVersion": version,"capabilities":{"tools":{}},"serverInfo":{"name":"umami","version":"0.1.0"}}),
            )
        }
        Some("notifications/initialized") if id.is_null() => status_response(StatusCode::ACCEPTED),
        Some("ping") => rpc(id, json!({})),
        Some("tools/list") => rpc(id, json!({"tools":tools()})),
        Some("tools/call") => {
            let Some(name) = body["params"]["name"].as_str() else {
                return rpc_error(id, -32602, "Missing tool name");
            };
            let empty = Map::new();
            let args = match body["params"].get("arguments") {
                None => &empty,
                Some(Value::Object(args)) => args,
                _ => return rpc_error(id, -32602, "Invalid arguments"),
            };
            let result = if matches!(name, "list_websites" | "get_website_stats") {
                call(state, headers, name, args).await
            } else {
                tools::invoke(&api, &headers, name, args).await
            };
            match result {
                Ok(v) => tool_result(id, v, false),
                Err(e) if e["code"] == "unknown_tool" => rpc_error(id, -32602, "Unknown tool"),
                Err(e) => tool_result(id, json!({"error":e}), true),
            }
        }
        _ => rpc_error(id, -32601, "Method not found"),
    }
}
