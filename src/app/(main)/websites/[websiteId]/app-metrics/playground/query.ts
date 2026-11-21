export const QUERY_TYPES = ['traffic-timeseries', 'custom-events-timeseries'] as const;
export const GRANULARITIES = ['minute', 'hour', 'day', 'month', 'year'] as const;

export type PlaygroundQueryType = (typeof QUERY_TYPES)[number];
export type PlaygroundGranularity = (typeof GRANULARITIES)[number];

export type PlaygroundQuery = {
  queryType: PlaygroundQueryType;
  granularity: PlaygroundGranularity;
  limit?: number;
};

export type QueryScope = {
  websiteId: string;
  startAt: number;
  endAt: number;
  timezone: string;
  filters?: Record<string, unknown>;
};

export type QueryRequest = {
  path: string;
  params: Record<string, unknown>;
};

export type TrafficResultRow = {
  bucket: string;
  pageviews: number;
  sessions: number;
  pageviewsPerSession: number | null;
};

export type EventResultRow = {
  bucket: string;
  eventName: string;
  signals: number;
};

const allowedKeys = new Set(['queryType', 'granularity', 'limit']);

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value);
}

export function validatePlaygroundQuery(value: unknown): PlaygroundQuery {
  if (!isRecord(value)) {
    throw new Error('Query must be a JSON object.');
  }

  const unsupported = Object.keys(value).filter(key => !allowedKeys.has(key));
  if (unsupported.length > 0) {
    throw new Error(
      `Unsupported option${unsupported.length === 1 ? '' : 's'}: ${unsupported.join(', ')}. ` +
        'Only queryType, granularity, and limit are accepted.',
    );
  }

  if (!QUERY_TYPES.includes(value.queryType as PlaygroundQueryType)) {
    throw new Error(
      `Unsupported queryType. Use one of: ${QUERY_TYPES.map(type => `"${type}"`).join(', ')}.`,
    );
  }

  if (!GRANULARITIES.includes(value.granularity as PlaygroundGranularity)) {
    throw new Error(
      `Unsupported granularity. Use one of: ${GRANULARITIES.map(unit => `"${unit}"`).join(', ')}.`,
    );
  }

  if (value.queryType === 'traffic-timeseries' && value.limit !== undefined) {
    throw new Error('limit is only supported for custom-events-timeseries queries.');
  }

  if (value.queryType === 'custom-events-timeseries') {
    if (!Number.isInteger(value.limit) || Number(value.limit) < 1 || Number(value.limit) > 50) {
      throw new Error('limit must be an integer from 1 to 50 for custom event queries.');
    }
  }

  return {
    queryType: value.queryType as PlaygroundQueryType,
    granularity: value.granularity as PlaygroundGranularity,
    ...(value.queryType === 'custom-events-timeseries' && { limit: Number(value.limit) }),
  };
}

export function parsePlaygroundQuery(source: string): PlaygroundQuery {
  let value: unknown;

  try {
    value = JSON.parse(source);
  } catch {
    throw new Error('Enter valid JSON before running the query.');
  }

  return validatePlaygroundQuery(value);
}

export function serializePlaygroundQuery(query: PlaygroundQuery): string {
  return JSON.stringify(
    {
      queryType: query.queryType,
      granularity: query.granularity,
      ...(query.queryType === 'custom-events-timeseries' && { limit: query.limit }),
    },
    null,
    2,
  );
}

export function buildQueryRequest(query: PlaygroundQuery, scope: QueryScope): QueryRequest {
  const filters = Object.fromEntries(
    Object.entries(scope.filters ?? {}).filter(([, value]) => value !== undefined),
  );
  const params = {
    startAt: scope.startAt,
    endAt: scope.endAt,
    timezone: scope.timezone,
    unit: query.granularity,
    ...filters,
  };

  if (query.queryType === 'custom-events-timeseries') {
    return {
      path: `/websites/${scope.websiteId}/events/series`,
      params: { ...params, limit: query.limit },
    };
  }

  return {
    path: `/websites/${scope.websiteId}/pageviews`,
    params,
  };
}

export function normalizeTrafficResults(response: unknown): TrafficResultRow[] {
  if (!isRecord(response)) {
    throw new Error('The analytics API returned an unsupported response.');
  }

  const pageviews = Array.isArray(response.pageviews) ? response.pageviews : [];
  const sessions = Array.isArray(response.sessions) ? response.sessions : [];
  const rows = new Map<string, TrafficResultRow>();

  for (const point of pageviews) {
    if (!isRecord(point) || typeof point.x !== 'string' || typeof point.y !== 'number') continue;
    rows.set(point.x, {
      bucket: point.x,
      pageviews: point.y,
      sessions: rows.get(point.x)?.sessions ?? 0,
      pageviewsPerSession: null,
    });
  }

  for (const point of sessions) {
    if (!isRecord(point) || typeof point.x !== 'string' || typeof point.y !== 'number') continue;
    const current = rows.get(point.x);
    rows.set(point.x, {
      bucket: point.x,
      pageviews: current?.pageviews ?? 0,
      sessions: point.y,
      pageviewsPerSession: null,
    });
  }

  return [...rows.values()]
    .map(row => ({
      ...row,
      pageviewsPerSession: row.sessions > 0 ? row.pageviews / row.sessions : null,
    }))
    .sort((a, b) => a.bucket.localeCompare(b.bucket));
}

export function normalizeEventResults(response: unknown): EventResultRow[] {
  if (!Array.isArray(response)) {
    throw new Error('The analytics API returned an unsupported response.');
  }

  return response
    .filter(
      (row): row is { x: string; t: string; y: number } =>
        isRecord(row) &&
        typeof row.x === 'string' &&
        typeof row.t === 'string' &&
        typeof row.y === 'number',
    )
    .map(row => ({ bucket: row.t, eventName: row.x, signals: row.y }))
    .sort((a, b) => a.bucket.localeCompare(b.bucket) || a.eventName.localeCompare(b.eventName));
}
