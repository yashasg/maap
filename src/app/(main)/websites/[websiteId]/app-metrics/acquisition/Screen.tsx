'use client';

import { useQuery } from '@tanstack/react-query';
import { Button, Column, Heading, Icon, Text } from '@umami/react-zen';
import { useMemo } from 'react';
import { useApi } from '@/components/hooks/useApi';
import { useDateParameters } from '@/components/hooks/useDateParameters';
import { useNavigation } from '@/components/hooks/useNavigation';
import { AlertTriangle, RefreshCw } from '@/components/icons';
import { CHART_COLORS } from '@/lib/constants';
import { AcquisitionHeader, AcquisitionSkeleton } from '../ReportSkeleton';
import styles from './Screen.module.css';

export interface AcquisitionSeriesPoint {
  bucket: string;
  activeUsers: number;
  newUsers: number;
}

export interface AcquisitionCountPoint {
  bucket: string;
  users: number;
}

export interface AcquisitionBreakdownPoint {
  name: string;
  users: number;
}

export interface AcquisitionReport {
  identity: {
    unit: 'appUser';
    source: 'clientUser';
    unidentifiedEventsExcluded: boolean;
  };
  range: {
    startAt: string;
    endAt: string;
    timezone: string;
    testMode: boolean;
    totalSignals: number;
    identifiedSignals: number;
  };
  series: {
    hourly: AcquisitionSeriesPoint[];
    daily: AcquisitionSeriesPoint[];
    weekly: AcquisitionSeriesPoint[];
    monthly: AcquisitionSeriesPoint[];
  };
  patterns: {
    hourOfDay: AcquisitionCountPoint[];
    hourOfDay4Weeks: AcquisitionCountPoint[];
    dayOfWeek: AcquisitionCountPoint[];
    dayOfWeek4Weeks: AcquisitionCountPoint[];
    weekpart: AcquisitionCountPoint[];
    weekpart4Weeks: AcquisitionCountPoint[];
  };
  breakdowns: {
    device: AcquisitionBreakdownPoint[];
    platform: AcquisitionBreakdownPoint[];
    region: AcquisitionBreakdownPoint[];
    language: AcquisitionBreakdownPoint[];
  };
  availability: {
    userLocalTime: boolean;
    reason?: string;
  };
  query: {
    milliseconds: number;
    grains: Record<string, { unit: string; window: string }>;
  };
}

const number = new Intl.NumberFormat();
const WEEKDAYS: Record<string, string> = {
  '1': 'Mon',
  '2': 'Tue',
  '3': 'Wed',
  '4': 'Thu',
  '5': 'Fri',
  '6': 'Sat',
  '7': 'Sun',
};

export default function AcquisitionScreen({ websiteId }: { websiteId: string }) {
  const { get } = useApi();
  const { startAt, endAt, timezone } = useDateParameters();
  const { query: urlQuery } = useNavigation();
  const testMode =
    urlQuery.testMode === 'true' || urlQuery.tag === 'test' || urlQuery.tag === 'eq.test';
  const query = useQuery<AcquisitionReport>({
    queryKey: ['app-report-acquisition', { websiteId, startAt, endAt, timezone, testMode }],
    queryFn: () =>
      get(`/websites/${websiteId}/app-reports/acquisition`, {
        startAt,
        endAt,
        timezone,
        testMode: testMode ? 'true' : undefined,
      }),
    enabled: Boolean(websiteId),
  });

  return (
    <Column gap="5" minWidth="0">
      <AcquisitionHeader />

      {query.isLoading ? (
        <AcquisitionSkeleton />
      ) : query.error ? (
        <ErrorState onRetry={() => query.refetch()} />
      ) : query.data ? (
        <Report report={query.data} />
      ) : null}
    </Column>
  );
}

function Report({ report }: { report: AcquisitionReport }) {
  if (report.range.totalSignals === 0) {
    return (
      <StateCard
        title="No signals in this range"
        body="Try a wider date range or switch Test Mode to view a different signal set."
      />
    );
  }

  if (report.range.identifiedSignals === 0) {
    return (
      <StateCard
        title="New users cannot be counted"
        body="Signals exist in this range, but none include an SDK clientUser identity. Maap will not substitute sessions or device fingerprints for app users."
      />
    );
  }

  return (
    <Column gap="6">
      <section className={styles.identity} aria-labelledby="acquisition-method">
        <div>
          <span className={styles.status}>First-seen app users</span>
          <Heading id="acquisition-method" size="sm">
            What Maap counts as a new user
          </Heading>
          <Text color="muted">
            The first stored signal for each SDK <code>clientUser</code>, including history before
            the selected range. Unidentified signals are excluded rather than counted as sessions.
          </Text>
        </div>
        <dl className={styles.summary}>
          <div>
            <dt>Identity coverage</dt>
            <dd>
              {Math.round(
                (report.range.identifiedSignals / Math.max(1, report.range.totalSignals)) * 100,
              )}
              %
            </dd>
          </div>
          <div>
            <dt>Query time</dt>
            <dd>{Math.round(report.query.milliseconds)} ms</dd>
          </div>
        </dl>
      </section>

      {(
        [
          ['hourly', 'Hourly', 'last 24h'],
          ['daily', 'Daily', 'selected range'],
          ['weekly', 'Weekly', 'past 3 months'],
          ['monthly', 'Monthly', 'past year'],
        ] as const
      ).map(([key, label, window]) => (
        <CardGrid key={key}>
          <SeriesCard
            title={`${label} New Users`}
            subtitle={window}
            points={report.series[key]}
            values={['newUsers']}
          />
          <SeriesCard
            title={`${label} Active vs New Users`}
            subtitle={window}
            points={report.series[key]}
            values={['activeUsers', 'newUsers']}
          />
        </CardGrid>
      ))}

      <ReportSection title="Acquisition Time Patterns">
        <CardGrid>
          <UnavailableCard
            title="New Users by Hour of Day"
            subtitle="User-local time"
            reason={report.availability.reason}
          />
          <CountSeriesCard
            title="New Users by Hour of Day"
            subtitle={`Past 4 weeks · ${report.range.timezone}`}
            points={report.patterns.hourOfDay4Weeks}
            labels={hourLabels}
          />
          <UnavailableCard
            title="New Users by Day of Week"
            subtitle="User-local time"
            reason={report.availability.reason}
          />
          <CountSeriesCard
            title="New Users by Day of Week"
            subtitle={`Past 4 weeks · ${report.range.timezone}`}
            points={report.patterns.dayOfWeek4Weeks}
            labels={bucket => WEEKDAYS[bucket] || bucket}
          />
          <DonutCard
            title="New Users on Weekends vs Weekdays"
            points={report.patterns.weekpart.map(({ bucket, users }) => ({
              name: bucket,
              users,
            }))}
          />
          <CountSeriesCard
            title="New Users on Weekends vs Weekdays"
            subtitle="Past 4 weeks"
            points={report.patterns.weekpart4Weeks}
          />
        </CardGrid>
      </ReportSection>

      <ReportSection title="Device & Platform Distribution">
        <CardGrid>
          <DonutCard title="New Users by Device Type" points={report.breakdowns.device} />
          <DonutCard title="New Users by Platform" points={report.breakdowns.platform} />
        </CardGrid>
      </ReportSection>

      <ReportSection title="Geographic & Language Distribution">
        <CardGrid>
          <DonutCard title="New Users by Preferred Region" points={report.breakdowns.region} />
          <DonutCard title="New Users by Preferred Language" points={report.breakdowns.language} />
        </CardGrid>
      </ReportSection>
    </Column>
  );
}

function ReportSection({ title, children }: { title: string; children: React.ReactNode }) {
  const id = title.replaceAll(' ', '-').replace('&', 'and').toLowerCase();

  return (
    <section aria-labelledby={id}>
      <Heading id={id} size="sm" className={styles.sectionTitle}>
        {title}
      </Heading>
      {children}
    </section>
  );
}

function CardGrid({ children }: { children: React.ReactNode }) {
  return <div className={styles.grid}>{children}</div>;
}

function SeriesCard({
  title,
  subtitle,
  points,
  values,
}: {
  title: string;
  subtitle: string;
  points: AcquisitionSeriesPoint[];
  values: ('activeUsers' | 'newUsers')[];
}) {
  const max = Math.max(0, ...points.flatMap(point => values.map(value => point[value])));
  const yMax = Math.max(1, max);
  const yTicks = max > 1 ? [max, Math.round(max / 2), 0] : [yMax, 0];

  return (
    <ChartCard title={title} subtitle={subtitle}>
      {points.length ? (
        <div className={styles.seriesFrame} role="img" aria-label={`${title} chart`}>
          <div className={styles.seriesAxis} aria-hidden="true">
            {yTicks.map(tick => (
              <span key={tick}>{number.format(tick)}</span>
            ))}
          </div>
          <div className={styles.series}>
            {points.map(point => (
              <div className={styles.seriesColumn} key={point.bucket}>
                <div className={styles.barGroup}>
                  {values.map(value => {
                    const count = point[value];
                    return (
                      <div
                        className={value === 'newUsers' ? styles.newBar : styles.activeBar}
                        key={value}
                        style={{
                          height: count > 0 ? `${Math.max(3, (count / yMax) * 100)}%` : '0%',
                        }}
                        title={`${formatBucket(point.bucket)} · ${
                          value === 'newUsers' ? 'New' : 'Active'
                        }: ${number.format(count)}`}
                      />
                    );
                  })}
                </div>
                <span>{formatBucket(point.bucket)}</span>
              </div>
            ))}
          </div>
        </div>
      ) : (
        <CardEmpty />
      )}
      <Legend values={values} />
    </ChartCard>
  );
}

function CountSeriesCard({
  title,
  subtitle,
  points,
  labels = bucket => bucket,
}: {
  title: string;
  subtitle?: string;
  points: AcquisitionCountPoint[];
  labels?: (bucket: string) => string;
}) {
  const sorted = useMemo(
    () => [...points].sort((a, b) => Number(a.bucket) - Number(b.bucket)),
    [points],
  );
  const max = Math.max(0, ...sorted.map(point => point.users));
  const yMax = Math.max(1, max);
  const yTicks = max > 1 ? [max, Math.round(max / 2), 0] : [yMax, 0];

  return (
    <ChartCard title={title} subtitle={subtitle}>
      {sorted.length ? (
        <div className={styles.seriesFrame} role="img" aria-label={`${title} chart`}>
          <div className={styles.seriesAxis} aria-hidden="true">
            {yTicks.map(tick => (
              <span key={tick}>{number.format(tick)}</span>
            ))}
          </div>
          <div className={styles.series}>
            {sorted.map(point => (
              <div className={styles.seriesColumn} key={point.bucket}>
                <div className={styles.barGroup}>
                  <div
                    className={styles.newBar}
                    style={{
                      height:
                        point.users > 0 ? `${Math.max(3, (point.users / yMax) * 100)}%` : '0%',
                    }}
                    title={`${labels(point.bucket)}: ${number.format(point.users)}`}
                  />
                </div>
                <span>{labels(point.bucket)}</span>
              </div>
            ))}
          </div>
        </div>
      ) : (
        <CardEmpty />
      )}
    </ChartCard>
  );
}

function DonutCard({ title, points }: { title: string; points: AcquisitionBreakdownPoint[] }) {
  const visible = points.filter(point => point.users > 0).slice(0, 8);
  const total = visible.reduce((sum, point) => sum + point.users, 0);
  let cursor = 0;
  const stops = visible.map((point, index) => {
    const start = cursor;
    cursor += total ? (point.users / total) * 100 : 0;
    return `${CHART_COLORS[index % CHART_COLORS.length]} ${start}% ${cursor}%`;
  });

  return (
    <ChartCard title={title}>
      {visible.length ? (
        <div className={styles.donutLayout}>
          <div
            className={styles.donut}
            role="img"
            aria-label={`${title}: ${visible
              .map(point => `${point.name} ${number.format(point.users)}`)
              .join(', ')}`}
            style={{ background: `conic-gradient(${stops.join(', ')})` }}
          >
            <div>
              <strong>{number.format(total)}</strong>
              <span>new users</span>
            </div>
          </div>
          <ul className={styles.breakdownList}>
            {visible.map((point, index) => (
              <li key={point.name}>
                <i style={{ background: CHART_COLORS[index % CHART_COLORS.length] }} />
                <span>{point.name}</span>
                <strong>{number.format(point.users)}</strong>
              </li>
            ))}
          </ul>
        </div>
      ) : (
        <CardEmpty />
      )}
    </ChartCard>
  );
}

function ChartCard({
  title,
  subtitle,
  children,
}: {
  title: string;
  subtitle?: string;
  children: React.ReactNode;
}) {
  return (
    <article className={styles.card}>
      <header className={styles.cardHeader}>
        <div>
          <Heading size="sm">{title}</Heading>
          {subtitle && (
            <Text size="sm" color="muted">
              {subtitle}
            </Text>
          )}
        </div>
      </header>
      {children}
    </article>
  );
}

function UnavailableCard({
  title,
  subtitle,
  reason,
}: {
  title: string;
  subtitle: string;
  reason?: string;
}) {
  return (
    <ChartCard title={title} subtitle={subtitle}>
      <div className={styles.unavailable}>
        <strong>User-local time is unavailable</strong>
        <span>{reason || 'No user-local timezone is stored for these signals.'}</span>
      </div>
    </ChartCard>
  );
}

function Legend({ values }: { values: ('activeUsers' | 'newUsers')[] }) {
  return (
    <div className={styles.legend}>
      {values.map(value => (
        <span key={value}>
          <i className={value === 'newUsers' ? styles.newKey : styles.activeKey} />
          {value === 'newUsers' ? 'New users' : 'Active users'}
        </span>
      ))}
    </div>
  );
}

function StateCard({ title, body }: { title: string; body: string }) {
  return (
    <div className={styles.state}>
      <Heading size="base">{title}</Heading>
      <Text color="muted">{body}</Text>
    </div>
  );
}

function CardEmpty() {
  return <div className={styles.cardEmpty}>No identified new users in this period.</div>;
}

function ErrorState({ onRetry }: { onRetry: () => void }) {
  return (
    <div className={styles.state}>
      <Icon size="lg">
        <AlertTriangle />
      </Icon>
      <Heading size="base">Acquisition could not be loaded</Heading>
      <Text color="muted">
        The report request failed. No activity has been interpreted as zero.
      </Text>
      <Button onPress={onRetry}>
        <Icon>
          <RefreshCw />
        </Icon>
        Try again
      </Button>
    </div>
  );
}

function formatBucket(bucket: string) {
  const date = new Date(bucket);
  if (!Number.isFinite(date.getTime())) {
    return bucket;
  }
  return new Intl.DateTimeFormat(undefined, {
    month: 'short',
    day: 'numeric',
    hour: date.getUTCHours() ? 'numeric' : undefined,
  }).format(date);
}

function hourLabels(bucket: string) {
  const hour = Number(bucket);
  if (!Number.isFinite(hour)) {
    return bucket;
  }
  return new Intl.DateTimeFormat(undefined, { hour: 'numeric' }).format(
    new Date(Date.UTC(2024, 0, 1, hour)),
  );
}
