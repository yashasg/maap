use crate::{check_bearer_auth, error_response, json_response, AppState};
use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::Response,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

const SETTING_KEY_PREFIX: &str = "workspaceDiscovery:";
const SELECT_SQL: &str = "SELECT value FROM app_setting WHERE key = $1";
const UPSERT_SQL: &str = r#"
    INSERT INTO app_setting ("key", value)
    VALUES ($1, $2)
    ON CONFLICT ("key") DO UPDATE SET value = EXCLUDED.value
    RETURNING value
"#;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum DiscoveryChoice {
    Search,
    Recommendation,
    Community,
    Content,
    Comparison,
    Other,
}

impl DiscoveryChoice {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "search" => Some(Self::Search),
            "recommendation" => Some(Self::Recommendation),
            "community" => Some(Self::Community),
            "content" => Some(Self::Content),
            "comparison" => Some(Self::Comparison),
            "other" => Some(Self::Other),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Search => "search",
            Self::Recommendation => "recommendation",
            Self::Community => "community",
            Self::Content => "content",
            Self::Comparison => "comparison",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DiscoveryInput {
    choice: DiscoveryChoice,
}

pub(super) fn register_routes(router: Router<AppState>) -> Router<AppState> {
    router.route(
        "/api/me/discovery",
        get(get_discovery).post(update_discovery),
    )
}

fn database(state: &AppState) -> Result<&PgPool, Response> {
    state
        .db
        .as_ref()
        .ok_or_else(|| error_response(StatusCode::SERVICE_UNAVAILABLE, "Database unavailable"))
}

fn setting_key(user_id: Uuid) -> String {
    format!("{SETTING_KEY_PREFIX}{user_id}")
}

fn private_json(choice: Option<DiscoveryChoice>) -> Response {
    let mut response = json_response(json!({ "choice": choice }));
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store, max-age=0"),
    );
    response
}

async fn authenticate(state: &AppState, headers: &HeaderMap) -> Result<(PgPool, Uuid), Response> {
    let pool = database(state)?;
    let Some((user_id, _)) = check_bearer_auth(pool, headers, "/api/me/discovery").await else {
        return Err(error_response(StatusCode::UNAUTHORIZED, "Unauthorized"));
    };
    Ok((pool.clone(), user_id))
}

async fn get_discovery(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let (pool, user_id) = match authenticate(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let value = match sqlx::query_scalar::<_, String>(SELECT_SQL)
        .bind(setting_key(user_id))
        .fetch_optional(&pool)
        .await
    {
        Ok(value) => value,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to load discovery choice",
            )
        }
    };
    let choice = match value {
        Some(value) => match DiscoveryChoice::parse(&value) {
            Some(choice) => Some(choice),
            None => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Invalid saved discovery choice",
                )
            }
        },
        None => None,
    };

    private_json(choice)
}

async fn update_discovery(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<DiscoveryInput>,
) -> Response {
    let (pool, user_id) = match authenticate(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let value = match sqlx::query_scalar::<_, String>(UPSERT_SQL)
        .bind(setting_key(user_id))
        .bind(input.choice.as_str())
        .fetch_one(&pool)
        .await
    {
        Ok(value) => value,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to save discovery choice",
            )
        }
    };
    let Some(choice) = DiscoveryChoice::parse(&value) else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Invalid saved discovery choice",
        );
    };

    private_json(Some(choice))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_documented_choices_and_no_extra_fields() {
        for choice in [
            "search",
            "recommendation",
            "community",
            "content",
            "comparison",
            "other",
        ] {
            assert!(serde_json::from_value::<DiscoveryInput>(json!({ "choice": choice })).is_ok());
        }
        assert!(
            serde_json::from_value::<DiscoveryInput>(json!({ "choice": "custom answer" })).is_err()
        );
        assert!(serde_json::from_value::<DiscoveryInput>(
            json!({ "choice": "search", "key": "twoFactorRequiredGlobal" })
        )
        .is_err());
    }

    #[test]
    fn uses_a_fixed_per_account_namespace() {
        let user_id = Uuid::parse_str("123e4567-e89b-12d3-a456-426614174000").unwrap();

        assert_eq!(
            setting_key(user_id),
            "workspaceDiscovery:123e4567-e89b-12d3-a456-426614174000"
        );
        assert!(SELECT_SQL.contains("WHERE key = $1"));
        assert!(UPSERT_SQL.contains("ON CONFLICT (\"key\") DO UPDATE"));
    }
}
