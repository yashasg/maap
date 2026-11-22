import { Column, Grid, Heading, Skeleton, SkeletonText, Text } from '@umami/react-zen';
import type { ReactNode } from 'react';
import { LoadingPanel } from '@/components/common/LoadingPanel';
import { PageHeader } from '@/components/common/PageHeader';
import { Panel } from '@/components/common/Panel';
import { useMessages } from '@/components/hooks';
import { ListTableSkeleton } from '@/components/metrics/ListTable';
import acquisition from './acquisition/Screen.module.css';
import overview from './overview/Overview.module.css';
import type { AppReport } from './sections';
import versions from './versions/Versions.module.css';

export const deviceMetrics = [
  {
    id: 'models',
    title: 'Models',
    description: 'Device model names reported on each signal.',
    visualization: 'list',
  },
  {
    id: 'systemVersions',
    title: 'System Versions',
    description: 'Full operating system versions reported by the SDK.',
    visualization: 'donut',
  },
  {
    id: 'types',
    title: 'Types',
    description: 'Technical hardware identifiers reported as model names.',
    visualization: 'list',
  },
  {
    id: 'platforms',
    title: 'Platforms',
    description: 'SDK-reported platforms such as iOS, macOS, Android, or visionOS.',
    visualization: 'donut',
  },
  {
    id: 'screenWidths',
    title: 'Screen Resolution Width',
    description: 'Reported screen width in points.',
    visualization: 'list',
  },
  {
    id: 'screenHeights',
    title: 'Screen Resolution Height',
    description: 'Reported screen height in points.',
    visualization: 'list',
  },
  {
    id: 'colorSchemes',
    title: 'Color Scheme',
    description: 'Light or dark appearance reported for each signal.',
    visualization: 'donut',
  },
  {
    id: 'orientations',
    title: 'Orientation',
    description: 'Device orientation reported by the SDK.',
    visualization: 'donut',
  },
] as const;

export function DevicesLayout({ children }: { children: ReactNode }) {
  return (
    <Column gap="6">
      <Column gap="2">
        <Heading size="3xl">Devices</Heading>
        <Text color="muted" style={{ maxWidth: '720px' }}>
          Understand the hardware, operating systems, display sizes, and appearance settings
          reported by your app.
        </Text>
      </Column>
      <Grid columns={{ base: '1fr', xl: 'repeat(2, minmax(0, 1fr))' }} gap="3">
        {children}
      </Grid>
    </Column>
  );
}

export function DeviceMetricSkeleton({ visualization }: { visualization: 'list' | 'donut' }) {
  const table = <ListTableSkeleton title="Reported value" metric="Signals" itemCount={7} />;
  return (
    <Column minHeight="290px" data-report-skeleton="device">
      {visualization === 'donut' ? (
        <Grid
          columns={{ base: '1fr', md: 'minmax(220px, 1fr) minmax(220px, 1fr)' }}
          gap="5"
          alignItems="center"
        >
          <Column height="230px" alignItems="center" justifyContent="center" aria-hidden>
            <Skeleton width="180px" height="180px" borderRadius="full" />
          </Column>
          {table}
        </Grid>
      ) : (
        table
      )}
    </Column>
  );
}

export function AcquisitionHeader() {
  return (
    <PageHeader
      title="Acquisition"
      description="See when identified app users first arrived and how those new users are distributed."
      showBorder={false}
    />
  );
}

export function AcquisitionSkeleton() {
  return (
    <Column
      gap="6"
      role="status"
      aria-label="Loading acquisition report"
      data-report-skeleton="acquisition"
    >
      <section className={acquisition.identity} aria-hidden>
        <Column gap="2" width="100%">
          <Skeleton width="160px" height="18px" />
          <Skeleton width="60%" height="21px" />
          <SkeletonText lines={2} />
        </Column>
        <div className={acquisition.summary}>
          {[0, 1].map(index => (
            <Column key={index} gap="2">
              <Skeleton width="108px" />
              <Skeleton width="60px" height="30px" />
            </Column>
          ))}
        </div>
      </section>
      {[8, 6, 2, 2].map((count, group) => (
        <Column gap="3" key={group} aria-hidden>
          {group > 0 && <Skeleton width="240px" height="24px" />}
          <div className={acquisition.grid}>
            {Array.from({ length: count }, (_, index) => (
              <article className={acquisition.card} key={index}>
                <div className={acquisition.cardHeader}>
                  <SkeletonText lines={2} />
                </div>
                <Skeleton height="178px" />
              </article>
            ))}
          </div>
        </Column>
      ))}
    </Column>
  );
}

export function VersionSkeleton() {
  const { t, labels } = useMessages();
  return (
    <div
      className={versions.list}
      role="status"
      aria-label={t(labels.loading)}
      data-report-skeleton="versions"
    >
      {[0, 1, 2, 3].map(index => (
        <div className={versions.row} key={index} aria-hidden>
          <div className={versions.rowHeader}>
            <Skeleton width="30%" height="21px" />
            <Skeleton width="45%" height="21px" />
          </div>
          <Skeleton height="8px" />
        </div>
      ))}
    </div>
  );
}

export function OverviewSkeleton() {
  const { t, labels } = useMessages();
  return (
    <div
      className={overview.screen}
      role="status"
      aria-label={t(labels.loading)}
      data-report-skeleton="overview"
    >
      <Panel className={overview.dailyPanel} aria-hidden>
        <SkeletonText lines={2} />
        <Skeleton height="14rem" />
      </Panel>
      <div className={overview.metricStrip} aria-hidden>
        {[0, 1, 2].map(index => (
          <div className={overview.fact} key={index}>
            <Skeleton width="70%" />
            <Skeleton width="40%" height="35px" />
            <Skeleton width="50%" />
          </div>
        ))}
      </div>
      <div className={overview.detailGrid} aria-hidden>
        {[0, 1, 2].map(index => (
          <Panel className={overview.detailPanel} key={index}>
            <Skeleton width="50%" height="24px" />
            <VersionSkeleton />
          </Panel>
        ))}
      </div>
    </div>
  );
}

export function ReportSkeleton({ screen }: { screen: AppReport }) {
  if (screen === 'overview') return <OverviewSkeleton />;
  if (screen === 'acquisition') {
    return (
      <Column gap="5" minWidth="0">
        <AcquisitionHeader />
        <AcquisitionSkeleton />
      </Column>
    );
  }
  if (screen === 'devices') {
    return (
      <DevicesLayout>
        {deviceMetrics.map(metric => (
          <Panel key={metric.id} title={metric.title} minHeight="390px">
            <Text color="muted">{metric.description}</Text>
            <DeviceMetricSkeleton visualization={metric.visualization} />
          </Panel>
        ))}
      </DevicesLayout>
    );
  }
  if (screen === 'versions') {
    return (
      <section className={versions.screen}>
        <Column className={versions.intro} gap="2">
          <Heading size="3xl">Versions</Heading>
          <Text color="muted">
            See which app, build, and SDK versions generated events in the selected date range and
            Test Mode scope.
          </Text>
        </Column>
        <div className={versions.panels}>
          {['App Versions', 'Build Numbers', 'SDK Versions'].map(title => (
            <Panel className={versions.panel} title={title} key={title}>
              <VersionSkeleton />
            </Panel>
          ))}
        </div>
      </section>
    );
  }
  return (
    <LoadingPanel isLoading minHeight="400px">
      {null}
    </LoadingPanel>
  );
}
