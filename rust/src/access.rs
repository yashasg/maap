use super::{check_bearer_auth, error_response, secret_hash, AppState};
use axum::{
    http::{HeaderMap, StatusCode},
    response::Response,
};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde_json::{Map, Value};
use sqlx::{PgPool, Row};
use std::collections::HashSet;
use uuid::Uuid;

const SHARE_TOKEN_HEADER: &str = "x-umami-share-token";
const SHARE_CONTEXT_HEADER: &str = "x-umami-share-context";
const SHARE_TOKEN_TYPE: &str = "share";
const WEBSITE_SHARE: i32 = 1;
const LINK_SHARE: i32 = 2;
const PIXEL_SHARE: i32 = 3;
const BOARD_SHARE: i32 = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Access {
    Read,
    Write,
    Delete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ShareDataVisibility {
    Aggregate,
    Rows,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ShareSection {
    Overview,
    Events,
    Sessions,
    Realtime,
    Performance,
    Compare,
    Breakdown,
    Goals,
    Funnels,
    Journeys,
    Retention,
    Utm,
    Revenue,
    Attribution,
}

impl ShareSection {
    fn parameter(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Events => "events",
            Self::Sessions => "sessions",
            Self::Realtime => "realtime",
            Self::Performance => "performance",
            Self::Compare => "compare",
            Self::Breakdown => "breakdown",
            Self::Goals => "goals",
            Self::Funnels => "funnels",
            Self::Journeys => "journeys",
            Self::Retention => "retention",
            Self::Utm => "utm",
            Self::Revenue => "revenue",
            Self::Attribution => "attribution",
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ShareScope {
    pub website_ids: Vec<Uuid>,
    pub pixel_ids: Vec<Uuid>,
    pub link_ids: Vec<Uuid>,
}

impl ShareScope {
    fn contains(&self, entity_id: Uuid) -> bool {
        self.website_ids.contains(&entity_id)
            || self.pixel_ids.contains(&entity_id)
            || self.link_ids.contains(&entity_id)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct VerifiedShare {
    pub share_type: i32,
    pub entity_id: Uuid,
    pub parameters: Map<String, Value>,
    pub scope: ShareScope,
}

pub(crate) async fn website_pool(
    state: &AppState,
    headers: &HeaderMap,
    website_id: Uuid,
    access: Access,
) -> Result<PgPool, Response> {
    let pool = database(state)?;

    if has_share_credentials(headers) {
        return Err(unauthorized());
    }

    if user_has_website_access(&pool, headers, website_id, access).await? {
        Ok(pool)
    } else {
        Err(unauthorized())
    }
}

pub(crate) async fn entity_pool(
    state: &AppState,
    headers: &HeaderMap,
    entity_id: Uuid,
    access: Access,
) -> Result<PgPool, Response> {
    let pool = database(state)?;

    if has_share_credentials(headers) {
        return Err(unauthorized());
    }

    let Some((user_id, user_role)) = check_bearer_auth(&pool, headers, "/api/share").await else {
        return Err(unauthorized());
    };

    if user_role == "admin" {
        return Ok(pool);
    }

    let row = sqlx::query(
        r#"
        WITH entity AS (
            SELECT user_id, team_id, 1 AS priority
            FROM website
            WHERE website_id = $1 AND deleted_at IS NULL
            UNION ALL
            SELECT user_id, team_id, 2 AS priority
            FROM link
            WHERE link_id = $1 AND deleted_at IS NULL
            UNION ALL
            SELECT user_id, team_id, 3 AS priority
            FROM pixel
            WHERE pixel_id = $1 AND deleted_at IS NULL
            UNION ALL
            SELECT user_id, team_id, 4 AS priority
            FROM board
            WHERE board_id = $1
        )
        SELECT e.user_id, tu.role AS team_role
        FROM entity e
        LEFT JOIN team t ON t.team_id = e.team_id AND t.deleted_at IS NULL
        LEFT JOIN team_user tu ON tu.team_id = t.team_id AND tu.user_id = $2
        ORDER BY e.priority
        LIMIT 1
        "#,
    )
    .bind(entity_id)
    .bind(user_id)
    .fetch_optional(&pool)
    .await
    .map_err(|_| internal_error("Unable to verify entity access"))?;

    let allowed = row.is_some_and(|row| {
        row.get::<Option<Uuid>, _>("user_id") == Some(user_id)
            || row
                .get::<Option<String>, _>("team_role")
                .is_some_and(|role| team_role_allows(&role, access))
    });

    if allowed {
        Ok(pool)
    } else {
        Err(unauthorized())
    }
}

pub(crate) async fn website_metadata_pool(
    state: &AppState,
    headers: &HeaderMap,
    website_id: Uuid,
) -> Result<PgPool, Response> {
    share_aware_website_pool(state, headers, website_id, |share| {
        share.scope.contains(website_id)
    })
    .await
}

pub(crate) async fn website_filter_pool(
    state: &AppState,
    headers: &HeaderMap,
    website_id: Uuid,
) -> Result<PgPool, Response> {
    share_aware_website_pool(state, headers, website_id, |share| {
        share.scope.contains(website_id)
            && share.parameters.get("allowFilter").and_then(Value::as_bool) != Some(false)
    })
    .await
}

pub(crate) async fn website_section_pool(
    state: &AppState,
    headers: &HeaderMap,
    website_id: Uuid,
    sections: &[ShareSection],
    visibility: ShareDataVisibility,
) -> Result<PgPool, Response> {
    share_aware_website_pool(state, headers, website_id, |share| {
        share.scope.contains(website_id) && share_allows_sections(share, sections, visibility)
    })
    .await
}

async fn share_aware_website_pool(
    state: &AppState,
    headers: &HeaderMap,
    website_id: Uuid,
    share_allows: impl FnOnce(&VerifiedShare) -> bool,
) -> Result<PgPool, Response> {
    let pool = database(state)?;

    if has_share_credentials(headers) {
        let Some(share) = verified_share(&pool, headers).await? else {
            return Err(unauthorized());
        };
        return if share_allows(&share) {
            Ok(pool)
        } else {
            Err(unauthorized())
        };
    }

    if user_has_website_access(&pool, headers, website_id, Access::Read).await? {
        Ok(pool)
    } else {
        Err(unauthorized())
    }
}

pub(crate) async fn verified_share(
    pool: &PgPool,
    headers: &HeaderMap,
) -> Result<Option<VerifiedShare>, Response> {
    let Some(token) = headers
        .get(SHARE_TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    if headers
        .get(SHARE_CONTEXT_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .is_none_or(str::is_empty)
    {
        return Ok(None);
    }

    let mut validation = Validation::new(Algorithm::HS256);
    validation.required_spec_claims.clear();
    validation.validate_exp = true;
    validation.leeway = 0;
    let claims = match decode::<Value>(
        token,
        &DecodingKey::from_secret(secret_hash().as_bytes()),
        &validation,
    ) {
        Ok(token) => token.claims,
        Err(_) => return Ok(None),
    };
    if claims.get("type").and_then(Value::as_str) != Some(SHARE_TOKEN_TYPE) {
        return Ok(None);
    }
    let Some(share_id) = claims
        .get("shareId")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
    else {
        return Ok(None);
    };

    let row = sqlx::query(
        r#"
        SELECT entity_id, share_type, parameters::text AS parameters
        FROM share
        WHERE share_id = $1
        "#,
    )
    .bind(share_id)
    .fetch_optional(pool)
    .await
    .map_err(|_| internal_error("Unable to verify share"))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let entity_id: Uuid = row.get("entity_id");
    let share_type: i32 = row.get("share_type");

    if !token_matches_share(&claims, share_type, entity_id) {
        return Ok(None);
    }

    let parameters = row
        .get::<String, _>("parameters")
        .parse::<Value>()
        .ok()
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    let scope = match share_type {
        WEBSITE_SHARE => {
            if !entity_exists_for_type(pool, WEBSITE_SHARE, entity_id).await? {
                return Ok(None);
            }
            ShareScope {
                website_ids: vec![entity_id],
                ..Default::default()
            }
        }
        LINK_SHARE => {
            if !entity_exists_for_type(pool, LINK_SHARE, entity_id).await? {
                return Ok(None);
            }
            ShareScope {
                link_ids: vec![entity_id],
                ..Default::default()
            }
        }
        PIXEL_SHARE => {
            if !entity_exists_for_type(pool, PIXEL_SHARE, entity_id).await? {
                return Ok(None);
            }
            ShareScope {
                pixel_ids: vec![entity_id],
                ..Default::default()
            }
        }
        BOARD_SHARE => match board_share_scope(pool, entity_id).await? {
            Some(scope) => scope,
            None => return Ok(None),
        },
        _ => return Ok(None),
    };

    Ok(Some(VerifiedShare {
        share_type,
        entity_id,
        parameters,
        scope,
    }))
}

pub(crate) async fn board_share_scope(
    pool: &PgPool,
    board_id: Uuid,
) -> Result<Option<ShareScope>, Response> {
    let row = sqlx::query(
        r#"
        SELECT type, parameters::text AS parameters, user_id, team_id
        FROM board
        WHERE board_id = $1
        "#,
    )
    .bind(board_id)
    .fetch_optional(pool)
    .await
    .map_err(|_| internal_error("Unable to load shared board"))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let board_type: String = row.get("type");
    let parameters = row
        .get::<String, _>("parameters")
        .parse::<Value>()
        .unwrap_or(Value::Null);
    let selected = selected_board_entities(&board_type, &parameters);
    let user_id = row.get::<Option<Uuid>, _>("user_id");
    let team_id = row.get::<Option<Uuid>, _>("team_id");
    let owner_role = match user_id {
        Some(user_id) => sqlx::query_scalar::<_, String>(
            r#"SELECT role FROM "user" WHERE user_id = $1 AND deleted_at IS NULL"#,
        )
        .bind(user_id)
        .fetch_optional(pool)
        .await
        .map_err(|_| internal_error("Unable to verify shared board owner"))?,
        None => None,
    };
    if user_id.is_some() && team_id.is_none() && owner_role.is_none() {
        return Ok(Some(ShareScope::default()));
    }
    let owner_is_admin = owner_role.as_deref() == Some("admin");

    Ok(Some(ShareScope {
        website_ids: filter_board_ids(
            pool,
            "website",
            "website_id",
            &selected.website_ids,
            user_id,
            team_id,
            owner_is_admin,
        )
        .await?,
        pixel_ids: filter_board_ids(
            pool,
            "pixel",
            "pixel_id",
            &selected.pixel_ids,
            user_id,
            team_id,
            owner_is_admin,
        )
        .await?,
        link_ids: filter_board_ids(
            pool,
            "link",
            "link_id",
            &selected.link_ids,
            user_id,
            team_id,
            owner_is_admin,
        )
        .await?,
    }))
}

fn database(state: &AppState) -> Result<PgPool, Response> {
    state
        .db
        .clone()
        .ok_or_else(|| error_response(StatusCode::SERVICE_UNAVAILABLE, "Database unavailable"))
}

async fn user_has_website_access(
    pool: &PgPool,
    headers: &HeaderMap,
    website_id: Uuid,
    access: Access,
) -> Result<bool, Response> {
    let Some((user_id, user_role)) = check_bearer_auth(pool, headers, "/api/websites").await else {
        return Ok(false);
    };
    let query = match access {
        Access::Read => {
            r#"
        WITH entity AS (
            SELECT user_id, team_id, 1 AS priority
            FROM website
            WHERE website_id = $1 AND deleted_at IS NULL
            UNION ALL
            SELECT user_id, team_id, 2 AS priority
            FROM link
            WHERE link_id = $1 AND deleted_at IS NULL
            UNION ALL
            SELECT user_id, team_id, 3 AS priority
            FROM pixel
            WHERE pixel_id = $1 AND deleted_at IS NULL
            UNION ALL
            SELECT user_id, team_id, 4 AS priority
            FROM board
            WHERE board_id = $1
        )
        SELECT e.user_id, tu.role AS team_role
        FROM entity e
        LEFT JOIN team t ON t.team_id = e.team_id AND t.deleted_at IS NULL
        LEFT JOIN team_user tu ON tu.team_id = t.team_id AND tu.user_id = $2
        ORDER BY e.priority
        LIMIT 1
        "#
        }
        Access::Write | Access::Delete => {
            r#"
        SELECT w.user_id, tu.role AS team_role
        FROM website w
        LEFT JOIN team t ON t.team_id = w.team_id AND t.deleted_at IS NULL
        LEFT JOIN team_user tu ON tu.team_id = t.team_id AND tu.user_id = $2
        WHERE w.website_id = $1 AND w.deleted_at IS NULL
        "#
        }
    };
    let row = sqlx::query(query)
        .bind(website_id)
        .bind(user_id)
        .fetch_optional(pool)
        .await
        .map_err(|_| internal_error("Unable to verify website access"))?;

    let website = row.map(|row| {
        (
            row.get::<Option<Uuid>, _>("user_id"),
            row.get::<Option<String>, _>("team_role"),
        )
    });
    Ok(website_access_allowed(website, user_id, &user_role, access))
}

fn website_access_allowed(
    website: Option<(Option<Uuid>, Option<String>)>,
    user_id: Uuid,
    user_role: &str,
    access: Access,
) -> bool {
    website.is_some_and(|(owner_id, team_role)| {
        user_role == "admin"
            || owner_id == Some(user_id)
            || team_role.is_some_and(|role| team_role_allows(&role, access))
    })
}

fn has_share_credentials(headers: &HeaderMap) -> bool {
    headers.contains_key(SHARE_TOKEN_HEADER) || headers.contains_key(SHARE_CONTEXT_HEADER)
}

fn share_allows_sections(
    share: &VerifiedShare,
    sections: &[ShareSection],
    visibility: ShareDataVisibility,
) -> bool {
    if sections.is_empty() {
        return false;
    }
    if share.share_type == BOARD_SHARE && visibility == ShareDataVisibility::Rows {
        return false;
    }

    let has_section_parameters = share
        .parameters
        .iter()
        .any(|(key, value)| key != "allowFilter" && value.is_boolean());
    if !has_section_parameters {
        return sections
            .iter()
            .any(|section| share.share_type != BOARD_SHARE || *section != ShareSection::Sessions);
    }

    sections.iter().any(|section| {
        share
            .parameters
            .get(section.parameter())
            .and_then(Value::as_bool)
            == Some(true)
    })
}

fn token_matches_share(claims: &Value, share_type: i32, entity_id: Uuid) -> bool {
    if claims.get("shareType").and_then(Value::as_i64) != Some(i64::from(share_type)) {
        return false;
    }
    let matches_id = |key: &str| {
        claims
            .get(key)
            .and_then(Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
            == Some(entity_id)
    };
    match share_type {
        WEBSITE_SHARE => matches_id("websiteId"),
        LINK_SHARE => matches_id("websiteId") && matches_id("linkId"),
        PIXEL_SHARE => matches_id("websiteId") && matches_id("pixelId"),
        BOARD_SHARE => matches_id("boardId"),
        _ => false,
    }
}

async fn entity_exists_for_type(
    pool: &PgPool,
    share_type: i32,
    entity_id: Uuid,
) -> Result<bool, Response> {
    let query = match share_type {
        WEBSITE_SHARE => {
            "SELECT EXISTS(SELECT 1 FROM website WHERE website_id = $1 AND deleted_at IS NULL)"
        }
        LINK_SHARE => "SELECT EXISTS(SELECT 1 FROM link WHERE link_id = $1 AND deleted_at IS NULL)",
        PIXEL_SHARE => {
            "SELECT EXISTS(SELECT 1 FROM pixel WHERE pixel_id = $1 AND deleted_at IS NULL)"
        }
        _ => return Ok(false),
    };
    sqlx::query_scalar::<_, bool>(query)
        .bind(entity_id)
        .fetch_one(pool)
        .await
        .map_err(|_| internal_error("Unable to verify shared entity"))
}

fn selected_board_entities(board_type: &str, parameters: &Value) -> ShareScope {
    let mut scope = ShareScope::default();
    let Some(parameters) = parameters.as_object() else {
        return scope;
    };

    match board_type {
        "website" => push_uuid(&mut scope.website_ids, parameters.get("websiteId")),
        "pixel" => push_uuid(&mut scope.pixel_ids, parameters.get("pixelId")),
        "link" => push_uuid(&mut scope.link_ids, parameters.get("linkId")),
        _ => {}
    }
    push_uuid(&mut scope.website_ids, parameters.get("websiteId"));
    push_uuid(&mut scope.pixel_ids, parameters.get("pixelId"));
    push_uuid(&mut scope.link_ids, parameters.get("linkId"));

    for row in parameters
        .get("rows")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for column in row
            .get("columns")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(component) = column.get("component").and_then(Value::as_object) else {
                continue;
            };
            match (
                component.get("entityType").and_then(Value::as_str),
                component.get("entityId"),
            ) {
                (Some("website"), Some(id)) if id.is_string() => {
                    push_uuid(&mut scope.website_ids, Some(id))
                }
                (Some("pixel"), Some(id)) if id.is_string() => {
                    push_uuid(&mut scope.pixel_ids, Some(id))
                }
                (Some("link"), Some(id)) if id.is_string() => {
                    push_uuid(&mut scope.link_ids, Some(id))
                }
                _ => push_uuid(&mut scope.website_ids, component.get("websiteId")),
            }
        }
    }
    scope
}

fn push_uuid(ids: &mut Vec<Uuid>, value: Option<&Value>) {
    let Some(id) = value
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
    else {
        return;
    };
    if !ids.contains(&id) {
        ids.push(id);
    }
}

async fn filter_board_ids(
    pool: &PgPool,
    table: &str,
    id_column: &str,
    ids: &[Uuid],
    user_id: Option<Uuid>,
    team_id: Option<Uuid>,
    owner_is_admin: bool,
) -> Result<Vec<Uuid>, Response> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let ownership = if team_id.is_some() {
        "e.team_id = $2"
    } else if user_id.is_some() && !owner_is_admin {
        r#"(e.user_id = $2 OR EXISTS (
            SELECT 1 FROM team_user tu
            JOIN team t ON t.team_id = tu.team_id
            WHERE tu.team_id = e.team_id AND tu.user_id = $2 AND t.deleted_at IS NULL
        ))"#
    } else if owner_is_admin {
        "true"
    } else {
        "false"
    };
    let query = format!(
        "SELECT e.{id_column} FROM {table} e \
         WHERE e.{id_column} = ANY($1::uuid[]) AND e.deleted_at IS NULL AND {ownership}"
    );
    let mut query = sqlx::query_scalar::<_, Uuid>(&query).bind(ids);
    if !owner_is_admin {
        if let Some(owner_id) = team_id.or(user_id) {
            query = query.bind(owner_id);
        }
    }
    let allowed = query
        .fetch_all(pool)
        .await
        .map_err(|_| internal_error("Unable to authorize shared board entities"))?
        .into_iter()
        .collect::<HashSet<_>>();
    Ok(ids
        .iter()
        .copied()
        .filter(|id| allowed.contains(id))
        .collect())
}

fn unauthorized() -> Response {
    error_response(StatusCode::UNAUTHORIZED, "Unauthorized")
}

fn internal_error(message: &str) -> Response {
    error_response(StatusCode::INTERNAL_SERVER_ERROR, message)
}

fn team_role_allows(role: &str, access: Access) -> bool {
    match access {
        Access::Read => matches!(
            role,
            "team-owner" | "team-manager" | "team-member" | "team-view-only"
        ),
        Access::Write | Access::Delete => {
            matches!(role, "team-owner" | "team-manager" | "team-member")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn share(share_type: i32, parameters: Value) -> VerifiedShare {
        VerifiedShare {
            share_type,
            entity_id: Uuid::nil(),
            parameters: parameters.as_object().cloned().unwrap_or_default(),
            scope: ShareScope::default(),
        }
    }

    #[test]
    fn team_permissions_match_node_roles() {
        assert!(team_role_allows("team-view-only", Access::Read));
        assert!(!team_role_allows("team-view-only", Access::Write));
        assert!(!team_role_allows("team-view-only", Access::Delete));
        assert!(team_role_allows("team-member", Access::Write));
        assert!(team_role_allows("team-manager", Access::Delete));
        assert!(!team_role_allows("unknown", Access::Read));
    }

    #[test]
    fn admin_access_still_requires_a_live_website() {
        let user_id = Uuid::new_v4();
        assert!(!website_access_allowed(
            None,
            user_id,
            "admin",
            Access::Read
        ));
        assert!(website_access_allowed(
            Some((None, None)),
            user_id,
            "admin",
            Access::Delete
        ));
    }

    #[test]
    fn share_credentials_fail_closed_on_legacy_access() {
        let mut headers = HeaderMap::new();
        assert!(!has_share_credentials(&headers));
        headers.insert(SHARE_CONTEXT_HEADER, "1".parse().unwrap());
        assert!(has_share_credentials(&headers));
    }

    #[test]
    fn section_and_row_visibility_match_node_permissions() {
        let gated = share(WEBSITE_SHARE, json!({ "overview": true, "events": false }));
        assert!(share_allows_sections(
            &gated,
            &[ShareSection::Overview],
            ShareDataVisibility::Aggregate
        ));
        assert!(!share_allows_sections(
            &gated,
            &[ShareSection::Events],
            ShareDataVisibility::Aggregate
        ));

        let board = share(BOARD_SHARE, json!({}));
        assert!(share_allows_sections(
            &board,
            &[ShareSection::Goals],
            ShareDataVisibility::Aggregate
        ));
        assert!(!share_allows_sections(
            &board,
            &[ShareSection::Sessions],
            ShareDataVisibility::Aggregate
        ));
        assert!(!share_allows_sections(
            &board,
            &[ShareSection::Events],
            ShareDataVisibility::Rows
        ));
    }

    #[test]
    fn board_scope_uses_only_selected_entity_ids() {
        let website_id = Uuid::new_v4();
        let pixel_id = Uuid::new_v4();
        let link_id = Uuid::new_v4();
        let scope = selected_board_entities(
            "mixed",
            &json!({
                "rows": [{
                    "columns": [
                        { "component": { "entityType": "website", "entityId": website_id } },
                        { "component": { "entityType": "pixel", "entityId": pixel_id } },
                        { "component": { "entityType": "link", "entityId": link_id } },
                        { "component": { "entityType": "website", "entityId": website_id } }
                    ]
                }]
            }),
        );
        assert_eq!(scope.website_ids, vec![website_id]);
        assert_eq!(scope.pixel_ids, vec![pixel_id]);
        assert_eq!(scope.link_ids, vec![link_id]);
    }

    #[test]
    fn token_scope_must_match_the_stored_share() {
        let entity_id = Uuid::new_v4();
        let valid = json!({
            "shareType": WEBSITE_SHARE,
            "websiteId": entity_id
        });
        let wrong = json!({
            "shareType": WEBSITE_SHARE,
            "websiteId": Uuid::new_v4()
        });
        assert!(token_matches_share(&valid, WEBSITE_SHARE, entity_id));
        assert!(!token_matches_share(&wrong, WEBSITE_SHARE, entity_id));
    }
}
