use crate::{
    access::{website_section_pool, ShareDataVisibility, ShareSection},
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
use regex::Regex;
use serde_json::{json, Value};
use sqlx::{Postgres, QueryBuilder, Row};
use std::{
    collections::{BTreeMap, HashMap},
    sync::OnceLock,
};
use uuid::Uuid;

#[derive(Clone, Copy)]
struct DeviceMetric {
    id: &'static str,
    primary: &'static str,
    fallback: &'static str,
}

const METRICS: &[DeviceMetric] = &[
    DeviceMetric {
        id: "models",
        primary: "TelemetryDeck.Device.modelName",
        fallback: "modelName",
    },
    DeviceMetric {
        id: "systemVersions",
        primary: "TelemetryDeck.Device.systemVersion",
        fallback: "systemVersion",
    },
    DeviceMetric {
        id: "types",
        primary: "TelemetryDeck.Device.modelName",
        fallback: "modelName",
    },
    DeviceMetric {
        id: "platforms",
        primary: "TelemetryDeck.Device.platform",
        fallback: "TelemetryDeck.Device.operatingSystem",
    },
    DeviceMetric {
        id: "screenWidths",
        primary: "TelemetryDeck.Device.screenResolutionWidth",
        fallback: "screenResolutionWidth",
    },
    DeviceMetric {
        id: "screenHeights",
        primary: "TelemetryDeck.Device.screenResolutionHeight",
        fallback: "screenResolutionHeight",
    },
    DeviceMetric {
        id: "colorSchemes",
        primary: "TelemetryDeck.UserPreference.colorScheme",
        fallback: "colorScheme",
    },
    DeviceMetric {
        id: "orientations",
        primary: "TelemetryDeck.Device.orientation",
        fallback: "orientation",
    },
];

pub(crate) fn register_devices_routes(router: Router<AppState>) -> Router<AppState> {
    router.route(
        "/api/websites/{website_id}/app-reports/devices",
        get(devices_report),
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

fn scoped_filter_params(params: &HashMap<String, String>) -> HashMap<String, String> {
    let mut scoped = ["tag", "testMode"]
        .into_iter()
        .filter_map(|key| {
            params
                .get(key)
                .map(|value| (key.to_string(), value.clone()))
        })
        .collect::<HashMap<_, _>>();
    if !scoped.contains_key("tag") {
        let tag = if params.get("testMode").map(String::as_str) == Some("true") {
            "test"
        } else {
            "neq.test"
        };
        scoped.insert("tag".to_string(), tag.to_string());
    }
    scoped
}

fn scoped_filters(params: &HashMap<String, String>) -> Result<AnalyticsFilterSet, Response> {
    AnalyticsFilterSet::from_params(&scoped_filter_params(params))
}

fn is_hardware_identifier(value: &str) -> bool {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN
        .get_or_init(|| Regex::new(r"^[A-Za-z][A-Za-z0-9-]*\d+(?:,\d+)?$").unwrap())
        .is_match(value)
}

fn report_json(
    metric: DeviceMetric,
    rows: impl IntoIterator<Item = (Option<String>, i64)>,
) -> Value {
    let mut counts = BTreeMap::<Option<String>, i64>::new();
    for (raw_value, signals) in rows {
        let trimmed = raw_value
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let value = if metric.id == "types"
            && trimmed
                .as_deref()
                .is_some_and(|value| !is_hardware_identifier(value))
        {
            None
        } else {
            trimmed
        };
        *counts.entry(value).or_default() += signals;
    }

    let total_signals = counts.values().sum::<i64>();
    let missing_signals = counts.get(&None).copied().unwrap_or_default();
    let mut values = counts.into_iter().collect::<Vec<_>>();
    values.sort_by(|(a_value, a_count), (b_value, b_count)| {
        b_count.cmp(a_count).then_with(|| match (a_value, b_value) {
            (None, None) => std::cmp::Ordering::Equal,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (Some(_), None) => std::cmp::Ordering::Less,
            (Some(a), Some(b)) => a.cmp(b),
        })
    });

    json!({
        "id": metric.id,
        "totalSignals": total_signals,
        "reportedSignals": total_signals - missing_signals,
        "values": values.into_iter().map(|(value, signals)| json!({
            "value": value,
            "signals": signals,
        })).collect::<Vec<_>>(),
    })
}

fn push_metric_values(query: &mut QueryBuilder<'_, Postgres>) {
    for (index, metric) in METRICS.iter().enumerate() {
        if index > 0 {
            query.push(", ");
        }
        query
            .push("(")
            .push_bind(index as i32)
            .push(", ")
            .push_bind(metric.primary)
            .push(", ")
            .push_bind(metric.fallback)
            .push(")");
    }
}

async fn devices_report(
    State(state): State<AppState>,
    Path(website_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let pool = match website_section_pool(
        &state,
        &headers,
        website_id,
        &[ShareSection::Events],
        ShareDataVisibility::Aggregate,
    )
    .await
    {
        Ok(pool) => pool,
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
        "WITH scoped AS (SELECT e.event_id, e.website_id FROM website_event e",
    );
    filters.push_session_join(&mut query);
    query
        .push(" WHERE e.website_id = ")
        .push_bind(website_id)
        .push(" AND e.created_at BETWEEN ")
        .push_bind(start)
        .push(" AND ")
        .push_bind(end)
        .push(" AND e.event_type IN (1, 2)");
    filters.push(&mut query);
    query.push("), metrics(metric_index, primary_key, fallback_key) AS (VALUES ");
    push_metric_values(&mut query);
    query.push(
        r#")
        SELECT metrics.metric_index,
               value.raw_value,
               count(*)::bigint AS signals
        FROM metrics
        CROSS JOIN scoped e
        LEFT JOIN LATERAL (
          SELECT NULLIF(btrim(d.string_value), '') AS raw_value
          FROM event_data d
          WHERE d.website_id = e.website_id
            AND d.website_event_id = e.event_id
            AND d.data_key IN (metrics.primary_key, metrics.fallback_key)
          ORDER BY CASE WHEN d.data_key = metrics.primary_key THEN 0 ELSE 1 END,
                   d.created_at DESC NULLS LAST,
                   d.event_data_id DESC
          LIMIT 1
        ) value ON true
        GROUP BY metrics.metric_index, value.raw_value
        ORDER BY metrics.metric_index
        "#,
    );

    match query.build().fetch_all(&pool).await {
        Ok(rows) => {
            let mut grouped = vec![Vec::<(Option<String>, i64)>::new(); METRICS.len()];
            for row in rows {
                let index = row.get::<i32, _>("metric_index");
                if let Some(values) = grouped.get_mut(index as usize) {
                    values.push((
                        row.get::<Option<String>, _>("raw_value"),
                        row.get::<i64, _>("signals"),
                    ));
                }
            }
            let reports = METRICS
                .iter()
                .copied()
                .zip(grouped)
                .map(|(metric, rows)| report_json(metric, rows))
                .collect::<Vec<_>>();
            json_response(json!({ "reports": reports }))
        }
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_missing_orientation_and_exact_tablet_dimensions() {
        let orientation = report_json(METRICS[7], [(Some("landscape".to_string()), 2), (None, 1)]);
        assert_eq!(orientation["totalSignals"], 3);
        assert_eq!(orientation["reportedSignals"], 2);
        assert_eq!(
            orientation["values"],
            json!([
                { "value": "landscape", "signals": 2 },
                { "value": null, "signals": 1 },
            ])
        );
        assert_eq!(
            report_json(METRICS[4], [(Some("834".to_string()), 3)])["values"][0],
            json!({ "value": "834", "signals": 3 })
        );
        assert_eq!(
            report_json(METRICS[5], [(Some("1194".to_string()), 3)])["values"][0],
            json!({ "value": "1194", "signals": 3 })
        );
    }

    #[test]
    fn type_report_does_not_invent_a_category_for_unknown_models() {
        let report = report_json(
            METRICS[2],
            [
                (Some("iPad14,3".to_string()), 2),
                (Some("Pixel 9".to_string()), 1),
                (None, 1),
            ],
        );
        assert_eq!(report["reportedSignals"], 2);
        assert_eq!(
            report["values"],
            json!([
                { "value": "iPad14,3", "signals": 2 },
                { "value": null, "signals": 2 },
            ])
        );
    }

    #[test]
    fn metric_values_are_valid_sql_tuples() {
        let mut query = QueryBuilder::<Postgres>::new("VALUES ");
        push_metric_values(&mut query);
        assert!(!query.sql().contains("(,"));
        assert_eq!(query.sql().matches('(').count(), METRICS.len());
        assert_eq!(query.sql().matches(')').count(), METRICS.len());
    }

    #[test]
    fn test_mode_selects_test_signals_without_widening_the_scope() {
        let test = scoped_filter_params(&HashMap::from([(
            "testMode".to_string(),
            "true".to_string(),
        )]));
        assert_eq!(test.get("tag").map(String::as_str), Some("test"));

        let release = scoped_filter_params(&HashMap::new());
        assert_eq!(release.get("tag").map(String::as_str), Some("neq.test"));
    }
}
