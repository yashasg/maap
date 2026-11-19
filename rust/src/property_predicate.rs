use chrono::{DateTime, Utc};
use sqlx::{Postgres, QueryBuilder};
use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub(crate) struct PropertyPredicates {
    event: Vec<PropertyPredicate>,
    session: Vec<PropertyPredicate>,
    timezone: Option<String>,
}

#[derive(Clone, Debug)]
struct PropertyPredicate {
    property_name: String,
    data_type: DataType,
    operator: Operator,
    value: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DataType {
    String = 1,
    Number = 2,
    Boolean = 3,
    Date = 4,
    Array = 5,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Operator {
    Equals,
    NotEquals,
    Set,
    NotSet,
    Contains,
    DoesNotContain,
    Regex,
    NotRegex,
    True,
    False,
    GreaterThan,
    LessThan,
    GreaterThanEquals,
    LessThanEquals,
    Before,
    After,
}

impl PropertyPredicates {
    pub(crate) fn take(
        params: &mut HashMap<String, String>,
    ) -> Result<Self, axum::response::Response> {
        let timezone = params.get("timezone").cloned();
        let mut event = take_numbered(params, "epf");
        let mut session = take_numbered(params, "spf");
        event.sort_by(|(left, _), (right, _)| left.cmp(right));
        session.sort_by(|(left, _), (right, _)| left.cmp(right));

        Ok(Self {
            event: event
                .into_iter()
                .filter_map(|(_, value)| parse_predicate(&value))
                .collect(),
            session: session
                .into_iter()
                .filter_map(|(_, value)| parse_predicate(&value))
                .collect(),
            timezone,
        })
    }

    pub(crate) fn push(
        &self,
        query: &mut QueryBuilder<'_, Postgres>,
        event_alias: &'static str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) {
        if !self.event.is_empty() {
            push_event_predicates(
                query,
                event_alias,
                start,
                end,
                &self.event,
                self.timezone.as_deref(),
            );
        }
        if !self.session.is_empty() {
            push_session_predicates(query, event_alias, &self.session, self.timezone.as_deref());
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.event.is_empty() && self.session.is_empty()
    }
}

fn take_numbered(params: &mut HashMap<String, String>, prefix: &str) -> Vec<(String, String)> {
    let keys = params
        .keys()
        .filter(|key| numbered_suffix(key, prefix).is_some())
        .cloned()
        .collect::<Vec<_>>();
    keys.into_iter()
        .filter_map(|key| params.remove(&key).map(|value| (key, value)))
        .collect()
}

fn numbered_suffix<'a>(key: &'a str, prefix: &str) -> Option<&'a str> {
    let suffix = key.strip_prefix(prefix)?;
    (!suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())).then_some(suffix)
}

fn parse_predicate(value: &str) -> Option<PropertyPredicate> {
    let mut parts = value.splitn(4, '.');
    let data_type = match parts.next()?.trim_start_matches('0') {
        "1" => DataType::String,
        "2" => DataType::Number,
        "3" => DataType::Boolean,
        "4" => DataType::Date,
        "5" => DataType::Array,
        _ => return None,
    };
    let operator = parse_operator(parts.next()?)?;
    let encoded_name = parts.next()?;
    if encoded_name.is_empty() {
        return None;
    }
    let value = parts.next()?.to_string();
    if value
        .chars()
        .any(|character| matches!(character, '\n' | '\r' | '\u{2028}' | '\u{2029}'))
    {
        return None;
    }

    let usable = match data_type {
        DataType::Number => true,
        DataType::Date | DataType::Array => !value.is_empty(),
        DataType::String | DataType::Boolean => match operator {
            Operator::Equals | Operator::NotEquals => value.split(',').any(|item| !item.is_empty()),
            _ => !value.is_empty(),
        },
    };
    usable.then(|| PropertyPredicate {
        property_name: crate::decode_uri(encoded_name, false),
        data_type,
        operator,
        value,
    })
}

fn parse_operator(value: &str) -> Option<Operator> {
    Some(match value {
        "eq" => Operator::Equals,
        "neq" => Operator::NotEquals,
        "s" => Operator::Set,
        "ns" => Operator::NotSet,
        "c" => Operator::Contains,
        "dnc" => Operator::DoesNotContain,
        "re" => Operator::Regex,
        "nre" => Operator::NotRegex,
        "t" => Operator::True,
        "f" => Operator::False,
        "gt" => Operator::GreaterThan,
        "lt" => Operator::LessThan,
        "gte" => Operator::GreaterThanEquals,
        "lte" => Operator::LessThanEquals,
        "bf" => Operator::Before,
        "af" => Operator::After,
        _ => return None,
    })
}

fn push_event_predicates(
    query: &mut QueryBuilder<'_, Postgres>,
    event_alias: &'static str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    predicates: &[PropertyPredicate],
    timezone: Option<&str>,
) {
    query.push(" AND ");
    push_event_column(query, event_alias, "event_id");
    query.push(
        " IN (SELECT property_data.website_event_id FROM event_data property_data \
         WHERE property_data.website_id = ",
    );
    push_event_column(query, event_alias, "website_id");
    query
        .push(" AND property_data.created_at BETWEEN ")
        .push_bind(start)
        .push(" AND ")
        .push_bind(end)
        .push(" AND property_data.data_key = ANY(")
        .push_bind(
            predicates
                .iter()
                .map(|predicate| predicate.property_name.clone())
                .collect::<Vec<_>>(),
        )
        .push(") GROUP BY property_data.website_event_id HAVING ");
    push_having(query, predicates, timezone);
    query.push(")");
}

fn push_session_predicates(
    query: &mut QueryBuilder<'_, Postgres>,
    event_alias: &'static str,
    predicates: &[PropertyPredicate],
    timezone: Option<&str>,
) {
    query.push(
        " AND EXISTS (SELECT 1 FROM session_data property_data \
         WHERE property_data.website_id = ",
    );
    push_event_column(query, event_alias, "website_id");
    query.push(" AND property_data.session_id = ");
    push_event_column(query, event_alias, "session_id");
    query
        .push(" AND property_data.data_key = ANY(")
        .push_bind(
            predicates
                .iter()
                .map(|predicate| predicate.property_name.clone())
                .collect::<Vec<_>>(),
        )
        .push(") GROUP BY property_data.website_id, property_data.session_id HAVING ");
    push_having(query, predicates, timezone);
    query.push(")");
}

fn push_event_column(
    query: &mut QueryBuilder<'_, Postgres>,
    event_alias: &'static str,
    column: &'static str,
) {
    assert!(
        !event_alias.is_empty()
            && event_alias
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'),
        "event alias must be a SQL identifier"
    );
    query.push(event_alias).push(".").push(column);
}

fn push_having(
    query: &mut QueryBuilder<'_, Postgres>,
    predicates: &[PropertyPredicate],
    timezone: Option<&str>,
) {
    for (index, predicate) in predicates.iter().enumerate() {
        if index > 0 {
            query.push(" AND ");
        }
        query
            .push("bool_or(property_data.data_key = ")
            .push_bind(predicate.property_name.clone())
            .push(" AND property_data.data_type = ")
            .push_bind(predicate.data_type as i32)
            .push(" AND ");
        push_value_condition(query, predicate, timezone);
        query.push(")");
    }
}

fn push_value_condition(
    query: &mut QueryBuilder<'_, Postgres>,
    predicate: &PropertyPredicate,
    timezone: Option<&str>,
) {
    match predicate.data_type {
        DataType::Number => {
            query
                .push("CAST(property_data.number_value AS decimal) ")
                .push(number_operator(predicate.operator))
                .push(" ")
                .push_bind(js_parse_float(&predicate.value))
                .push("::numeric");
        }
        DataType::Date => {
            query.push("(property_data.date_value AT TIME ZONE ");
            if timezone.is_some_and(|timezone| {
                !timezone.is_empty() && !timezone.eq_ignore_ascii_case("utc")
            }) {
                query.push_bind(timezone.expect("checked timezone").to_string());
            } else {
                query.push("'utc'");
            }
            query
                .push(")::date ")
                .push(match predicate.operator {
                    Operator::Before => "<",
                    Operator::After => ">",
                    _ => "=",
                })
                .push(" ")
                .push_bind(predicate.value.clone())
                .push("::date");
        }
        DataType::Array => {
            if predicate.operator != Operator::Contains {
                query.push("NOT ");
            }
            query.push(
                "EXISTS (SELECT 1 FROM \
                 jsonb_array_elements_text(CASE WHEN property_data.data_type = 5 \
                 THEN COALESCE(property_data.string_value, '[]')::jsonb ELSE '[]'::jsonb END) \
                 AS array_item(value) WHERE array_item.value = ",
            );
            query.push_bind(predicate.value.clone()).push(")");
        }
        DataType::String | DataType::Boolean => match predicate.operator {
            Operator::Equals | Operator::NotEquals => {
                query
                    .push("property_data.string_value ")
                    .push(if predicate.operator == Operator::Equals {
                        "= ANY("
                    } else {
                        "!= ALL("
                    })
                    .push_bind(
                        predicate
                            .value
                            .split(',')
                            .filter(|item| !item.is_empty())
                            .map(str::to_string)
                            .collect::<Vec<_>>(),
                    )
                    .push(")");
            }
            Operator::Regex | Operator::NotRegex => {
                query
                    .push("property_data.string_value ")
                    .push(if predicate.operator == Operator::Regex {
                        "~* "
                    } else {
                        "!~* "
                    })
                    .push_bind(predicate.value.clone());
            }
            _ => {
                query
                    .push("property_data.string_value ")
                    .push(if predicate.operator == Operator::Contains {
                        "ILIKE "
                    } else {
                        "NOT ILIKE "
                    })
                    .push_bind(format!("%{}%", predicate.value));
            }
        },
    }
}

fn number_operator(operator: Operator) -> &'static str {
    match operator {
        Operator::NotEquals => "!=",
        Operator::GreaterThan => ">",
        Operator::LessThan => "<",
        Operator::GreaterThanEquals => ">=",
        Operator::LessThanEquals => "<=",
        _ => "=",
    }
}

fn js_parse_float(value: &str) -> String {
    let value = value.trim_start();
    let bytes = value.as_bytes();
    let mut index = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
    if value[index..].starts_with("Infinity") {
        return value[..index + "Infinity".len()].to_string();
    }

    let integer_start = index;
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    let mut has_digit = index > integer_start;
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        let fraction_start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        has_digit |= index > fraction_start;
    }
    if !has_digit {
        return "0".to_string();
    }

    let mantissa_end = index;
    if matches!(bytes.get(index), Some(b'e' | b'E')) {
        index += 1;
        if matches!(bytes.get(index), Some(b'+' | b'-')) {
            index += 1;
        }
        let exponent_start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == exponent_start {
            index = mantissa_end;
        }
    }

    let parsed = value[..index].parse::<f64>().unwrap_or(0.0);
    if parsed == 0.0 || parsed.is_nan() {
        "0".to_string()
    } else if parsed.is_infinite() {
        if parsed.is_sign_negative() {
            "-Infinity".to_string()
        } else {
            "Infinity".to_string()
        }
    } else {
        parsed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use sqlx::Execute;

    fn params(values: &[(&str, &str)]) -> HashMap<String, String> {
        values
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    fn sql_for(values: &[(&str, &str)]) -> String {
        let mut values = params(values);
        let predicates = PropertyPredicates::take(&mut values).unwrap();
        let mut builder = QueryBuilder::<Postgres>::new("SELECT 1 FROM website_event e WHERE true");
        predicates.push(
            &mut builder,
            "e",
            Utc.timestamp_millis_opt(1_000).single().unwrap(),
            Utc.timestamp_millis_opt(2_000).single().unwrap(),
        );
        builder.build().sql().to_string()
    }

    #[test]
    fn takes_only_numbered_universal_filters() {
        let mut values = params(&[
            ("epf0", "1.eq.plan%2Eid.pro"),
            ("spf12", "4.bf.created%20at.2026-07-01"),
            ("epf", "keep"),
            ("epfx", "keep"),
            ("pf_plan", "1.eq.keep"),
            ("timezone", "America/Los_Angeles"),
        ]);
        let predicates = PropertyPredicates::take(&mut values).unwrap();

        assert_eq!(predicates.event[0].property_name, "plan.id");
        assert_eq!(predicates.session[0].property_name, "created at");
        assert_eq!(predicates.timezone.as_deref(), Some("America/Los_Angeles"));
        assert_eq!(values.get("epf").map(String::as_str), Some("keep"));
        assert_eq!(values.get("epfx").map(String::as_str), Some("keep"));
        assert_eq!(values.get("pf_plan").map(String::as_str), Some("1.eq.keep"));
        assert!(values.contains_key("timezone"));
        assert!(!values.contains_key("epf0"));
        assert!(!values.contains_key("spf12"));
    }

    #[test]
    fn ignores_malformed_and_generator_empty_filters() {
        let mut values = params(&[
            ("epf0", "eq.plan.value"),
            ("epf1", "1.unknown.plan.value"),
            ("epf2", "1.eq.plan."),
            ("spf0", "4.eq.date."),
            ("spf1", "2.eq.count.not-a-number"),
        ]);
        let predicates = PropertyPredicates::take(&mut values).unwrap();

        assert!(predicates.event.is_empty());
        assert_eq!(predicates.session.len(), 1);
        assert_eq!(predicates.session[0].data_type, DataType::Number);
        assert!(values.is_empty());
    }

    #[test]
    fn preserves_repeated_property_keys_as_distinct_predicates() {
        let mut values = params(&[
            ("epf10", "1.eq.plan.pro"),
            ("epf2", "1.eq.plan.team"),
            ("spf1", "5.c.roles.admin"),
            ("spf2", "5.dnc.roles.guest"),
        ]);
        let predicates = PropertyPredicates::take(&mut values).unwrap();

        assert_eq!(predicates.event.len(), 2);
        assert_eq!(predicates.event[0].value, "pro");
        assert_eq!(predicates.event[1].value, "team");
        assert_eq!(predicates.session.len(), 2);
    }

    #[test]
    fn emits_event_and_session_scopes_with_typed_conditions() {
        let sql = sql_for(&[
            ("timezone", "America/Los_Angeles"),
            ("epf0", "4.af.created%2Eat.2026-07-01"),
            ("epf1", "2.gte.amount.5.5"),
            ("spf0", "5.c.roles.admin"),
            ("spf1", "1.neq.roles.free,trial"),
        ]);

        assert!(sql.contains("e.event_id IN (SELECT property_data.website_event_id"));
        assert!(sql.contains("property_data.created_at BETWEEN $1 AND $2"));
        assert!(sql.contains("(property_data.date_value AT TIME ZONE $"));
        assert!(sql.contains("CAST(property_data.number_value AS decimal) >= $"));
        assert!(sql.contains("AND EXISTS (SELECT 1 FROM session_data property_data"));
        assert!(sql.contains("property_data.session_id = e.session_id"));
        assert!(sql.contains("jsonb_array_elements_text"));
        assert!(sql.contains("CASE WHEN property_data.data_type = 5"));
        assert!(sql.contains("property_data.string_value != ALL("));
    }

    #[test]
    fn binds_property_names_values_and_timezone() {
        let injection = "x'); DROP TABLE event_data; --";
        let sql = sql_for(&[
            ("timezone", injection),
            ("epf0", "1.re.name.x'); DROP TABLE website_event; --"),
        ]);

        assert!(!sql.contains("DROP TABLE"));
        assert!(sql.contains('$'));
    }

    #[test]
    fn matches_javascript_parse_float_prefix_behavior() {
        assert_eq!(js_parse_float("  -12.5tail"), "-12.5");
        assert_eq!(js_parse_float("1e"), "1");
        assert_eq!(js_parse_float("0x10"), "0");
        assert_eq!(js_parse_float("garbage"), "0");
        assert_eq!(js_parse_float("+Infinity!"), "+Infinity");
    }

    #[test]
    fn default_is_empty() {
        assert!(PropertyPredicates::default().is_empty());
    }
}
