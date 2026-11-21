'use client';

import { useQuery } from '@tanstack/react-query';
import { Column, Heading, Text } from '@umami/react-zen';
import { useMemo } from 'react';
import { PageHeader } from '@/components/common/PageHeader';
import { useApi } from '@/components/hooks/useApi';
import { useDateParameters } from '@/components/hooks/useDateParameters';
import { useNavigation } from '@/components/hooks/useNavigation';
import { CHART_COLORS } from '@/lib/constants';
import {
  CardGrid,
  CountSeriesCard,
  ErrorState,
  LoadingState,
  ReportSection,
  StateCard,
  type SeriesProps,
  type CountPoint,
} from '../LifecycleCards';
import styles from '../Lifecycle.module.css';

export type RetentionPoint = CountPoint;

export type RetentionReport = {
  criteria: {
    verified: boolean;
    rule: string;
  };
  scope: {
    startAt: string;
    endAt: string;
    timezone: string;
    testMode: boolean;
    totalSignals: number;
    identifiedSignals: number;
    returningUsers: number;
  };
  series: {
    hourly: RetentionPoint[];
    daily: RetentionPoint[];
    weekly: RetentionPoint[];
    monthly: RetentionPoint[];
  };
  patterns: {
    hourOfDay: RetentionPoint[];
    hourOfDayFourWeeks: RetentionPoint[];
    dayOfWeek: RetentionPoint[];
    dayOfWeekFourWeeks: RetentionPoint[];
    weekpart: RetentionPoint[];
    weekpartFourWeeks: RetentionPoint[];
  };
  breakdowns: {
    device: RetentionPoint[];
    platform: RetentionPoint[];
    region: RetentionPoint[];
    language: RetentionPoint[];
  };
  availability: {
    userLocalTime: boolean;
    preferredRegion: boolean;
    reason: string;
  };
};

type ApiSeriesPoint = { bucket: string; returningUsers: number };
type ApiCountPoint = { bucket: string; users: number };
type ApiBreakdownPoint = { name: string; users: number };

export type RetentionApiResponse = {
  criteria: RetentionReport['criteria'];
  range: {
    startAt: string;
    endAt: string;
    timezone: string;
    testMode: boolean;
  };
  scope: Pick<RetentionReport['scope'], 'totalSignals' | 'identifiedSignals' | 'returningUsers'>;
  series: Record<keyof RetentionReport['series'], ApiSeriesPoint[]>;
  patterns: Record<keyof RetentionReport['patterns'], ApiCountPoint[]>;
  breakdowns: Record<keyof RetentionReport['breakdowns'], ApiBreakdownPoint[]>;
  availability: RetentionReport['availability'];
};

const number = new Intl.NumberFormat();
const weekDays = ['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday', 'Sunday'];

function seriesPoints(
  points: ApiSeriesPoint[],
  kind: 'hour' | 'day' | 'week' | 'month',
): RetentionPoint[] {
  return points.map(point => ({
    key: point.bucket,
    label:
      kind === 'hour'
        ? point.bucket.slice(11, 16)
        : kind === 'month'
          ? point.bucket.slice(0, 7)
          : point.bucket.slice(5, 10).replace('-', '/'),
    count: point.returningUsers,
  }));
}

function patternPoints(
  points: ApiCountPoint[],
  kind: 'hour' | 'dayOfWeek' | 'weekpart',
): RetentionPoint[] {
  return [...points]
    .sort((left, right) =>
      kind === 'weekpart'
        ? left.bucket.localeCompare(right.bucket)
        : Number(left.bucket) - Number(right.bucket),
    )
    .map(point => {
      const numeric = Number(point.bucket);
      const label =
        kind === 'hour'
          ? `${String(numeric).padStart(2, '0')}:00`
          : kind === 'dayOfWeek'
            ? weekDays[numeric - 1] || point.bucket
            : point.bucket;
      return { key: point.bucket, label, count: point.users };
    });
}

export function normalizeRetentionReport(response: RetentionApiResponse): RetentionReport {
  return {
    criteria: response.criteria,
    scope: { ...response.range, ...response.scope },
    series: {
      hourly: seriesPoints(response.series.hourly, 'hour'),
      daily: seriesPoints(response.series.daily, 'day'),
      weekly: seriesPoints(response.series.weekly, 'week'),
      monthly: seriesPoints(response.series.monthly, 'month'),
    },
    patterns: {
      hourOfDay: patternPoints(response.patterns.hourOfDay, 'hour'),
      hourOfDayFourWeeks: patternPoints(response.patterns.hourOfDayFourWeeks, 'hour'),
      dayOfWeek: patternPoints(response.patterns.dayOfWeek, 'dayOfWeek'),
      dayOfWeekFourWeeks: patternPoints(response.patterns.dayOfWeekFourWeeks, 'dayOfWeek'),
      weekpart: patternPoints(response.patterns.weekpart, 'weekpart'),
      weekpartFourWeeks: patternPoints(response.patterns.weekpartFourWeeks, 'weekpart'),
    },
    breakdowns: Object.fromEntries(
      Object.entries(response.breakdowns).map(([key, points]) => [
        key,
        points.map(point => ({ key: point.name, label: point.name, count: point.users })),
      ]),
    ) as RetentionReport['breakdowns'],
    availability: response.availability,
  };
}

export default function RetentionScreen({ websiteId }: { websiteId: string }) {
  const { get } = useApi();
  const { startAt, endAt, timezone } = useDateParameters();
  const { query: urlQuery } = useNavigation();
  const testMode =
    urlQuery.testMode === 'true' || urlQuery.tag === 'test' || urlQuery.tag === 'eq.test';

  const query = useQuery<RetentionApiResponse, Error, RetentionReport>({
    queryKey: ['app-report-retention', { websiteId, startAt, endAt, timezone, testMode }],
    queryFn: () =>
      get(`/websites/${websiteId}/app-reports/retention`, {
        startAt,
        endAt,
        timezone,
        testMode: testMode ? 'true' : undefined,
      }),
    enabled: Boolean(websiteId),
    select: normalizeRetentionReport,
  });

  return (
    <Column className={styles.returning} gap="5" minWidth="0">
      <PageHeader
        title="Retention"
        description="See when identified users return to your app and how their return activity is distributed."
        showBorder={false}
      />

      {query.isLoading ? (
        <LoadingState label="Loading retention report" />
      ) : query.error ? (
        <ErrorState report="Retention" onRetry={() => query.refetch()} />
      ) : query.data ? (
        <Report report={query.data} />
      ) : null}
    </Column>
  );
}

function Report({ report }: { report: RetentionReport }) {
  const { scope } = report;
  const coverage =
    scope.totalSignals > 0 ? Math.round((scope.identifiedSignals / scope.totalSignals) * 100) : 0;

  if (scope.totalSignals === 0) {
    return (
      <StateCard
        title="No signals in this range"
        body="Try a wider date range or switch Test Mode to view a different signal set."
      />
    );
  }

  if (scope.identifiedSignals === 0) {
    return (
      <StateCard
        title="Returning users cannot be counted"
        body="Signals exist in this range, but none include an SDK clientUser identity. Maap will not substitute sessions or device fingerprints for users."
      />
    );
  }

  return (
    <Column gap="6">
      <section className={styles.definition} aria-labelledby="retention-criteria">
        <div>
          <span className={styles.status}>
            {report.criteria.verified ? 'Criteria verified' : 'Parity criteria unverified'}
          </span>
          <Heading id="retention-criteria" size="sm">
            What Maap counts as a returning user
          </Heading>
          <Text color="muted">
            {report.criteria.rule} Counts use distinct SDK <code>clientUser</code> identities; Maap
            does not reuse session-cohort percentages as returning-user counts.
          </Text>
          {!report.availability.userLocalTime && (
            <Text color="muted" size="sm">
              Time patterns use the dashboard timezone. {report.availability.reason}
            </Text>
          )}
        </div>
        <dl className={styles.summary}>
          <div>
            <dt>Returning users (daily rule)</dt>
            <dd>{number.format(scope.returningUsers)}</dd>
          </div>
          <div>
            <dt>Signal identity coverage</dt>
            <dd>{coverage}%</dd>
          </div>
        </dl>
      </section>

      <CardGrid>
        <SeriesCard title="Hourly Returning Users" points={report.series.hourly} />
        <SeriesCard title="Daily Returning Users" points={report.series.daily} />
        <SeriesCard title="Weekly Returning Users" points={report.series.weekly} />
        <SeriesCard title="Monthly Returning Users" points={report.series.monthly} />
      </CardGrid>

      <ReportSection title="Time Patterns">
        <CardGrid>
          <SeriesCard
            title="Returning Users by Hour of Day"
            subtitle="Dashboard timezone"
            points={report.patterns.hourOfDay}
          />
          <SeriesCard
            title="Returning Users by Hour of Day"
            subtitle="Past 4 weeks within the selected scope"
            points={report.patterns.hourOfDayFourWeeks}
          />
          <SeriesCard title="Returning Users by Day of Week" points={report.patterns.dayOfWeek} />
          <SeriesCard
            title="Returning Users by Day"
            subtitle="Past 4 weeks within the selected scope"
            points={report.patterns.dayOfWeekFourWeeks}
          />
          <BreakdownCard
            title="Returning Users on Weekends vs Weekdays"
            points={report.patterns.weekpart}
          />
          <SeriesCard
            title="Returning Users on Weekends vs Weekdays"
            subtitle="Past 4 weeks within the selected scope"
            points={report.patterns.weekpartFourWeeks}
          />
        </CardGrid>
      </ReportSection>

      <ReportSection title="Retention by Device & Platform">
        <CardGrid>
          <BreakdownCard title="Returning Users by Device Type" points={report.breakdowns.device} />
          <BreakdownCard title="Returning Users by Platform" points={report.breakdowns.platform} />
        </CardGrid>
      </ReportSection>

      <ReportSection title="Retention by Geography & Language">
        <CardGrid>
          {report.availability.preferredRegion ? (
            <BreakdownCard
              title="Returning Users by Preferred Region"
              points={report.breakdowns.region}
            />
          ) : (
            <UnavailableCard
              title="Returning Users by Preferred Region"
              body="Preferred region is not stored per signal. Maap does not substitute IP-derived geography."
            />
          )}
          <BreakdownCard
            title="Returning Users by Preferred Language"
            points={report.breakdowns.language}
          />
        </CardGrid>
      </ReportSection>
    </Column>
  );
}

function SeriesCard(props: SeriesProps) {
  return (
    <CountSeriesCard
      {...props}
      peakLabel="Peak "
      missingMessage="No returning users for this breakdown."
    />
  );
}

function BreakdownCard({ title, points }: { title: string; points: RetentionPoint[] }) {
  const visible = useMemo(() => {
    const ranked = [...points]
      .filter(point => point.count > 0)
      .sort((left, right) => right.count - left.count);
    if (ranked.length <= 6) return ranked;
    return [
      ...ranked.slice(0, 5),
      {
        key: 'other',
        label: 'Other',
        count: ranked.slice(5).reduce((sum, point) => sum + point.count, 0),
      },
    ];
  }, [points]);
  const total = visible.reduce((sum, point) => sum + point.count, 0);
  let cursor = 0;
  const stops = visible.map((point, index) => {
    const start = cursor;
    cursor += total ? (point.count / total) * 100 : 0;
    return `${CHART_COLORS[index % CHART_COLORS.length]} ${start}% ${cursor}%`;
  });

  return (
    <article className={styles.card}>
      <header className={styles.cardHeader}>
        <Heading size="sm">{title}</Heading>
        <div className={styles.cardValue}>{number.format(total)} users</div>
      </header>
      {visible.length ? (
        <div className={styles.breakdown}>
          <div
            className={styles.donut}
            role="img"
            aria-label={`${title}: ${visible
              .map(point => `${point.label} ${number.format(point.count)}`)
              .join(', ')}`}
            style={{ background: `conic-gradient(${stops.join(', ')})` }}
          >
            <span>{number.format(total)}</span>
          </div>
          <ul className={styles.legend}>
            {visible.map((point, index) => (
              <li key={point.key}>
                <span
                  className={styles.swatch}
                  style={{ backgroundColor: CHART_COLORS[index % CHART_COLORS.length] }}
                />
                <span>{point.label}</span>
                <strong>{number.format(point.count)}</strong>
              </li>
            ))}
          </ul>
        </div>
      ) : (
        <CardEmpty />
      )}
    </article>
  );
}

function CardEmpty() {
  return <div className={styles.cardEmpty}>No returning users for this breakdown.</div>;
}

function UnavailableCard({ title, body }: { title: string; body: string }) {
  return (
    <article className={styles.card}>
      <Heading size="sm">{title}</Heading>
      <div className={styles.cardEmpty}>{body}</div>
    </article>
  );
}
