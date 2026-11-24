'use client';

import { Button, Column, Heading, Icon, Row, Text } from '@umami/react-zen';
import { useMemo } from 'react';
import { PieChart } from '@/components/charts/PieChart';
import { LoadingPanel } from '@/components/common/LoadingPanel';
import { Panel } from '@/components/common/Panel';
import { useAnalyticsQuery } from '@/components/hooks/queries/useAnalyticsQuery';
import { RefreshCw } from '@/components/icons';
import { CHART_COLORS } from '@/lib/constants';
import { formatLongNumber } from '@/lib/format';
import styles from './Screen.module.css';

const REPORTS = [
  { id: 'preferredContentSize', title: 'Preferred Content Size (Dynamic Type)' },
  { id: 'boldText', title: 'Bold Text' },
  { id: 'reduceMotion', title: 'Reduce Motion' },
  { id: 'reduceTransparency', title: 'Reduce Transparency' },
  { id: 'darkerSystemColors', title: 'Darker System Colors' },
  { id: 'differentiateWithoutColor', title: 'Differentiate Without Color' },
  { id: 'invertedColors', title: 'Inverted Colors' },
] as const;

interface AccessibilityValue {
  label: string;
  signals: number;
}

interface AccessibilityReport {
  id: (typeof REPORTS)[number]['id'];
  totalSignals: number;
  reportedSignals: number;
  values: AccessibilityValue[];
}

interface AccessibilityResponse {
  reports: AccessibilityReport[];
}

function AccessibilityCard({ title, report }: { title: string; report?: AccessibilityReport }) {
  const chartData = useMemo(() => {
    const chartValues =
      report?.values
        .map((value, index) => ({
          ...value,
          color: CHART_COLORS[index % CHART_COLORS.length],
        }))
        .filter(value => value.signals > 0) ?? [];

    return chartValues.length
      ? {
          labels: chartValues.map(value => value.label),
          datasets: [
            {
              data: chartValues.map(value => value.signals),
              backgroundColor: chartValues.map(value => value.color),
              borderWidth: 0,
            },
          ],
        }
      : null;
  }, [report]);

  return (
    <Panel className={styles.card} title={title}>
      {!report ? (
        <Column className={styles.state} alignItems="center" justifyContent="center" gap="2">
          <Text weight="bold">Report unavailable</Text>
          <Text color="muted" align="center">
            This aggregate was not returned for the selected app.
          </Text>
        </Column>
      ) : report.totalSignals === 0 ? (
        <Column className={styles.state} alignItems="center" justifyContent="center" gap="2">
          <Text weight="bold">No signals in this range</Text>
          <Text color="muted" align="center">
            Change the date range or Test Mode scope to check another interval.
          </Text>
        </Column>
      ) : (
        <>
          <div className={styles.breakdown}>
            {chartData && (
              <div className={styles.chart} aria-hidden="true">
                <PieChart type="doughnut" chartData={chartData} height="180px" />
              </div>
            )}
            <ul className={styles.legend} aria-label={`${title} breakdown`}>
              {report.values.map((value, index) => {
                const percent = report.totalSignals
                  ? (value.signals / report.totalSignals) * 100
                  : 0;
                return (
                  <li key={value.label} className={styles.legendRow}>
                    <span
                      className={styles.swatch}
                      style={{
                        backgroundColor: CHART_COLORS[index % CHART_COLORS.length],
                      }}
                      aria-hidden="true"
                    />
                    <span className={styles.label}>{value.label}</span>
                    <span className={styles.count}>
                      {formatLongNumber(value.signals)} · {percent.toFixed(1)}%
                    </span>
                  </li>
                );
              })}
            </ul>
          </div>
          <Text className={styles.footer} color="muted" size="sm">
            {formatLongNumber(report.reportedSignals)} of {formatLongNumber(report.totalSignals)}{' '}
            signals reported this setting
          </Text>
        </>
      )}
    </Panel>
  );
}

export default function AccessibilityScreen({ websiteId }: { websiteId: string }) {
  const query = useAnalyticsQuery<AccessibilityResponse>(
    'app-reports/accessibility',
    { websiteId },
    { staleTime: 60_000 },
  );
  const reports = useMemo(
    () => new Map(query.data?.reports.map(report => [report.id, report])),
    [query.data],
  );

  return (
    <section className={styles.screen} aria-labelledby="accessibility-title">
      <Row className={styles.header} alignItems="flex-start" justifyContent="space-between" gap="4">
        <Column className={styles.intro} gap="2">
          <Heading id="accessibility-title" size="3xl">
            Accessibility
          </Heading>
          <Text color="muted">
            Understand which accessibility settings were reported by signals from this app.
          </Text>
          <Text className={styles.note} color="muted" size="sm">
            These are aggregate signal counts, never individual user records. Not reported means the
            field was absent; it is not counted as Off.
          </Text>
        </Column>
        <Button
          size="sm"
          variant="quiet"
          aria-label="Refresh accessibility reports"
          isDisabled={query.isFetching}
          onPress={() => query.refetch()}
        >
          <Icon>
            <RefreshCw />
          </Icon>
          Refresh
        </Button>
      </Row>

      <LoadingPanel
        data={query.data?.reports}
        error={query.error}
        isLoading={query.isLoading}
        isFetching={query.isFetching && !query.data}
        minHeight="24rem"
      >
        <div className={styles.grid}>
          {REPORTS.map(report => (
            <AccessibilityCard
              key={report.id}
              title={report.title}
              report={reports.get(report.id)}
            />
          ))}
        </div>
      </LoadingPanel>
    </section>
  );
}
