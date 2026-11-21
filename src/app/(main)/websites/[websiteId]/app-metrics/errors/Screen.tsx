'use client';

import { Button, Column, Grid, Icon, Row, Text } from '@umami/react-zen';
import { useCallback, useMemo } from 'react';
import { BarChart } from '@/components/charts/BarChart';
import { LinkButton } from '@/components/common/LinkButton';
import { LoadingPanel } from '@/components/common/LoadingPanel';
import { PageHeader } from '@/components/common/PageHeader';
import { useDateParameters, useLocale, usePropertySeriesQuery } from '@/components/hooks';
import { ExternalLink, RefreshCw } from '@/components/icons';
import { ListTable } from '@/components/metrics/ListTable';
import { renderDateLabels } from '@/lib/charts';
import { CHART_COLORS, DATA_TYPE, OPERATORS } from '@/lib/constants';
import { generateTimeSeries } from '@/lib/date';
import type { EventDataSeriesPoint, PropertyFilter } from '@/lib/types';

const ERROR_EVENT = 'TelemetryDeck.Error.occurred';
const ERROR_ID_PROPERTY = 'TelemetryDeck.Error.id';
const ERROR_CATEGORY_PROPERTY = 'TelemetryDeck.Error.category';
const ERROR_DOCUMENTATION = 'https://telemetrydeck.com/docs/articles/preset-errors/';

const ERROR_REPORTS = [
  { key: 'overall', label: 'Errors (overall)' },
  { key: 'thrown-exception', label: 'Thrown Exception Errors' },
  { key: 'user-input', label: 'User Input Errors' },
  { key: 'app-state', label: 'App State Errors' },
] as const;

type ErrorCategory = (typeof ERROR_REPORTS)[number]['key'];

function categoryFilters(category: ErrorCategory): PropertyFilter[] {
  if (category === 'overall') return [];

  return [
    {
      propertyName: ERROR_CATEGORY_PROPERTY,
      dataType: DATA_TYPE.string,
      operator: OPERATORS.equals,
      value: category,
    },
  ];
}

export function buildErrorReport(rows: EventDataSeriesPoint[] = []) {
  const frequency = new Map<string, number>();
  const history = new Map<string, number>();

  for (const row of rows) {
    const count = Number(row.y);
    if (!row.x || !row.t || !Number.isFinite(count) || count <= 0) continue;
    frequency.set(row.x, (frequency.get(row.x) ?? 0) + count);
    history.set(row.t, (history.get(row.t) ?? 0) + count);
  }

  const total = [...frequency.values()].reduce((sum, count) => sum + count, 0);

  return {
    total,
    frequency: [...frequency]
      .map(([label, count]) => ({
        label,
        count,
        percent: total ? (count / total) * 100 : 0,
      }))
      .sort((a, b) => b.count - a.count || a.label.localeCompare(b.label)),
    history: [...history].map(([x, y]) => ({ x, y })).sort((a, b) => a.x.localeCompare(b.x)),
  };
}

export default function ErrorsScreen({ websiteId }: { websiteId: string }) {
  const overall = useNativeErrorQuery(websiteId, 'overall');
  const thrownException = useNativeErrorQuery(websiteId, 'thrown-exception');
  const userInput = useNativeErrorQuery(websiteId, 'user-input');
  const appState = useNativeErrorQuery(websiteId, 'app-state');
  const reports = [
    { definition: ERROR_REPORTS[0], query: overall },
    { definition: ERROR_REPORTS[1], query: thrownException },
    { definition: ERROR_REPORTS[2], query: userInput },
    { definition: ERROR_REPORTS[3], query: appState },
  ];
  const isRefreshing = reports.some(({ query }) => query.isFetching);

  const refresh = () => {
    for (const { query } of reports) void query.refetch();
  };

  return (
    <Column gap="4" minWidth="0">
      <PageHeader
        title="Errors"
        description="Find the native app errors reported most often and see when they occurred."
        showBorder={false}
      >
        <Row gap="2" wrap="wrap">
          <Button variant="quiet" isDisabled={isRefreshing} onPress={refresh}>
            <Icon>
              <RefreshCw />
            </Icon>
            <Text>{isRefreshing ? 'Refreshing' : 'Refresh'}</Text>
          </Button>
          <LinkButton href={ERROR_DOCUMENTATION} target="_blank" asAnchor variant="outline">
            <Icon>
              <ExternalLink />
            </Icon>
            <Text>Open documentation</Text>
          </LinkButton>
        </Row>
      </PageHeader>

      <Text color="muted">
        Overall includes every <strong>{ERROR_EVENT}</strong> signal with a stored error ID,
        including uncategorized errors. Category reports use the SDK values{' '}
        <strong>thrown-exception</strong>, <strong>user-input</strong>, and{' '}
        <strong>app-state</strong>. Browser JavaScript issue reports are not included.
      </Text>

      <Column gap="4">
        {reports.map(({ definition, query }) => (
          <ErrorReportPair key={definition.key} label={definition.label} query={query} />
        ))}
      </Column>
    </Column>
  );
}

function useNativeErrorQuery(websiteId: string, category: ErrorCategory) {
  return usePropertySeriesQuery(
    'event',
    websiteId,
    ERROR_ID_PROPERTY,
    categoryFilters(category),
    ERROR_EVENT,
  );
}

function ErrorReportPair({
  label,
  query,
}: {
  label: string;
  query: ReturnType<typeof useNativeErrorQuery>;
}) {
  const { startAt, endAt, unit } = useDateParameters();
  const { locale, dateLocale } = useLocale();
  const startDate = useMemo(() => new Date(startAt), [startAt]);
  const endDate = useMemo(() => new Date(endAt), [endAt]);
  const report = useMemo(() => buildErrorReport(query.data), [query.data]);
  const renderXLabel = useCallback(renderDateLabels(unit, locale), [locale, unit]);
  const chartData = useMemo(
    () => ({
      datasets: [
        {
          label: 'occurrences',
          data: generateTimeSeries(report.history, startDate, endDate, unit, dateLocale),
          backgroundColor: CHART_COLORS[0],
          borderColor: CHART_COLORS[0],
          borderWidth: 1,
        },
      ],
    }),
    [dateLocale, endDate, report.history, startDate, unit],
  );
  const emptyMessage = `No ${label.toLowerCase()} with a stored error ID were reported in this range.`;

  return (
    <Grid columns={{ base: '1fr', lg: 'minmax(0, 1fr) minmax(0, 1fr)' }} gap="4">
      <ReportPanel
        title={`Most Frequent ${label}`}
        total={report.total}
        query={query}
        emptyMessage={emptyMessage}
      >
        <ListTable
          data={report.frequency}
          title="Error ID"
          metric="Occurrences"
          animate={false}
          virtualize={report.frequency.length > 10}
          itemCount={10}
        />
      </ReportPanel>

      <ReportPanel
        title={`History of ${label}`}
        total={report.total}
        query={query}
        emptyMessage={emptyMessage}
      >
        <BarChart
          chartData={chartData}
          minDate={startDate}
          maxDate={endDate}
          unit={unit}
          renderXLabel={renderXLabel}
          height="280px"
        />
      </ReportPanel>
    </Grid>
  );
}

function ReportPanel({
  title,
  total,
  query,
  emptyMessage,
  children,
}: {
  title: string;
  total: number;
  query: ReturnType<typeof useNativeErrorQuery>;
  emptyMessage: string;
  children: React.ReactNode;
}) {
  return (
    <Column
      border
      borderRadius
      backgroundColor="surface"
      minHeight="360px"
      minWidth="0"
      overflow="hidden"
    >
      <Row alignItems="center" justifyContent="space-between" gap="3" padding="4" border="bottom">
        <Text weight="bold">{title}</Text>
        {!query.isLoading && !query.error && (
          <Text color="muted" size="sm">
            {total.toLocaleString()} {total === 1 ? 'occurrence' : 'occurrences'}
          </Text>
        )}
      </Row>
      <LoadingPanel
        data={query.data}
        error={query.error}
        isLoading={query.isLoading}
        isFetching={query.isFetching && !query.data}
        minHeight="300px"
        padding="4"
        renderEmpty={() => (
          <Column alignItems="center" justifyContent="center" minHeight="280px" padding="4">
            <Text color="muted" align="center">
              {emptyMessage}
            </Text>
          </Column>
        )}
      >
        {children}
      </LoadingPanel>
    </Column>
  );
}
