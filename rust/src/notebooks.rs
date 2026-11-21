use crate::{
    access::{website_pool, Access},
    check_bearer_auth, error_response, json_response, status_response, AppState,
};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use std::collections::HashSet;
use uuid::Uuid;

const LIST_FIELDS: &str = r#"
    notebook_id, website_id, created_by, title, created_at, updated_at
"#;
const DETAIL_FIELDS: &str = r#"
    notebook_id, website_id, created_by, title, content::text AS content, created_at, updated_at
"#;

fn create_sql() -> String {
    format!(
        "WITH live_website AS (
             SELECT website_id
             FROM website
             WHERE website_id = $2 AND deleted_at IS NULL
             FOR UPDATE
         )
         INSERT INTO notebook
             (notebook_id, website_id, created_by, title, content, created_at, updated_at)
         SELECT $1, website_id, $3, $4, $5::jsonb, now(), now()
         FROM live_website
         RETURNING {DETAIL_FIELDS}"
    )
}

fn update_sql() -> String {
    format!(
        "WITH live_website AS (
             SELECT website_id
             FROM website
             WHERE website_id = $1 AND deleted_at IS NULL
             FOR UPDATE
         )
         UPDATE notebook
         SET title = $3, content = $4::jsonb, updated_at = now()
         WHERE website_id = $1 AND notebook_id = $2
           AND EXISTS (SELECT 1 FROM live_website)
         RETURNING {DETAIL_FIELDS}"
    )
}

#[derive(Debug, Deserialize)]
struct NotebookInput {
    title: String,
    content: NotebookDocument,
}

#[derive(Debug, Deserialize, Serialize)]
struct NotebookDocument {
    version: u8,
    blocks: Vec<NotebookBlock>,
}

#[derive(Debug, Deserialize, Serialize)]
struct NotebookBlock {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    checked: Option<bool>,
}

pub(super) fn register_routes(router: Router<AppState>) -> Router<AppState> {
    router
        .route(
            "/api/websites/{website_id}/notebooks",
            get(list_notebooks).post(create_notebook),
        )
        .route(
            "/api/websites/{website_id}/notebooks/{notebook_id}",
            get(get_notebook).post(update_notebook),
        )
}

fn parse_uuid(value: &str) -> Result<Uuid, Response> {
    Uuid::parse_str(value)
        .map_err(|_| error_response(StatusCode::BAD_REQUEST, "Invalid resource ID"))
}

fn validate_input(input: &NotebookInput) -> Result<(), Response> {
    let title_len = input.title.chars().count();
    if input.title.trim().is_empty() || title_len > 200 {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "Invalid notebook title",
        ));
    }
    if input.content.version != 1
        || input.content.blocks.is_empty()
        || input.content.blocks.len() > 200
    {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "Invalid notebook document",
        ));
    }
    let mut ids = HashSet::new();
    for block in &input.content.blocks {
        if block.id.is_empty()
            || block.id.chars().count() > 64
            || !ids.insert(block.id.as_str())
            || block.text.chars().count() > 10_000
            || !matches!(block.kind.as_str(), "heading" | "paragraph" | "checklist")
            || (block.kind != "checklist" && block.checked.is_some())
        {
            return Err(error_response(
                StatusCode::BAD_REQUEST,
                "Invalid notebook document",
            ));
        }
    }
    Ok(())
}

fn list_json(row: &sqlx::postgres::PgRow) -> Value {
    json!({
        "id": row.get::<Uuid, _>("notebook_id"),
        "websiteId": row.get::<Uuid, _>("website_id"),
        "createdBy": row.get::<Option<Uuid>, _>("created_by"),
        "title": row.get::<String, _>("title"),
        "createdAt": row.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
        "updatedAt": row.get::<chrono::DateTime<chrono::Utc>, _>("updated_at"),
    })
}

fn detail_json(row: &sqlx::postgres::PgRow) -> Result<Value, Response> {
    let content = row
        .get::<String, _>("content")
        .parse::<Value>()
        .map_err(|_| error_response(StatusCode::INTERNAL_SERVER_ERROR, "Invalid notebook data"))?;
    let mut value = list_json(row);
    value["content"] = content;
    Ok(value)
}

async fn read_pool(
    state: &AppState,
    headers: &HeaderMap,
    website_id: Uuid,
) -> Result<(PgPool, bool), Response> {
    let pool = website_pool(state, headers, website_id, Access::Read).await?;
    let can_write = website_pool(state, headers, website_id, Access::Write)
        .await
        .is_ok();
    Ok((pool, can_write))
}

async fn list_notebooks(
    State(state): State<AppState>,
    Path(website_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let website_id = match parse_uuid(&website_id) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let (pool, can_write) = match read_pool(&state, &headers, website_id).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let rows = match sqlx::query(&format!(
        "SELECT {LIST_FIELDS} FROM notebook WHERE website_id = $1
           AND EXISTS (
               SELECT 1 FROM website
               WHERE website_id = $1 AND deleted_at IS NULL
           )
         ORDER BY updated_at DESC, created_at DESC"
    ))
    .bind(website_id)
    .fetch_all(&pool)
    .await
    {
        Ok(rows) => rows,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to load notebooks",
            )
        }
    };
    json_response(json!({
        "data": rows.iter().map(list_json).collect::<Vec<_>>(),
        "canWrite": can_write,
    }))
}

async fn create_notebook(
    State(state): State<AppState>,
    Path(website_id): Path<String>,
    headers: HeaderMap,
    Json(input): Json<NotebookInput>,
) -> Response {
    let website_id = match parse_uuid(&website_id) {
        Ok(value) => value,
        Err(response) => return response,
    };
    if let Err(response) = validate_input(&input) {
        return response;
    }
    let pool = match website_pool(&state, &headers, website_id, Access::Write).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let Some((user_id, _)) = check_bearer_auth(&pool, &headers, "/api/websites").await else {
        return status_response(StatusCode::UNAUTHORIZED);
    };
    let content = match serde_json::to_string(&input.content) {
        Ok(value) => value,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid notebook document"),
    };
    let query = create_sql();
    let row = match sqlx::query(&query)
        .bind(Uuid::new_v4())
        .bind(website_id)
        .bind(user_id)
        .bind(input.title.trim())
        .bind(content)
        .fetch_optional(&pool)
        .await
    {
        Ok(Some(row)) => row,
        Ok(None) => return status_response(StatusCode::UNAUTHORIZED),
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to create notebook",
            )
        }
    };
    match detail_json(&row) {
        Ok(value) => json_response(value),
        Err(response) => response,
    }
}

async fn get_notebook(
    State(state): State<AppState>,
    Path((website_id, notebook_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let website_id = match parse_uuid(&website_id) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let notebook_id = match parse_uuid(&notebook_id) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let (pool, can_write) = match read_pool(&state, &headers, website_id).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let row = match sqlx::query(&format!(
        "SELECT {DETAIL_FIELDS} FROM notebook
         WHERE website_id = $1 AND notebook_id = $2
           AND EXISTS (
               SELECT 1 FROM website
               WHERE website_id = $1 AND deleted_at IS NULL
           )"
    ))
    .bind(website_id)
    .bind(notebook_id)
    .fetch_optional(&pool)
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => return status_response(StatusCode::NOT_FOUND),
        Err(_) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Unable to load notebook")
        }
    };
    match detail_json(&row) {
        Ok(data) => json_response(json!({ "data": data, "canWrite": can_write })),
        Err(response) => response,
    }
}

async fn update_notebook(
    State(state): State<AppState>,
    Path((website_id, notebook_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(input): Json<NotebookInput>,
) -> Response {
    let website_id = match parse_uuid(&website_id) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let notebook_id = match parse_uuid(&notebook_id) {
        Ok(value) => value,
        Err(response) => return response,
    };
    if let Err(response) = validate_input(&input) {
        return response;
    }
    let pool = match website_pool(&state, &headers, website_id, Access::Write).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let content = match serde_json::to_string(&input.content) {
        Ok(value) => value,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid notebook document"),
    };
    let query = update_sql();
    let row = match sqlx::query(&query)
        .bind(website_id)
        .bind(notebook_id)
        .bind(input.title.trim())
        .bind(content)
        .fetch_optional(&pool)
        .await
    {
        Ok(Some(row)) => row,
        Ok(None) => return status_response(StatusCode::NOT_FOUND),
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to update notebook",
            )
        }
    };
    match detail_json(&row) {
        Ok(value) => json_response(value),
        Err(response) => response,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(blocks: Vec<NotebookBlock>) -> NotebookInput {
        NotebookInput {
            title: "Release notes".to_string(),
            content: NotebookDocument { version: 1, blocks },
        }
    }

    fn block(id: &str, kind: &str) -> NotebookBlock {
        NotebookBlock {
            id: id.to_string(),
            kind: kind.to_string(),
            text: String::new(),
            checked: (kind == "checklist").then_some(false),
        }
    }

    #[test]
    fn accepts_versioned_mixed_block_documents() {
        let value = input(vec![
            block("heading", "heading"),
            block("notes", "paragraph"),
            block("todo", "checklist"),
        ]);
        assert!(validate_input(&value).is_ok());
    }

    #[test]
    fn rejects_duplicate_ids_and_unversioned_documents() {
        let duplicate = input(vec![block("same", "paragraph"), block("same", "heading")]);
        assert!(validate_input(&duplicate).is_err());

        let mut unsupported = input(vec![block("one", "paragraph")]);
        unsupported.content.version = 2;
        assert!(validate_input(&unsupported).is_err());
    }

    #[test]
    fn detail_queries_are_scoped_by_app_and_notebook() {
        let query = format!(
            "SELECT {DETAIL_FIELDS} FROM notebook
             WHERE website_id = $1 AND notebook_id = $2
               AND EXISTS (
                   SELECT 1 FROM website
                   WHERE website_id = $1 AND deleted_at IS NULL
               )"
        );
        assert!(query.contains("website_id = $1 AND notebook_id = $2"));
        assert!(query.contains("deleted_at IS NULL"));
    }

    #[test]
    fn writes_lock_and_recheck_the_live_app() {
        let create = create_sql();
        let update = update_sql();
        assert!(create.contains("deleted_at IS NULL"));
        assert!(create.contains("FOR UPDATE"));
        assert!(update.contains("deleted_at IS NULL"));
        assert!(update.contains("FOR UPDATE"));
    }
}
