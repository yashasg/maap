'use client';

import { Column, Grid, Heading, Text } from '@umami/react-zen';
import { PieChart } from '@/components/charts/PieChart';
import { LoadingPanel } from '@/components/common/LoadingPanel';
import { Panel } from '@/components/common/Panel';
import { usePropertyValuesQuery } from '@/components/hooks';
import { ListTable } from '@/components/metrics/ListTable';
import { CHART_COLORS } from '@/lib/constants';

interface LocalizationScreenProps {
  websiteId: string;
}

interface PropertyValue {
  value: string | null;
  total: number | string;
}

interface LocalizationMetric {
  title: string;
  description: string;
  propertyName: string;
  formatValue?: (value: string) => string;
}

export const LOCALIZATION_METRICS: LocalizationMetric[] = [
  {
    title: 'Preferred Language',
    description: 'The language selected in the user’s device preferences.',
    propertyName: 'TelemetryDeck.UserPreference.language',
  },
  {
    title: 'App Language',
    description: 'The language the app was running in when it sent the signal.',
    propertyName: 'TelemetryDeck.RunContext.language',
  },
  {
    title: 'Region',
    description: 'The configured device region reported by the SDK, not an IP-derived location.',
    propertyName: 'TelemetryDeck.UserPreference.region',
  },
  {
    title: 'Layout Direction',
    description: 'The interface direction reported by the SDK for each signal.',
    propertyName: 'TelemetryDeck.UserPreference.layoutDirection',
    formatValue: value =>
      (
        ({
          leftToRight: 'Left to right',
          rightToLeft: 'Right to left',
        }) as Record<string, string>
      )[value] ?? value,
  },
];

export default function LocalizationScreen({ websiteId }: LocalizationScreenProps) {
  return (
    <Column gap="6">
      <Column gap="2">
        <Heading size="3xl">Localization</Heading>
        <Text color="muted" style={{ maxWidth: '720px' }}>
          Compare the language, region, and layout settings reported by your app. Each card uses its
          own SDK field so these settings remain independent.
        </Text>
      </Column>

      <Grid columns={{ base: '1fr', xl: 'repeat(2, minmax(0, 1fr))' }} gap="3">
        {LOCALIZATION_METRICS.map(metric => (
          <LocalizationCard key={metric.propertyName} websiteId={websiteId} metric={metric} />
        ))}
      </Grid>
    </Column>
  );
}

function LocalizationCard({
  websiteId,
  metric,
}: {
  websiteId: string;
  metric: LocalizationMetric;
}) {
  const query = usePropertyValuesQuery('event', websiteId, metric.propertyName);
  const rows = normalizeValues(query.data, metric.formatValue);
  const mayHaveMoreValues = Array.isArray(query.data) && query.data.length >= 100;
  const total = rows.reduce((sum, row) => sum + row.count, 0);
  const chartData = {
    labels: rows.map(row => row.label),
    datasets: [
      {
        data: rows.map(row => row.count),
        backgroundColor: CHART_COLORS,
        borderWidth: 0,
      },
    ],
  };

  return (
    <Panel title={metric.title} minHeight="390px">
      <Text color="muted">{metric.description}</Text>
      <LoadingPanel
        data={rows}
        error={query.error}
        isLoading={query.isLoading}
        isFetching={query.isFetching}
        renderEmpty={() => <NotReported propertyName={metric.propertyName} />}
        minHeight="290px"
      >
        <Grid
          columns={{ base: '1fr', md: 'minmax(220px, 1fr) minmax(220px, 1fr)' }}
          gap="5"
          alignItems="center"
        >
          <PieChart type="doughnut" chartData={chartData} height="230px" />
          <ListTable
            title="Reported value"
            metric="Signals"
            itemCount={8}
            data={rows.map(row => ({
              ...row,
              percent: total ? (row.count / total) * 100 : 0,
            }))}
          />
          {mayHaveMoreValues && (
            <Text color="muted" style={{ gridColumn: '1 / -1' }}>
              Showing the 100 most frequently reported values.
            </Text>
          )}
        </Grid>
      </LoadingPanel>
    </Panel>
  );
}

function NotReported({ propertyName }: { propertyName: string }) {
  return (
    <Column alignItems="center" justifyContent="center" minHeight="260px" gap="2">
      <Heading size="lg">Not reported in this range</Heading>
      <Text color="muted" align="center">
        No selected signal reported <code>{propertyName}</code>.
      </Text>
    </Column>
  );
}

export function normalizeValues(
  values: PropertyValue[] | undefined,
  formatValue: (value: string) => string = value => value,
) {
  return (values ?? [])
    .flatMap(item => {
      const value = item.value?.trim();
      const count = Number(item.total);

      return value && Number.isFinite(count) && count > 0
        ? [{ label: formatValue(value), count }]
        : [];
    })
    .sort((a, b) => b.count - a.count || a.label.localeCompare(b.label));
}
