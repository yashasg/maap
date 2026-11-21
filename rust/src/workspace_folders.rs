use super::{check_bearer_auth, error_response, json_response, AppState};
use axum::{
    extract::{rejection::JsonRejection, Query, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::get,
    Json, Router,
};
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use uuid::Uuid;

const PERSONAL_ASSIGNMENTS_SQL: &str = r#"
    SELECT a.website_id, a.folder_id
    FROM website_folder_assignment a
    JOIN workspace_folder f ON f.folder_id = a.folder_id
    JOIN website w ON w.website_id = a.website_id
    WHERE f.user_id = $1 AND f.team_id IS NULL
      AND w.user_id = $1 AND w.team_id IS NULL
      AND w.deleted_at IS NULL
"#;
const TEAM_ASSIGNMENTS_SQL: &str = r#"
    SELECT a.website_id, a.folder_id
    FROM website_folder_assignment a
    JOIN workspace_folder f ON f.folder_id = a.folder_id
    JOIN website w ON w.website_id = a.website_id
    WHERE f.team_id = $1 AND f.user_id IS NULL
      AND w.team_id = $1 AND w.user_id IS NULL
      AND w.deleted_at IS NULL
"#;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkspaceScope {
    Personal(Uuid),
    Team(Uuid),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkspaceQuery {
    team_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateFolderInput {
    team_id: Option<Uuid>,
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MoveWebsiteInput {
    team_id: Option<Uuid>,
    website_id: Uuid,
    folder_id: Option<Uuid>,
}

pub(super) fn register_routes(router: Router<AppState>) -> Router<AppState> {
    router.route(
        "/api/workspace-folders",
        get(list_folders).post(create_folder).put(move_website),
    )
}

fn database(state: &AppState) -> Result<&PgPool, Response> {
    state
        .db
        .as_ref()
        .ok_or_else(|| error_response(StatusCode::SERVICE_UNAVAILABLE, "Database unavailable"))
}

fn bad_request(message: &str) -> Response {
    error_response(StatusCode::BAD_REQUEST, message)
}

fn unauthorized() -> Response {
    error_response(StatusCode::UNAUTHORIZED, "Unauthorized")
}

fn internal_error(message: &str) -> Response {
    error_response(StatusCode::INTERNAL_SERVER_ERROR, message)
}

fn global_role_allows_write(role: &str) -> bool {
    matches!(role, "admin" | "user")
}

fn team_role_allows_write(role: &str) -> bool {
    matches!(role, "team-owner" | "team-manager" | "team-member")
}

async fn authorize_scope(
    pool: &PgPool,
    headers: &HeaderMap,
    team_id: Option<Uuid>,
    write: bool,
) -> Result<WorkspaceScope, Response> {
    let Some((user_id, user_role)) =
        check_bearer_auth(pool, headers, "/api/workspace-folders").await
    else {
        return Err(unauthorized());
    };

    if write && !global_role_allows_write(&user_role) {
        return Err(unauthorized());
    }

    let Some(team_id) = team_id else {
        return Ok(WorkspaceScope::Personal(user_id));
    };

    let role = sqlx::query_scalar::<_, String>(
        r#"
        SELECT tu.role
        FROM team_user tu
        JOIN team t ON t.team_id = tu.team_id
        WHERE tu.team_id = $1 AND tu.user_id = $2 AND t.deleted_at IS NULL
        "#,
    )
    .bind(team_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(|_| internal_error("Unable to verify workspace access"))?;

    let Some(role) = role else {
        return Err(unauthorized());
    };
    if write && !team_role_allows_write(&role) {
        return Err(unauthorized());
    }

    Ok(WorkspaceScope::Team(team_id))
}

fn parse_body<T: DeserializeOwned>(
    body: Result<Json<Value>, JsonRejection>,
    required_fields: &[&str],
) -> Result<T, Response> {
    let Json(value) = body.map_err(|_| bad_request("Invalid request"))?;
    if required_fields
        .iter()
        .any(|field| value.get(*field).is_none())
    {
        return Err(bad_request("Invalid request"));
    }
    serde_json::from_value(value).map_err(|_| bad_request("Invalid request"))
}

fn validate_name(name: &str) -> Result<&str, Response> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        Err(bad_request("Invalid folder name"))
    } else {
        Ok(name)
    }
}

async fn list_folders(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<WorkspaceQuery>,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let scope = match authorize_scope(pool, &headers, query.team_id, false).await {
        Ok(scope) => scope,
        Err(response) => return response,
    };
    let (folder_sql, assignment_sql, scope_id) = match scope {
        WorkspaceScope::Personal(user_id) => (
            "SELECT folder_id, name FROM workspace_folder \
             WHERE user_id = $1 AND team_id IS NULL ORDER BY name ASC, created_at ASC",
            PERSONAL_ASSIGNMENTS_SQL,
            user_id,
        ),
        WorkspaceScope::Team(team_id) => (
            "SELECT folder_id, name FROM workspace_folder \
             WHERE team_id = $1 AND user_id IS NULL ORDER BY name ASC, created_at ASC",
            TEAM_ASSIGNMENTS_SQL,
            team_id,
        ),
    };
    let folders = match sqlx::query(folder_sql).bind(scope_id).fetch_all(pool).await {
        Ok(rows) => rows
            .iter()
            .map(|row| {
                json!({
                    "id": row.get::<Uuid, _>("folder_id"),
                    "name": row.get::<String, _>("name"),
                })
            })
            .collect::<Vec<_>>(),
        Err(_) => return internal_error("Unable to load workspace folders"),
    };
    let assignments = match sqlx::query(assignment_sql)
        .bind(scope_id)
        .fetch_all(pool)
        .await
    {
        Ok(rows) => rows
            .iter()
            .map(|row| {
                json!({
                    "websiteId": row.get::<Uuid, _>("website_id"),
                    "folderId": row.get::<Uuid, _>("folder_id"),
                })
            })
            .collect::<Vec<_>>(),
        Err(_) => return internal_error("Unable to load workspace assignments"),
    };

    json_response(json!({ "folders": folders, "assignments": assignments }))
}

async fn create_folder(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<Value>, JsonRejection>,
) -> Response {
    let input = match parse_body::<CreateFolderInput>(body, &["name"]) {
        Ok(input) => input,
        Err(response) => return response,
    };
    let name = match validate_name(&input.name) {
        Ok(name) => name,
        Err(response) => return response,
    };
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let scope = match authorize_scope(pool, &headers, input.team_id, true).await {
        Ok(scope) => scope,
        Err(response) => return response,
    };
    let folder_id = Uuid::new_v4();
    let result = match scope {
        WorkspaceScope::Personal(user_id) => {
            sqlx::query(
                "INSERT INTO workspace_folder \
                 (folder_id, name, user_id, team_id, created_at, updated_at) \
                 VALUES ($1, $2, $3, NULL, now(), now())",
            )
            .bind(folder_id)
            .bind(name)
            .bind(user_id)
            .execute(pool)
            .await
        }
        WorkspaceScope::Team(team_id) => {
            sqlx::query(
                "INSERT INTO workspace_folder \
                 (folder_id, name, user_id, team_id, created_at, updated_at) \
                 VALUES ($1, $2, NULL, $3, now(), now())",
            )
            .bind(folder_id)
            .bind(name)
            .bind(team_id)
            .execute(pool)
            .await
        }
    };
    if result.is_err() {
        return internal_error("Unable to create workspace folder");
    }

    json_response(json!({ "id": folder_id, "name": name }))
}

async fn move_website(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<Value>, JsonRejection>,
) -> Response {
    let input = match parse_body::<MoveWebsiteInput>(body, &["websiteId", "folderId"]) {
        Ok(input) => input,
        Err(response) => return response,
    };
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let scope = match authorize_scope(pool, &headers, input.team_id, true).await {
        Ok(scope) => scope,
        Err(response) => return response,
    };
    let mut transaction = match pool.begin().await {
        Ok(transaction) => transaction,
        Err(_) => return internal_error("Unable to move website"),
    };
    let (website_sql, folder_sql, scope_id) = match scope {
        WorkspaceScope::Personal(user_id) => (
            "SELECT EXISTS(SELECT 1 FROM website WHERE website_id = $1 \
             AND user_id = $2 AND team_id IS NULL AND deleted_at IS NULL)",
            "SELECT EXISTS(SELECT 1 FROM workspace_folder WHERE folder_id = $1 \
             AND user_id = $2 AND team_id IS NULL)",
            user_id,
        ),
        WorkspaceScope::Team(team_id) => (
            "SELECT EXISTS(SELECT 1 FROM website WHERE website_id = $1 \
             AND team_id = $2 AND user_id IS NULL AND deleted_at IS NULL)",
            "SELECT EXISTS(SELECT 1 FROM workspace_folder WHERE folder_id = $1 \
             AND team_id = $2 AND user_id IS NULL)",
            team_id,
        ),
    };
    let website_allowed = match sqlx::query_scalar::<_, bool>(website_sql)
        .bind(input.website_id)
        .bind(scope_id)
        .fetch_one(&mut *transaction)
        .await
    {
        Ok(allowed) => allowed,
        Err(_) => return internal_error("Unable to verify website scope"),
    };
    if !website_allowed {
        return unauthorized();
    }

    let result = if let Some(folder_id) = input.folder_id {
        let folder_allowed = match sqlx::query_scalar::<_, bool>(folder_sql)
            .bind(folder_id)
            .bind(scope_id)
            .fetch_one(&mut *transaction)
            .await
        {
            Ok(allowed) => allowed,
            Err(_) => return internal_error("Unable to verify folder scope"),
        };
        if !folder_allowed {
            return unauthorized();
        }
        sqlx::query(
            "INSERT INTO website_folder_assignment \
             (website_id, folder_id, created_at, updated_at) \
             VALUES ($1, $2, now(), now()) \
             ON CONFLICT (website_id) DO UPDATE SET \
             folder_id = EXCLUDED.folder_id, updated_at = now()",
        )
        .bind(input.website_id)
        .bind(folder_id)
        .execute(&mut *transaction)
        .await
    } else {
        sqlx::query("DELETE FROM website_folder_assignment WHERE website_id = $1")
            .bind(input.website_id)
            .execute(&mut *transaction)
            .await
    };
    if result.is_err() || transaction.commit().await.is_err() {
        return internal_error("Unable to move website");
    }

    json_response(json!({ "ok": true }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_folder_names_and_nullable_move_target() {
        assert_eq!(validate_name("  Roadmap  ").unwrap(), "Roadmap");
        assert!(validate_name("   ").is_err());
        assert!(validate_name(&"x".repeat(101)).is_err());

        let root = serde_json::from_value::<MoveWebsiteInput>(json!({
            "websiteId": Uuid::new_v4(),
            "folderId": null
        }));
        assert!(root.is_ok());
    }

    #[test]
    fn write_roles_match_website_organization_permissions() {
        assert!(global_role_allows_write("user"));
        assert!(!global_role_allows_write("view-only"));
        assert!(team_role_allows_write("team-member"));
        assert!(!team_role_allows_write("team-view-only"));
    }

    #[test]
    fn assignment_reads_hide_deleted_and_transferred_websites() {
        for sql in [PERSONAL_ASSIGNMENTS_SQL, TEAM_ASSIGNMENTS_SQL] {
            assert!(sql.contains("w.deleted_at IS NULL"));
            assert!(sql.contains("JOIN workspace_folder"));
        }
        assert!(PERSONAL_ASSIGNMENTS_SQL.contains("w.user_id = $1 AND w.team_id IS NULL"));
        assert!(TEAM_ASSIGNMENTS_SQL.contains("w.team_id = $1 AND w.user_id IS NULL"));
    }

    #[test]
    fn routes_register_without_conflicts() {
        let _ = register_routes(Router::new());
    }
}
