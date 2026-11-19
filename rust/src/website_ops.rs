use crate::{
    access::{website_pool, Access},
    check_bearer_auth, error_response, json_response, metrics, status_response, AppState,
};
use axum::{
    body::to_bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use std::{
    collections::HashMap,
    io::{Cursor, Write},
};
use uuid::Uuid;
use zip::{write::SimpleFileOptions, ZipWriter};

pub(super) fn register_routes(router: Router<AppState>) -> Router<AppState> {
    router
        .route("/api/websites/{website_id}/export", get(export))
        .route("/api/websites/{website_id}/transfer", post(transfer))
}

fn csv_cell(s: &str) -> String {
    let s = if s.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{s}")
    } else {
        s.into()
    };
    if s.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s
    }
}

async fn export(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = website_pool(&state, &headers, id, Access::Read).await {
        return r;
    }
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, kind) in [
        ("events", "event"),
        ("pages", "path"),
        ("referrers", "referrer"),
        ("browsers", "browser"),
        ("os", "os"),
        ("devices", "device"),
        ("countries", "country"),
    ] {
        let mut query = params.clone();
        query.insert("type".into(), kind.into());
        let response = metrics::handle_website_metrics(
            State(state.clone()),
            Path(id),
            Query(query),
            headers.clone(),
        )
        .await;
        if !response.status().is_success() {
            return response;
        }
        let Ok(bytes) = to_bytes(response.into_body(), 4 * 1024 * 1024).await else {
            return status_response(StatusCode::INTERNAL_SERVER_ERROR);
        };
        let Ok(rows) = serde_json::from_slice::<Vec<Value>>(&bytes) else {
            return status_response(StatusCode::INTERNAL_SERVER_ERROR);
        };
        let mut csv = String::new();
        if !rows.is_empty() {
            csv.push_str("x,y");
            for row in rows {
                csv.push_str("\r\n");
                csv.push_str(&csv_cell(row["x"].as_str().unwrap_or("")));
                csv.push(',');
                csv.push_str(&row["y"].to_string());
            }
        }
        if zip
            .start_file(
                format!("{name}.csv"),
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
            )
            .is_err()
            || zip.write_all(csv.as_bytes()).is_err()
        {
            return status_response(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }
    match zip.finish() {
        Ok(file) => json_response(json!({"zip":STANDARD.encode(file.into_inner())})),
        Err(_) => status_response(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Target {
    user_id: Option<Uuid>,
    team_id: Option<Uuid>,
}

async fn transfer(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let pool = match website_pool(&state, &headers, id, Access::Write).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some((viewer, role)) = check_bearer_auth(&pool, &headers, "/api/websites").await else {
        return status_response(StatusCode::UNAUTHORIZED);
    };
    let Ok(target) = serde_json::from_value::<Target>(body) else {
        return status_response(StatusCode::BAD_REQUEST);
    };
    let (to_user, to_team) = match (target.user_id, target.team_id) {
        (Some(user), _) => (Some(user), None),
        (None, Some(team)) => (None, Some(team)),
        _ => return status_response(StatusCode::BAD_REQUEST),
    };
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    let current = match sqlx::query(
        "SELECT user_id,team_id FROM website WHERE website_id=$1 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return status_response(StatusCode::NOT_FOUND),
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    let (user, team): (Option<Uuid>, Option<Uuid>) =
        (current.get("user_id"), current.get("team_id"));
    let allowed=sqlx::query_scalar::<_,bool>(r#"SELECT CASE WHEN $1::uuid IS NOT NULL THEN
      EXISTS (SELECT 1 FROM "user" WHERE user_id=$1 AND deleted_at IS NULL)
      AND ($6 OR ($1=$5 AND EXISTS(SELECT 1 FROM team_user WHERE team_id=$4 AND user_id=$5 AND role='team-owner')))
      ELSE EXISTS (SELECT 1 FROM team WHERE team_id=$2 AND deleted_at IS NULL)
      AND ($6 OR ($3=$5 AND EXISTS(SELECT 1 FROM team_user WHERE team_id=$2 AND user_id=$5 AND role IN ('team-owner','team-manager')))) END"#)
        .bind(to_user).bind(to_team).bind(user).bind(team).bind(viewer).bind(role=="admin").fetch_one(&mut *tx).await;
    match allowed {
        Ok(true) => (),
        Ok(false) => return status_response(StatusCode::UNAUTHORIZED),
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
    let result:Result<String,sqlx::Error>=async{
        sqlx::query("UPDATE board SET user_id=$1,team_id=$2,updated_at=now() WHERE type='website' AND parameters->>'websiteId'=$3 AND user_id IS NOT DISTINCT FROM $4 AND team_id IS NOT DISTINCT FROM $5").bind(to_user).bind(to_team).bind(id.to_string()).bind(user).bind(team).execute(&mut *tx).await?;
        let row=sqlx::query_scalar("WITH updated AS (UPDATE website SET user_id=$1,team_id=$2,updated_at=now() WHERE website_id=$3 RETURNING *) SELECT jsonb_build_object('id',website_id,'userId',user_id,'teamId',team_id,'domain',domain,'name',name,'createdAt',created_at,'createdBy',created_by,'updatedAt',updated_at,'resetAt',reset_at,'deletedAt',deleted_at,'recorderEnabled',recorder_enabled,'replayConfig',replay_config)::text FROM updated").bind(to_user).bind(to_team).bind(id).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok(row)
    }.await;
    match result {
        Ok(s) => match serde_json::from_str(&s) {
            Ok(v) => json_response(v),
            Err(_) => status_response(StatusCode::INTERNAL_SERVER_ERROR),
        },
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}
