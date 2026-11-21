use crate::{
    access::{website_pool, Access},
    error_response, json_response,
    lifecycle::{clamp_to_activity_floor, website_activity_floor},
    metrics::AnalyticsFilterSet,
    AppState,
};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::get,
    Router,
};
use chrono::{DateTime, TimeZone, Utc};
use serde_json::{json, Value};
use sqlx::{Postgres, QueryBuilder, Row};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VersionDimension {
    AppVersion,
    BuildNumber,
    SdkVersion,
}

impl VersionDimension {
    fn parse(value: Option<&str>) -> Result<Self, Response> {
        match value {
            Some("appVersion") => Ok(Self::AppVersion),
            Some("buildNumber") => Ok(Self::BuildNumber),
            Some("sdkVersion") => Ok(Self::SdkVersion),
            _ => Err(error_response(
                StatusCode::BAD_REQUEST,
                "dimension must be appVersion, buildNumber, or sdkVersion",
            )),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::AppVersion => "appVersion",
            Self::BuildNumber => "buildNumber",
            Self::SdkVersion => "sdkVersion",
        }
    }

    fn keys(self) -> &'static [&'static str] {
        match self {
            Self::AppVersion => &["TelemetryDeck.AppInfo.version", "appVersion"],
            Self::BuildNumber => &["TelemetryDeck.AppInfo.buildNumber", "buildNumber"],
            Self::SdkVersion => &["TelemetryDeck.SDK.version", "sdkVersion"],
        }
    }
}

pub fn register_versions_routes(router: Router<AppState>) -> Router<AppState> {
    router.route(
        "/api/websites/{website_id}/app-reports/versions",
        get(version_breakdown),
    )
}

fn required_date_bounds(
    params: &HashMap<String, String>,
) -> Result<(DateTime<Utc>, DateTime<Utc>), Response> {
    let parse = |key: &str| {
        params
            .get(key)
            .and_then(|value| value.parse::<i64>().ok())
            .and_then(|value| Utc.timestamp_millis_opt(value).single())
    };
    let (Some(start), Some(end)) = (parse("startAt"), parse("endAt")) else {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "A valid start and end date are required",
        ));
    };
    if start > end {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "The start date must not be after the end date",
        ));
    }
    Ok((start, end))
}

fn scoped_filters(params: &HashMap<String, String>) -> Result<AnalyticsFilterSet, Response> {
    let scoped = ["tag", "testMode"]
        .into_iter()
        .filter_map(|key| {
            params
                .get(key)
                .map(|value| (key.to_string(), value.clone()))
        })
        .collect();
    AnalyticsFilterSet::from_params(&scoped)
}

fn push_dimension_value(query: &mut QueryBuilder<'_, Postgres>, dimension: VersionDimension) {
    query.push(
        "LEFT JOIN LATERAL (SELECT NULLIF(BTRIM(d.string_value), '') AS value \
         FROM event_data d WHERE d.website_id = e.website_id \
         AND d.website_event_id = e.event_id AND d.data_key IN (",
    );
    {
        let mut separated = query.separated(", ");
        for key in dimension.keys() {
            separated.push_bind(*key);
        }
    }
    query.push(") ORDER BY CASE d.data_key ");
    for (index, key) in dimension.keys().iter().enumerate() {
        query
            .push("WHEN ")
            .push_bind(*key)
            .push(" THEN ")
            .push(index.to_string())
            .push(" ");
    }
    query.push(" ELSE 99 END LIMIT 1) dimension ON TRUE");
}

async fn version_breakdown(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match website_pool(&state, &headers, website_id, Access::Read).await {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let dimension = match VersionDimension::parse(params.get("dimension").map(String::as_str)) {
        Ok(dimension) => dimension,
        Err(response) => return response,
    };
    let (requested_start, end) = match required_date_bounds(&params) {
        Ok(bounds) => bounds,
        Err(response) => return response,
    };
    let minimum = match website_activity_floor(&pool, website_id).await {
        Ok(minimum) => minimum,
        Err(response) => return response,
    };
    let start = clamp_to_activity_floor(requested_start, minimum);
    let filters = match scoped_filters(&params) {
        Ok(filters) => filters,
        Err(response) => return response,
    };

    let mut query = QueryBuilder::<Postgres>::new(
        "WITH filtered_events AS (SELECT e.event_id, e.website_id FROM website_event e",
    );
    filters.push_session_join(&mut query);
    query
        .push(" WHERE e.website_id = ")
        .push_bind(website_id)
        .push(" AND e.created_at BETWEEN ")
        .push_bind(start)
        .push(" AND ")
        .push_bind(end);
    filters.push(&mut query);
    query.push(
        "), dimension_events AS (SELECT dimension.value AS version \
                FROM filtered_events e ",
    );
    push_dimension_value(&mut query, dimension);
    query.push(
        ") SELECT version, count(*)::bigint AS events FROM dimension_events \
         GROUP BY version ORDER BY events DESC, version ASC NULLS LAST",
    );

    match query.build().fetch_all(&pool).await {
        Ok(rows) => {
            let values = rows
                .into_iter()
                .map(|row| {
                    let value = row.get::<Option<String>, _>("version");
                    let events = row.get::<i64, _>("events");
                    (value, events)
                })
                .collect::<Vec<_>>();
            let total_events = values.iter().map(|(_, events)| *events).sum::<i64>();
            let reported_events = values
                .iter()
                .filter(|(value, _)| value.is_some())
                .map(|(_, events)| *events)
                .sum::<i64>();
            let values = values
                .into_iter()
                .map(|(value, events)| json!({ "value": value, "events": events }))
                .collect::<Vec<Value>>();
            json_response(json!({
                "dimension": dimension.name(),
                "totalEvents": total_events,
                "reportedEvents": reported_events,
                "values": values
            }))
        }
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Execute;

    #[test]
    fn dimensions_use_signal_properties_in_precedence_order() {
        assert_eq!(
            VersionDimension::AppVersion.keys(),
            &["TelemetryDeck.AppInfo.version", "appVersion"]
        );
        assert_eq!(
            VersionDimension::BuildNumber.keys(),
            &["TelemetryDeck.AppInfo.buildNumber", "buildNumber"]
        );
        assert_eq!(
            VersionDimension::SdkVersion.keys(),
            &["TelemetryDeck.SDK.version", "sdkVersion"]
        );
    }

    #[test]
    fn date_bounds_are_required_and_ordered() {
        let params = HashMap::from([
            ("startAt".to_string(), "2000".to_string()),
            ("endAt".to_string(), "1000".to_string()),
        ]);
        assert!(required_date_bounds(&params).is_err());
        assert!(required_date_bounds(&HashMap::new()).is_err());
    }

    #[test]
    fn dimension_precedence_case_keeps_sql_tokens_separated() {
        let mut query = QueryBuilder::<Postgres>::new("");
        push_dimension_value(&mut query, VersionDimension::SdkVersion);
        let built = query.build();

        assert!(!built.sql().contains("0WHEN"));
        assert!(built.sql().contains("THEN 0 WHEN"));
    }
}
