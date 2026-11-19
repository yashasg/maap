use crate::{cohort::Cohort, metrics::resolve_segment, property_predicate::PropertyPredicates};
use axum::response::Response;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, QueryBuilder};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Clone, Debug, Default)]
pub(crate) struct Audience {
    cohort: Option<Cohort>,
    properties: PropertyPredicates,
}

impl Audience {
    pub(crate) async fn resolve(
        pool: &PgPool,
        website_id: Uuid,
        params: &mut HashMap<String, String>,
    ) -> Result<Self, Response> {
        resolve_segment(pool, website_id, params).await?;
        Ok(Self {
            cohort: Cohort::take(pool, website_id, params).await?,
            properties: PropertyPredicates::take(params)?,
        })
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.cohort.is_none() && self.properties.is_empty()
    }

    pub(crate) fn push(
        &self,
        query: &mut QueryBuilder<'_, Postgres>,
        event_alias: &'static str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) {
        if let Some(cohort) = &self.cohort {
            cohort.push(query, event_alias);
        }
        self.properties.push(query, event_alias, start, end);
    }
}
