use super::{error_response, json_response, secret_hash, AppState};
use crate::access::{board_share_scope, entity_pool, Access};
use axum::{
    extract::{rejection::JsonRejection, Path, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use rand::{distributions::Alphanumeric, Rng};
use serde_json::{json, Map, Value};
use sqlx::{postgres::PgRow, PgPool, Row};
use uuid::Uuid;

const WEBSITE_SHARE: i32 = 1;
const LINK_SHARE: i32 = 2;
const PIXEL_SHARE: i32 = 3;
const BOARD_SHARE: i32 = 4;

struct CreateShare {
    entity_id: Uuid,
    share_type: i32,
    name: String,
    slug: Option<String>,
    parameters: Map<String, Value>,
}

struct UpdateShare {
    name: String,
    slug: String,
    parameters: Map<String, Value>,
}

pub(super) fn register_routes(router: Router<AppState>) -> Router<AppState> {
    router
        .route("/api/share", post(create_share))
        .route("/api/share/{slug}", get(resolve_share))
        .route(
            "/api/share/id/{share_id}",
            get(get_share).post(update_share).delete(delete_share),
        )
}

async fn resolve_share(State(state): State<AppState>, Path(slug): Path<String>) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let row = match sqlx::query(
        r#"
        SELECT share_id, entity_id, share_type, parameters::text AS parameters
        FROM share
        WHERE slug = $1
        "#,
    )
    .bind(slug)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => return not_found(),
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to load share: {error}"),
            )
        }
    };
    let share_id: Uuid = row.get("share_id");
    let entity_id: Uuid = row.get("entity_id");
    let share_type: i32 = row.get("share_type");
    let parameters = json_object(row.get::<String, _>("parameters"));
    let mut data = Map::from_iter([
        ("shareId".to_string(), json!(share_id)),
        ("shareType".to_string(), json!(share_type)),
        ("parameters".to_string(), Value::Object(parameters)),
    ]);

    match share_type {
        WEBSITE_SHARE => {
            if !match entity_exists(pool, "website", "website_id", entity_id).await {
                Ok(exists) => exists,
                Err(response) => return response,
            } {
                return not_found();
            }
            data.insert("websiteId".to_string(), json!(entity_id));
        }
        LINK_SHARE => {
            if !match entity_exists(pool, "link", "link_id", entity_id).await {
                Ok(exists) => exists,
                Err(response) => return response,
            } {
                return not_found();
            }
            data.insert("websiteId".to_string(), json!(entity_id));
            data.insert("linkId".to_string(), json!(entity_id));
        }
        PIXEL_SHARE => {
            if !match entity_exists(pool, "pixel", "pixel_id", entity_id).await {
                Ok(exists) => exists,
                Err(response) => return response,
            } {
                return not_found();
            }
            data.insert("websiteId".to_string(), json!(entity_id));
            data.insert("pixelId".to_string(), json!(entity_id));
        }
        BOARD_SHARE => {
            let scope = match board_share_scope(pool, entity_id).await {
                Ok(Some(scope)) => scope,
                Ok(None) => return not_found(),
                Err(response) => return response,
            };
            data.insert("boardId".to_string(), json!(entity_id));
            data.insert("websiteIds".to_string(), json!(scope.website_ids));
            data.insert("pixelIds".to_string(), json!(scope.pixel_ids));
            data.insert("linkIds".to_string(), json!(scope.link_ids));
        }
        _ => return not_found(),
    }

    let token = match create_share_token(&data) {
        Some(token) => token,
        None => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to create share token",
            )
        }
    };
    data.insert("token".to_string(), json!(token));
    json_response(Value::Object(data))
}

async fn create_share(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<Value>, JsonRejection>,
) -> Response {
    let input = match parse_create_share(body) {
        Ok(input) => input,
        Err(response) => return response,
    };
    let pool = match entity_pool(&state, &headers, input.entity_id, Access::Write).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let slug = input.slug.unwrap_or_else(random_slug);
    let row = sqlx::query(
        r#"
        INSERT INTO share
            (share_id, entity_id, name, share_type, slug, parameters, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, $6, NOW(), NOW())
        RETURNING share_id, entity_id, name, share_type, slug,
                  parameters::text AS parameters, created_at, updated_at
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(input.entity_id)
    .bind(input.name)
    .bind(input.share_type)
    .bind(slug)
    .bind(Value::Object(input.parameters))
    .fetch_one(&pool)
    .await;
    match row {
        Ok(row) => json_response(share_json(&row)),
        Err(error) if unique_slug_error(&error) => {
            error_response(StatusCode::BAD_REQUEST, "That slug is already taken.")
        }
        Err(error) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to create share: {error}"),
        ),
    }
}

async fn get_share(
    State(state): State<AppState>,
    Path(share_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let row = match fetch_share(pool, &share_id).await {
        Ok(Some(row)) => row,
        Ok(None) => return not_found(),
        Err(response) => return response,
    };
    let entity_id = row.get::<Uuid, _>("entity_id");
    if let Err(response) = entity_pool(&state, &headers, entity_id, Access::Read).await {
        return response;
    }
    json_response(share_json(&row))
}

async fn update_share(
    State(state): State<AppState>,
    Path(share_id): Path<String>,
    headers: HeaderMap,
    body: Result<Json<Value>, JsonRejection>,
) -> Response {
    let input = match parse_update_share(body) {
        Ok(input) => input,
        Err(response) => return response,
    };
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let row = match fetch_share(pool, &share_id).await {
        Ok(Some(row)) => row,
        Ok(None) => return not_found(),
        Err(response) => return response,
    };
    let entity_id = row.get::<Uuid, _>("entity_id");
    let pool = match entity_pool(&state, &headers, entity_id, Access::Write).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let share_id = row.get::<Uuid, _>("share_id");
    let row = sqlx::query(
        r#"
        UPDATE share
        SET name = $2, slug = $3, parameters = $4, updated_at = NOW()
        WHERE share_id = $1
        RETURNING share_id, entity_id, name, share_type, slug,
                  parameters::text AS parameters, created_at, updated_at
        "#,
    )
    .bind(share_id)
    .bind(input.name)
    .bind(input.slug)
    .bind(Value::Object(input.parameters))
    .fetch_one(&pool)
    .await;
    match row {
        Ok(row) => json_response(share_json(&row)),
        Err(error) if unique_slug_error(&error) => {
            error_response(StatusCode::BAD_REQUEST, "That slug is already taken.")
        }
        Err(error) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to update share: {error}"),
        ),
    }
}

async fn delete_share(
    State(state): State<AppState>,
    Path(share_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let pool = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let row = match fetch_share(pool, &share_id).await {
        Ok(Some(row)) => row,
        Ok(None) => return not_found(),
        Err(response) => return response,
    };
    let entity_id = row.get::<Uuid, _>("entity_id");
    let pool = match entity_pool(&state, &headers, entity_id, Access::Delete).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let share_id = row.get::<Uuid, _>("share_id");
    match sqlx::query("DELETE FROM share WHERE share_id = $1")
        .bind(share_id)
        .execute(&pool)
        .await
    {
        Ok(_) => json_response(json!({ "ok": true })),
        Err(error) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to delete share: {error}"),
        ),
    }
}

fn parse_create_share(body: Result<Json<Value>, JsonRejection>) -> Result<CreateShare, Response> {
    let Json(Value::Object(mut body)) =
        body.map_err(|_| error_response(StatusCode::BAD_REQUEST, "Invalid request body"))?
    else {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "Invalid request body",
        ));
    };
    let entity_id = take_uuid(&mut body, "entityId")?;
    let share_type = take_i32(&mut body, "shareType")?;
    let name = take_string(&mut body, "name", 200)?;
    let parameters = take_object(&mut body, "parameters")?;
    let slug = match body.remove("slug") {
        None => None,
        Some(Value::String(slug)) if slug.chars().count() <= 100 => {
            (!slug.is_empty()).then_some(slug)
        }
        _ => {
            return Err(error_response(
                StatusCode::BAD_REQUEST,
                "Invalid request body",
            ))
        }
    };
    Ok(CreateShare {
        entity_id,
        share_type,
        name,
        slug,
        parameters,
    })
}

fn parse_update_share(body: Result<Json<Value>, JsonRejection>) -> Result<UpdateShare, Response> {
    let Json(Value::Object(mut body)) =
        body.map_err(|_| error_response(StatusCode::BAD_REQUEST, "Invalid request body"))?
    else {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "Invalid request body",
        ));
    };
    Ok(UpdateShare {
        name: take_string(&mut body, "name", 200)?,
        slug: take_string(&mut body, "slug", 100)?,
        parameters: take_object(&mut body, "parameters")?,
    })
}

fn take_uuid(body: &mut Map<String, Value>, key: &str) -> Result<Uuid, Response> {
    body.remove(key)
        .and_then(|value| value.as_str().and_then(|value| Uuid::parse_str(value).ok()))
        .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, "Invalid request body"))
}

fn take_i32(body: &mut Map<String, Value>, key: &str) -> Result<i32, Response> {
    let value = body.remove(key);
    let parsed = match value {
        Some(Value::Number(value)) => value.as_i64(),
        Some(Value::String(value)) => value.parse::<i64>().ok(),
        _ => None,
    };
    parsed
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, "Invalid request body"))
}

fn take_string(
    body: &mut Map<String, Value>,
    key: &str,
    max_length: usize,
) -> Result<String, Response> {
    body.remove(key)
        .and_then(|value| value.as_str().map(str::to_string))
        .filter(|value| value.chars().count() <= max_length)
        .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, "Invalid request body"))
}

fn take_object(body: &mut Map<String, Value>, key: &str) -> Result<Map<String, Value>, Response> {
    body.remove(key)
        .and_then(|value| value.as_object().cloned())
        .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, "Invalid request body"))
}

fn create_share_token(data: &Map<String, Value>) -> Option<String> {
    create_share_token_with_secret(data, &secret_hash())
}

fn create_share_token_with_secret(data: &Map<String, Value>, secret: &str) -> Option<String> {
    let mut claims = data.clone();
    claims.insert("type".to_string(), json!("share"));
    claims.insert("iat".to_string(), json!(Utc::now().timestamp()));
    encode(
        &Header::new(Algorithm::HS256),
        &Value::Object(claims),
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .ok()
}

async fn fetch_share(pool: &PgPool, share_id: &str) -> Result<Option<PgRow>, Response> {
    let Some(share_id) = Uuid::parse_str(share_id).ok() else {
        return Ok(None);
    };
    sqlx::query(
        r#"
        SELECT share_id, entity_id, name, share_type, slug,
               parameters::text AS parameters, created_at, updated_at
        FROM share
        WHERE share_id = $1
        "#,
    )
    .bind(share_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to load share: {error}"),
        )
    })
}

async fn entity_exists(
    pool: &PgPool,
    table: &str,
    id_column: &str,
    id: Uuid,
) -> Result<bool, Response> {
    let query = format!(
        "SELECT EXISTS(SELECT 1 FROM {table} WHERE {id_column} = $1 AND deleted_at IS NULL)"
    );
    sqlx::query_scalar::<_, bool>(&query)
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(|error| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to load shared entity: {error}"),
            )
        })
}

fn share_json(row: &PgRow) -> Value {
    json!({
        "id": row.get::<Uuid, _>("share_id"),
        "entityId": row.get::<Uuid, _>("entity_id"),
        "name": row.get::<String, _>("name"),
        "shareType": row.get::<i32, _>("share_type"),
        "slug": row.get::<String, _>("slug"),
        "parameters": json_object(row.get::<String, _>("parameters")),
        "createdAt": row.get::<Option<DateTime<Utc>>, _>("created_at"),
        "updatedAt": row.get::<Option<DateTime<Utc>>, _>("updated_at")
    })
}

fn json_object(value: String) -> Map<String, Value> {
    serde_json::from_str::<Value>(&value)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default()
}

fn random_slug() -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(16)
        .map(char::from)
        .collect()
}

fn unique_slug_error(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|error| error.code())
        .as_deref()
        == Some("23505")
        && error
            .as_database_error()
            .and_then(|error| error.constraint())
            .is_some_and(|constraint| constraint.contains("slug"))
}

fn database(state: &AppState) -> Result<&PgPool, Response> {
    state
        .db
        .as_ref()
        .ok_or_else(|| error_response(StatusCode::SERVICE_UNAVAILABLE, "Database unavailable"))
}

fn not_found() -> Response {
    error_response(StatusCode::NOT_FOUND, "Not found")
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{decode, DecodingKey, Validation};

    #[test]
    fn create_input_matches_node_validation() {
        let entity_id = Uuid::new_v4();
        let parsed = parse_create_share(Ok(Json(json!({
            "entityId": entity_id,
            "shareType": "1",
            "name": "",
            "slug": "",
            "parameters": { "overview": true }
        }))))
        .unwrap();
        assert_eq!(parsed.entity_id, entity_id);
        assert_eq!(parsed.share_type, WEBSITE_SHARE);
        assert_eq!(parsed.slug, None);
        assert_eq!(parsed.parameters.get("overview"), Some(&json!(true)));

        assert!(parse_create_share(Ok(Json(json!({
            "entityId": entity_id,
            "name": "missing fields"
        }))))
        .is_err());
    }

    #[test]
    fn share_token_is_plain_hs256_and_explicitly_typed() {
        let secret = "share-test-secret";
        let share_id = Uuid::new_v4();
        let website_id = Uuid::new_v4();
        let data = Map::from_iter([
            ("shareId".to_string(), json!(share_id)),
            ("shareType".to_string(), json!(WEBSITE_SHARE)),
            ("websiteId".to_string(), json!(website_id)),
            ("parameters".to_string(), json!({ "overview": true })),
        ]);
        let token = create_share_token_with_secret(&data, secret).unwrap();
        let mut validation = Validation::new(Algorithm::HS256);
        validation.required_spec_claims.clear();
        let claims = decode::<Value>(
            &token,
            &DecodingKey::from_secret(secret.as_bytes()),
            &validation,
        )
        .unwrap()
        .claims;
        assert_eq!(claims.get("type"), Some(&json!("share")));
        assert_eq!(claims.get("shareId"), Some(&json!(share_id)));
        assert_eq!(claims.get("websiteId"), Some(&json!(website_id)));
    }
}
