use crate::{
    access::{website_pool, Access},
    error_response, json_response, AppState,
};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

const SELECT_SQL: &str = r#"
    SELECT overview_layout, color_scheme, revenue_display_mode
    FROM app_settings
    WHERE website_id = $1
      AND EXISTS (
          SELECT 1
          FROM website
          WHERE website_id = $1 AND deleted_at IS NULL
      )
"#;

const UPSERT_SQL: &str = r#"
    WITH live_website AS (
        SELECT website_id
        FROM website
        WHERE website_id = $1 AND deleted_at IS NULL
        FOR UPDATE
    )
    INSERT INTO app_settings
        (website_id, overview_layout, color_scheme, revenue_display_mode, created_at, updated_at)
    SELECT website_id, $2, $3, $4, now(), now()
    FROM live_website
    ON CONFLICT (website_id) DO UPDATE SET
        overview_layout = EXCLUDED.overview_layout,
        color_scheme = EXCLUDED.color_scheme,
        revenue_display_mode = EXCLUDED.revenue_display_mode,
        updated_at = now()
    RETURNING overview_layout, color_scheme, revenue_display_mode
"#;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum OverviewLayout {
    Application,
    Website,
}

impl OverviewLayout {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "application" => Some(Self::Application),
            "website" => Some(Self::Website),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Application => "application",
            Self::Website => "website",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
enum ColorScheme {
    Default,
    Vibrant,
    HighContrast,
    Emo,
}

impl ColorScheme {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "default" => Some(Self::Default),
            "vibrant" => Some(Self::Vibrant),
            "high-contrast" => Some(Self::HighContrast),
            "emo" => Some(Self::Emo),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Vibrant => "vibrant",
            Self::HighContrast => "high-contrast",
            Self::Emo => "emo",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum RevenueDisplayMode {
    None,
    Revenuecat,
}

impl RevenueDisplayMode {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "none" => Some(Self::None),
            "revenuecat" => Some(Self::Revenuecat),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Revenuecat => "revenuecat",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AppSettingsInput {
    overview_layout: OverviewLayout,
    color_scheme: ColorScheme,
    revenue_display_mode: RevenueDisplayMode,
}

impl Default for AppSettingsInput {
    fn default() -> Self {
        Self {
            overview_layout: OverviewLayout::Application,
            color_scheme: ColorScheme::Default,
            revenue_display_mode: RevenueDisplayMode::None,
        }
    }
}

impl AppSettingsInput {
    fn from_row(row: &sqlx::postgres::PgRow) -> Option<Self> {
        let overview_layout = row.get::<String, _>("overview_layout");
        let color_scheme = row.get::<String, _>("color_scheme");
        let revenue_display_mode = row.get::<String, _>("revenue_display_mode");

        Some(Self {
            overview_layout: OverviewLayout::parse(&overview_layout)?,
            color_scheme: ColorScheme::parse(&color_scheme)?,
            revenue_display_mode: RevenueDisplayMode::parse(&revenue_display_mode)?,
        })
    }
}

pub(super) fn register_routes(router: Router<AppState>) -> Router<AppState> {
    router.route(
        "/api/websites/{website_id}/app-settings",
        get(get_app_settings).post(update_app_settings),
    )
}

fn parse_uuid(value: &str) -> Result<Uuid, Response> {
    Uuid::parse_str(value)
        .map_err(|_| error_response(StatusCode::BAD_REQUEST, "Invalid resource ID"))
}

fn response_json(website_id: Uuid, settings: AppSettingsInput, can_write: bool) -> Response {
    json_response(json!({
        "websiteId": website_id,
        "overviewLayout": settings.overview_layout,
        "colorScheme": settings.color_scheme,
        "revenueDisplayMode": settings.revenue_display_mode,
        "canWrite": can_write,
    }))
}

async fn get_app_settings(
    State(state): State<AppState>,
    Path(website_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let website_id = match parse_uuid(&website_id) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let pool = match website_pool(&state, &headers, website_id, Access::Read).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let can_write = website_pool(&state, &headers, website_id, Access::Write)
        .await
        .is_ok();
    let settings = match sqlx::query(SELECT_SQL)
        .bind(website_id)
        .fetch_optional(&pool)
        .await
    {
        Ok(Some(row)) => match AppSettingsInput::from_row(&row) {
            Some(value) => value,
            None => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Invalid app settings data",
                )
            }
        },
        Ok(None) => AppSettingsInput::default(),
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to load app settings",
            )
        }
    };

    response_json(website_id, settings, can_write)
}

async fn update_app_settings(
    State(state): State<AppState>,
    Path(website_id): Path<String>,
    headers: HeaderMap,
    Json(input): Json<AppSettingsInput>,
) -> Response {
    let website_id = match parse_uuid(&website_id) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let pool = match website_pool(&state, &headers, website_id, Access::Write).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let row = match sqlx::query(UPSERT_SQL)
        .bind(website_id)
        .bind(input.overview_layout.as_str())
        .bind(input.color_scheme.as_str())
        .bind(input.revenue_display_mode.as_str())
        .fetch_optional(&pool)
        .await
    {
        Ok(Some(row)) => row,
        Ok(None) => {
            return error_response(StatusCode::UNAUTHORIZED, "Unauthorized");
        }
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to save app settings",
            )
        }
    };
    let Some(settings) = AppSettingsInput::from_row(&row) else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Invalid app settings data",
        );
    };

    response_json(website_id, settings, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_the_documented_finite_options() {
        let valid = serde_json::from_value::<AppSettingsInput>(json!({
            "overviewLayout": "website",
            "colorScheme": "high-contrast",
            "revenueDisplayMode": "revenuecat"
        }));
        assert!(valid.is_ok());

        for invalid in [
            json!({
                "overviewLayout": "dashboard",
                "colorScheme": "default",
                "revenueDisplayMode": "none"
            }),
            json!({
                "overviewLayout": "application",
                "colorScheme": "custom",
                "revenueDisplayMode": "none"
            }),
            json!({
                "overviewLayout": "application",
                "colorScheme": "default",
                "revenueDisplayMode": "stripe"
            }),
        ] {
            assert!(serde_json::from_value::<AppSettingsInput>(invalid).is_err());
        }
    }

    #[test]
    fn queries_are_scoped_to_one_app_record() {
        assert!(SELECT_SQL.contains("WHERE website_id = $1"));
        assert!(SELECT_SQL.contains("deleted_at IS NULL"));
        assert!(UPSERT_SQL.contains("FROM live_website"));
        assert!(UPSERT_SQL.contains("FOR UPDATE"));
        assert!(UPSERT_SQL.contains("ON CONFLICT (website_id) DO UPDATE"));
        assert!(UPSERT_SQL.contains("RETURNING overview_layout"));
    }
}
