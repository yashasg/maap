'use client';

import { Column, Grid, Heading, Text } from '@umami/react-zen';
import { PieChart } from '@/components/charts/PieChart';
import { LoadingPanel } from '@/components/common/LoadingPanel';
import { Panel } from '@/components/common/Panel';
import { ListTable } from '@/components/metrics/ListTable';
import { CHART_COLORS } from '@/lib/constants';
import { DeviceMetricSkeleton, DevicesLayout, deviceMetrics } from '../ReportSkeleton';
import { useDevicesReport } from './useDevicesReport';

interface DevicesScreenProps {
  websiteId: string;
}

interface DeviceReportRow {
  label: string;
  count: number;
}

export interface DeviceReportValue {
  value: string | null;
  signals: number;
}

export interface DeviceReport {
  id: string;
  totalSignals: number;
  reportedSignals: number;
  values: DeviceReportValue[];
}

interface DeviceMetric {
  title: string;
  description: string;
  visualization: 'list' | 'donut';
  rows: DeviceReportRow[];
}

export default function DevicesScreen({ websiteId }: DevicesScreenProps) {
  const query = useDevicesReport(websiteId);
  const report = (id: string) => query.data?.reports.find(item => item.id === id);
  const metrics: DeviceMetric[] = deviceMetrics.map(metric => ({
    ...metric,
    rows: normalizeDeviceReport(
      report(metric.id),
      ['screenWidths', 'screenHeights'].includes(metric.id)
        ? formatPoints
        : ['colorSchemes', 'orientations'].includes(metric.id)
          ? sentenceCase
          : undefined,
    ),
  }));

  return (
    <DevicesLayout>
      {metrics.map(metric => (
        <DeviceMetricCard key={metric.title} metric={metric} query={query} />
      ))}
    </DevicesLayout>
  );
}

function DeviceMetricCard({
  metric,
  query,
}: {
  metric: DeviceMetric;
  query: ReturnType<typeof useDevicesReport>;
}) {
  const total = metric.rows.reduce((sum, row) => sum + row.count, 0);
  const rows = metric.rows.map(row => ({
    ...row,
    percent: total ? (row.count / total) * 100 : 0,
  }));
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
        renderEmpty={() => <NotReported />}
        minHeight="290px"
        loadingContent={<DeviceMetricSkeleton visualization={metric.visualization} />}
      >
        {metric.visualization === 'donut' ? (
          <Grid
            columns={{ base: '1fr', md: 'minmax(220px, 1fr) minmax(220px, 1fr)' }}
            gap="5"
            alignItems="center"
          >
            <PieChart type="doughnut" chartData={chartData} height="230px" />
            <ListTable title="Reported value" metric="Signals" itemCount={8} data={rows} />
          </Grid>
        ) : (
          <ListTable title="Reported value" metric="Signals" itemCount={10} data={rows} />
        )}
      </LoadingPanel>
    </Panel>
  );
}

function NotReported() {
  return (
    <Column alignItems="center" justifyContent="center" minHeight="260px" gap="2">
      <Heading size="lg">Not reported in this range</Heading>
      <Text color="muted" align="center">
        No eligible signals were recorded for this dimension.
      </Text>
    </Column>
  );
}

function formatPoints(value: string) {
  const points = Number(value);

  return Number.isFinite(points) && points > 0 ? `${points.toLocaleString()} pt` : value;
}

function sentenceCase(value: string) {
  if (value === 'Not reported') return value;
  return value ? `${value[0].toLocaleUpperCase()}${value.slice(1)}` : value;
}

export function normalizeDeviceReport(
  report: DeviceReport | undefined,
  formatValue: (value: string) => string = value => value,
): DeviceReportRow[] {
  return (report?.values ?? [])
    .flatMap(({ value, signals }) => {
      const label = value?.trim() || 'Not reported';
      const count = Number(signals);

      return Number.isFinite(count) && count > 0 ? [{ label: formatValue(label), count }] : [];
    })
    .sort((a, b) => b.count - a.count || a.label.localeCompare(b.label));
}
