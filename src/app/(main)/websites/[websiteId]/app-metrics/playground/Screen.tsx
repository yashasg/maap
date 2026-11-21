'use client';

import { Heading, Text } from '@umami/react-zen';
import { useEffect, useState } from 'react';
import { Panel } from '@/components/common/Panel';
import { useApi } from '@/components/hooks/useApi';
import { useDateParameters } from '@/components/hooks/useDateParameters';
import { useFilterParameters } from '@/components/hooks/useFilterParameters';
import {
  buildQueryRequest,
  GRANULARITIES,
  normalizeEventResults,
  normalizeTrafficResults,
  type PlaygroundGranularity,
  type PlaygroundQuery,
  type PlaygroundQueryType,
  parsePlaygroundQuery,
  serializePlaygroundQuery,
  validatePlaygroundQuery,
} from './query';
import styles from './Screen.module.css';

type EditorMode = 'visual' | 'json';
type QueryResult =
  | { type: 'traffic'; rows: ReturnType<typeof normalizeTrafficResults> }
  | { type: 'events'; rows: ReturnType<typeof normalizeEventResults> };

const DEFAULT_QUERY: PlaygroundQuery = {
  queryType: 'traffic-timeseries',
  granularity: 'day',
};

function formatBucket(value: string, timezone: string) {
  const isDateOnly = /^\d{4}-\d{2}-\d{2}$/.test(value);
  const date = new Date(isDateOnly ? `${value}T00:00:00.000Z` : value);
  return Number.isNaN(date.valueOf())
    ? value
    : new Intl.DateTimeFormat(undefined, {
        dateStyle: 'medium',
        timeStyle: isDateOnly ? undefined : 'short',
        timeZone: isDateOnly ? 'UTC' : timezone,
      }).format(date);
}

function formatRatio(value: number | null) {
  return value === null ? '—' : value.toLocaleString(undefined, { maximumFractionDigits: 2 });
}

export default function PlaygroundScreen({ websiteId }: { websiteId: string }) {
  const { get } = useApi();
  const { startAt, endAt, timezone, unit } = useDateParameters();
  const filters = useFilterParameters({ includePagination: false });
  const [query, setQuery] = useState<PlaygroundQuery>(() => {
    return {
      ...DEFAULT_QUERY,
      granularity: GRANULARITIES.includes(unit as PlaygroundGranularity)
        ? (unit as PlaygroundGranularity)
        : DEFAULT_QUERY.granularity,
    };
  });
  const [mode, setMode] = useState<EditorMode>('visual');
  const [jsonSource, setJsonSource] = useState(() => serializePlaygroundQuery(query));
  const [result, setResult] = useState<QueryResult | null>(null);
  const [error, setError] = useState('');
  const [isRunning, setIsRunning] = useState(false);
  const [updatedAt, setUpdatedAt] = useState<Date | null>(null);
  const filterKey = JSON.stringify(filters);

  useEffect(() => {
    setResult(null);
    setUpdatedAt(null);
  }, [startAt, endAt, timezone, filterKey]);

  const handleModeChange = (nextMode: EditorMode) => {
    if (nextMode === 'json') {
      setJsonSource(serializePlaygroundQuery(query));
    }
    setError('');
    setMode(nextMode);
  };

  const handleQueryTypeChange = (queryType: PlaygroundQueryType) => {
    setQuery(current =>
      queryType === 'custom-events-timeseries'
        ? { ...current, queryType, limit: current.limit ?? 10 }
        : { queryType, granularity: current.granularity },
    );
    setResult(null);
  };

  const runQuery = async () => {
    setError('');
    setIsRunning(true);

    try {
      const nextQuery =
        mode === 'json' ? parsePlaygroundQuery(jsonSource) : validatePlaygroundQuery(query);
      const request = buildQueryRequest(nextQuery, {
        websiteId,
        startAt,
        endAt,
        timezone,
        filters,
      });
      const response = await get(request.path, request.params);
      const nextResult =
        nextQuery.queryType === 'traffic-timeseries'
          ? ({ type: 'traffic', rows: normalizeTrafficResults(response) } as const)
          : ({ type: 'events', rows: normalizeEventResults(response) } as const);

      setQuery(nextQuery);
      setJsonSource(serializePlaygroundQuery(nextQuery));
      setResult(nextResult);
      setUpdatedAt(new Date());
    } catch (queryError) {
      setResult(null);
      setUpdatedAt(null);
      setError(queryError instanceof Error ? queryError.message : 'The query could not be run.');
    } finally {
      setIsRunning(false);
    }
  };

  return (
    <section className={styles.screen}>
      <header className={styles.heading}>
        <Heading size="2xl">Playground</Heading>
        <Text color="muted">
          Compose a bounded analytics query and inspect the authorized result for this app.
        </Text>
      </header>

      <div className={styles.editorTabs} role="tablist" aria-label="Query editor mode">
        <button
          className={styles.tab}
          type="button"
          role="tab"
          aria-selected={mode === 'visual'}
          aria-controls="playground-editor"
          onClick={() => handleModeChange('visual')}
        >
          Visual editor
        </button>
        <button
          className={styles.tab}
          type="button"
          role="tab"
          aria-selected={mode === 'json'}
          aria-controls="playground-editor"
          onClick={() => handleModeChange('json')}
        >
          JSON editor
        </button>
      </div>

      <Panel title="Query">
        <div id="playground-editor" className={styles.editor} role="tabpanel">
          <ol className={styles.scope} aria-label="Query scope">
            <li>
              <strong>1</strong> Authorized app
            </li>
            <li>
              <strong>2</strong> Shared date range
            </li>
            <li>
              <strong>3</strong> Read-only aggregate
            </li>
            <li>
              <strong>4</strong> Bounded result
            </li>
          </ol>

          {mode === 'visual' ? (
            <div className={styles.fields}>
              <div className={styles.field}>
                <label htmlFor="playground-query-type">Query type</label>
                <select
                  id="playground-query-type"
                  className={styles.select}
                  value={query.queryType}
                  onChange={event =>
                    handleQueryTypeChange(event.currentTarget.value as PlaygroundQueryType)
                  }
                >
                  <option value="traffic-timeseries">Traffic time series</option>
                  <option value="custom-events-timeseries">Custom event time series</option>
                </select>
                <span className={styles.help}>
                  {query.queryType === 'traffic-timeseries'
                    ? 'Uses the existing pageviews report and returns pageviews and sessions.'
                    : 'Uses the existing custom-events series and returns signal counts by name.'}
                </span>
              </div>

              <div className={styles.field}>
                <label htmlFor="playground-granularity">Granularity</label>
                <select
                  id="playground-granularity"
                  className={styles.select}
                  value={query.granularity}
                  onChange={event => {
                    setQuery(current => ({
                      ...current,
                      granularity: event.currentTarget.value as PlaygroundGranularity,
                    }));
                    setResult(null);
                  }}
                >
                  {GRANULARITIES.map(granularity => (
                    <option key={granularity} value={granularity}>
                      {granularity}
                    </option>
                  ))}
                </select>
                <span className={styles.help}>
                  Buckets use the timezone selected in the shared report controls.
                </span>
              </div>

              {query.queryType === 'custom-events-timeseries' && (
                <div className={styles.field}>
                  <label htmlFor="playground-limit">Event names</label>
                  <input
                    id="playground-limit"
                    className={styles.numberInput}
                    type="number"
                    min={1}
                    max={50}
                    step={1}
                    value={query.limit ?? 10}
                    onChange={event => {
                      setQuery(current => ({
                        ...current,
                        limit: Number(event.currentTarget.value),
                      }));
                      setResult(null);
                    }}
                  />
                  <span className={styles.help}>Between 1 and 50 event names.</span>
                </div>
              )}
            </div>
          ) : (
            <div className={styles.field}>
              <label htmlFor="playground-json">Structured query</label>
              <textarea
                id="playground-json"
                className={styles.jsonEditor}
                value={jsonSource}
                spellCheck={false}
                onChange={event => {
                  setJsonSource(event.currentTarget.value);
                  setResult(null);
                }}
              />
              <span className={styles.help}>
                Accepted keys: queryType, granularity, and limit. Unknown keys are rejected.
              </span>
            </div>
          )}

          <div className={styles.notice}>
            <strong>Supported:</strong> app-scoped traffic and custom-event time series. The shared
            date range, timezone, and Test Mode selection are applied when the query runs.{' '}
            <strong>Not supported:</strong> SQL, scripts, mutations, joins, schema access, or custom
            authorization and date overrides.
          </div>

          {error && (
            <div className={styles.error} role="alert">
              {error}
            </div>
          )}

          <div className={styles.actions}>
            <span className={styles.help}>
              Results come directly from Maap&apos;s existing authorized analytics APIs.
            </span>
            <button
              className={styles.runButton}
              type="button"
              disabled={isRunning}
              onClick={runQuery}
            >
              {isRunning ? 'Running query…' : 'Run query'}
            </button>
          </div>
        </div>
      </Panel>

      <Panel title="Query result">
        <div aria-live="polite">
          {!result ? (
            <div className={styles.empty}>Run the structured query to inspect its result.</div>
          ) : result.rows.length === 0 ? (
            <div className={styles.empty}>
              The query completed successfully, but no matching analytics were recorded.
            </div>
          ) : (
            <div className={styles.tableWrap}>
              {result.type === 'traffic' ? (
                <table className={styles.table}>
                  <thead>
                    <tr>
                      <th scope="col">Bucket</th>
                      <th scope="col">Pageviews</th>
                      <th scope="col">Sessions</th>
                      <th scope="col">Pageviews per session</th>
                    </tr>
                  </thead>
                  <tbody>
                    {result.rows.map(row => (
                      <tr key={row.bucket}>
                        <td>{formatBucket(row.bucket, timezone)}</td>
                        <td>{row.pageviews.toLocaleString()}</td>
                        <td>{row.sessions.toLocaleString()}</td>
                        <td>{formatRatio(row.pageviewsPerSession)}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              ) : (
                <table className={`${styles.table} ${styles.events}`}>
                  <thead>
                    <tr>
                      <th scope="col">Bucket</th>
                      <th scope="col">Event name</th>
                      <th scope="col">Signals</th>
                    </tr>
                  </thead>
                  <tbody>
                    {result.rows.map(row => (
                      <tr key={`${row.bucket}:${row.eventName}`}>
                        <td>{formatBucket(row.bucket, timezone)}</td>
                        <td>{row.eventName}</td>
                        <td>{row.signals.toLocaleString()}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </div>
          )}
        </div>

        <footer className={styles.resultFooter}>
          <span>
            {result
              ? `${result.rows.length.toLocaleString()} result ${
                  result.rows.length === 1 ? 'row' : 'rows'
                }`
              : 'No query run yet'}
          </span>
          <span>
            {updatedAt
              ? `Updated ${updatedAt.toLocaleTimeString(undefined, {
                  hour: 'numeric',
                  minute: '2-digit',
                  second: '2-digit',
                })}`
              : 'Read-only'}
          </span>
        </footer>
      </Panel>
    </section>
  );
}
