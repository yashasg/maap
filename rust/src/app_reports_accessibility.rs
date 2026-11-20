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
use serde_json::{json, Value};
use sqlx::{Postgres, QueryBuilder, Row};
use std::collections::{BTreeMap, HashMap};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MetricKind {
    ContentSize,
    Boolean,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AccessibilityMetric {
    id: &'static str,
    property: &'static str,
    kind: MetricKind,
}

const METRICS: &[AccessibilityMetric] = &[
    AccessibilityMetric {
        id: "preferredContentSize",
        property: "TelemetryDeck.Accessibility.preferredContentSizeCategory",
        kind: MetricKind::ContentSize,
    },
    AccessibilityMetric {
        id: "boldText",
        property: "TelemetryDeck.Accessibility.isBoldTextEnabled",
        kind: MetricKind::Boolean,
    },
    AccessibilityMetric {
        id: "reduceMotion",
        property: "TelemetryDeck.Accessibility.isReduceMotionEnabled",
        kind: MetricKind::Boolean,
    },
    AccessibilityMetric {
        id: "reduceTransparency",
        property: "TelemetryDeck.Accessibility.isReduceTransparencyEnabled",
        kind: MetricKind::Boolean,
    },
    AccessibilityMetric {
        id: "darkerSystemColors",
        property: "TelemetryDeck.Accessibility.isDarkerSystemColorsEnabled",
        kind: MetricKind::Boolean,
    },
    AccessibilityMetric {
        id: "differentiateWithoutColor",
        property: "TelemetryDeck.Accessibility.shouldDifferentiateWithoutColor",
        kind: MetricKind::Boolean,
    },
    AccessibilityMetric {
        id: "invertedColors",
        property: "TelemetryDeck.Accessibility.isInvertColorsEnabled",
        kind: MetricKind::Boolean,
    },
];

pub(crate) fn register_accessibility_routes(router: Router<AppState>) -> Router<AppState> {
    router.route(
        "/api/websites/{website_id}/app-reports/accessibility",
        get(accessibility_breakdowns),
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

fn safe_label(metric: AccessibilityMetric, present: bool, raw_value: Option<&str>) -> &'static str {
    if !present {
        return "Not reported";
    }

    let value = raw_value.unwrap_or_default().trim();
    match metric.kind {
        MetricKind::Boolean if value.eq_ignore_ascii_case("true") || value == "1" => "On",
        MetricKind::Boolean if value.eq_ignore_ascii_case("false") || value == "0" => "Off",
        MetricKind::Boolean => "Other reported value",
        MetricKind::ContentSize => match value {
            "XS" => "Extra small",
            "S" => "Small",
            "M" => "Medium",
            "L" => "Large",
            "XL" => "Extra large",
            "XXL" => "2× extra large",
            "XXXL" => "3× extra large",
            "AccessibilityM" => "Accessibility medium",
            "AccessibilityL" => "Accessibility large",
            "AccessibilityXL" => "Accessibility extra large",
            "AccessibilityXXL" => "Accessibility 2× extra large",
            "AccessibilityXXXL" => "Accessibility 3× extra large",
            _ => "Other reported size",
        },
    }
}

fn label_order(metric: AccessibilityMetric, label: &str) -> usize {
    match metric.kind {
        MetricKind::Boolean => match label {
            "On" => 0,
            "Off" => 1,
            "Other reported value" => 2,
            "Not reported" => 3,
            _ => 4,
        },
        MetricKind::ContentSize => match label {
            "Extra small" => 0,
            "Small" => 1,
            "Medium" => 2,
            "Large" => 3,
            "Extra large" => 4,
            "2× extra large" => 5,
            "3× extra large" => 6,
            "Accessibility medium" => 7,
            "Accessibility large" => 8,
            "Accessibility extra large" => 9,
            "Accessibility 2× extra large" => 10,
            "Accessibility 3× extra large" => 11,
            "Other reported size" => 12,
            "Not reported" => 13,
            _ => 14,
        },
    }
}

fn report_json(
    metric: AccessibilityMetric,
    rows: impl IntoIterator<Item = (bool, Option<String>, i64)>,
) -> Value {
    let mut counts = BTreeMap::<&'static str, i64>::new();
    for (present, raw_value, signals) in rows {
        let label = safe_label(metric, present, raw_value.as_deref());
        *counts.entry(label).or_default() += signals;
    }

    if metric.kind == MetricKind::Boolean {
        counts.entry("On").or_default();
        counts.entry("Off").or_default();
        counts.entry("Not reported").or_default();
    }

    let total_signals = counts.values().sum::<i64>();
    let missing_signals = counts.get("Not reported").copied().unwrap_or_default();
    let mut values = counts.into_iter().collect::<Vec<_>>();
    values.sort_by_key(|(label, _)| label_order(metric, label));

    json!({
        "id": metric.id,
        "totalSignals": total_signals,
        "reportedSignals": total_signals - missing_signals,
        "values": values.into_iter().map(|(label, signals)| json!({
            "label": label,
            "signals": signals,
        })).collect::<Vec<_>>(),
    })
}

async fn accessibility_breakdowns(
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
        "WITH filtered_events AS (SELECT e.event_id, e.website_id FROM website_event e",
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
    query.push("), metrics(metric_index, property_name) AS (VALUES ");
    for (index, metric) in METRICS.iter().enumerate() {
        if index > 0 {
            query.push(", ");
        }
        query
            .push("(")
            .push_bind(index as i32)
            .push(", ")
            .push_bind(metric.property)
            .push(")");
    }
    query.push(
        r#")
        SELECT m.metric_index,
               COALESCE(value.present, false) AS present,
               value.raw_value,
               count(*)::bigint AS signals
        FROM metrics m
        CROSS JOIN filtered_events e
        LEFT JOIN LATERAL (
          SELECT true AS present, d.string_value AS raw_value
          FROM event_data d
          WHERE d.website_id = e.website_id
            AND d.website_event_id = e.event_id
            AND d.data_key = m.property_name
          ORDER BY d.created_at DESC NULLS LAST, d.event_data_id DESC
          LIMIT 1
        ) value ON true
        GROUP BY m.metric_index, value.present, value.raw_value
        ORDER BY m.metric_index
        "#,
    );

    match query.build().fetch_all(&pool).await {
        Ok(rows) => {
            let mut grouped = vec![Vec::<(bool, Option<String>, i64)>::new(); METRICS.len()];
            for row in rows {
                let index = row.get::<i32, _>("metric_index");
                if let Some(values) = grouped.get_mut(index as usize) {
                    values.push((
                        row.get::<bool, _>("present"),
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
    fn metrics_follow_the_approved_reference_order() {
        assert_eq!(
            METRICS.iter().map(|metric| metric.id).collect::<Vec<_>>(),
            [
                "preferredContentSize",
                "boldText",
                "reduceMotion",
                "reduceTransparency",
                "darkerSystemColors",
                "differentiateWithoutColor",
                "invertedColors",
            ]
        );
    }

    #[test]
    fn absent_boolean_values_are_not_counted_as_off() {
        let report = report_json(
            METRICS[1],
            [
                (true, Some("true".to_string()), 2),
                (true, Some("false".to_string()), 3),
                (false, None, 5),
            ],
        );
        assert_eq!(report["totalSignals"], 10);
        assert_eq!(report["reportedSignals"], 5);
        assert_eq!(report["values"][0], json!({ "label": "On", "signals": 2 }));
        assert_eq!(report["values"][1], json!({ "label": "Off", "signals": 3 }));
        assert_eq!(
            report["values"][2],
            json!({ "label": "Not reported", "signals": 5 })
        );
    }

    #[test]
    fn unexpected_values_are_aggregated_without_being_echoed() {
        let report = report_json(
            METRICS[0],
            [
                (true, Some("L".to_string()), 4),
                (true, Some("private-value".to_string()), 2),
                (false, None, 1),
            ],
        );
        assert_eq!(
            report["values"],
            json!([
                { "label": "Large", "signals": 4 },
                { "label": "Other reported size", "signals": 2 },
                { "label": "Not reported", "signals": 1 },
            ])
        );
        assert!(!report.to_string().contains("private-value"));
    }
}
