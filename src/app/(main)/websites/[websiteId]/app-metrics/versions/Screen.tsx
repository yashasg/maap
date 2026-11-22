'use client';

import { Button, Column, Heading, Icon, Row, Text } from '@umami/react-zen';
import { LoadingPanel } from '@/components/common/LoadingPanel';
import { Panel } from '@/components/common/Panel';
import { useAnalyticsQuery } from '@/components/hooks/queries/useAnalyticsQuery';
import { RefreshCw } from '@/components/icons';
import { formatLongNumber } from '@/lib/format';
import { VersionSkeleton } from '../ReportSkeleton';
import styles from './Versions.module.css';

type Dimension = 'appVersion' | 'buildNumber' | 'sdkVersion';

interface VersionValue {
  value: string | null;
  events: number;
}

interface VersionBreakdown {
  dimension: Dimension;
  totalEvents: number;
  reportedEvents: number;
  values: VersionValue[];
}

interface VersionPanelProps {
  websiteId: string;
  dimension: Dimension;
  title: string;
}

function VersionPanel({ websiteId, dimension, title }: VersionPanelProps) {
  const query = useAnalyticsQuery<VersionBreakdown>(
    'app-reports/versions',
    { websiteId, dimension },
    { staleTime: 60_000 },
  );
  const data = query.data;

  return (
    <Panel className={styles.panel} title={title}>
      <LoadingPanel
        data={data?.values}
        error={query.error}
        isLoading={query.isLoading}
        isFetching={query.isFetching && !data}
        minHeight="14rem"
        loadingContent={<VersionSkeleton />}
        renderEmpty={() => (
          <Column alignItems="center" justifyContent="center" minHeight="14rem" gap="2">
            <Text weight="bold">No version data</Text>
            <Text color="muted" align="center">
              No events were recorded for this app in the selected scope.
            </Text>
          </Column>
        )}
      >
        <div className={styles.list} aria-label={`${title} by event`}>
          {data?.values.map(item => {
            const percent = data.totalEvents ? (item.events / data.totalEvents) * 100 : 0;
            const label = item.value || 'Not reported';
            return (
              <div className={styles.row} key={item.value ?? '__not-reported'}>
                <div className={styles.rowHeader}>
                  <span className={styles.label} title={label}>
                    {label}
                  </span>
                  <span className={styles.value}>
                    {formatLongNumber(item.events)} events · {percent.toFixed(1)}%
                  </span>
                </div>
                <div
                  className={styles.track}
                  role="progressbar"
                  aria-label={`${label}: ${item.events} events`}
                  aria-valuemin={0}
                  aria-valuemax={data.totalEvents}
                  aria-valuenow={item.events}
                >
                  <div className={styles.bar} style={{ width: `${percent}%` }} />
                </div>
              </div>
            );
          })}
        </div>
      </LoadingPanel>
      {data && (
        <Row className={styles.footer} alignItems="center" justifyContent="space-between" gap="2">
          <Text color="muted" size="sm">
            {formatLongNumber(data.reportedEvents)} of {formatLongNumber(data.totalEvents)} events
            reported a value
          </Text>
          <Button
            size="sm"
            variant="quiet"
            aria-label={`Refresh ${title}`}
            isDisabled={query.isFetching}
            onPress={() => query.refetch()}
          >
            <Icon>
              <RefreshCw />
            </Icon>
            Refresh
          </Button>
        </Row>
      )}
    </Panel>
  );
}

export default function VersionsScreen({ websiteId }: { websiteId: string }) {
  return (
    <section className={styles.screen} aria-labelledby="versions-title">
      <Column className={styles.intro} gap="2">
        <Heading id="versions-title" size="3xl">
          Versions
        </Heading>
        <Text color="muted">
          See which app, build, and SDK versions generated events in the selected date range and
          Test Mode scope.
        </Text>
      </Column>
      <div className={styles.panels}>
        <VersionPanel websiteId={websiteId} dimension="appVersion" title="App Versions" />
        <VersionPanel websiteId={websiteId} dimension="buildNumber" title="Build Numbers" />
        <VersionPanel websiteId={websiteId} dimension="sdkVersion" title="SDK Versions" />
      </div>
    </section>
  );
}
