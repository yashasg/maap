'use client';

import { Column, Heading, Text } from '@umami/react-zen';
import { useMemo } from 'react';
import { PageHeader } from '@/components/common/PageHeader';
import { useAnalyticsQuery } from '@/components/hooks/queries/useAnalyticsQuery';
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

export type ActivationPoint = CountPoint;

export type ActivationReport = {
  definition: {
    rule: string;
    thresholdSeconds: number;
    sessionLimit: number;
    source: string;
    configurable: boolean;
    calculation: string;
  };
  configuration: {
    status: 'empty' | 'identity-missing' | 'configuration-missing' | 'partial' | 'ready';
    coreReady: boolean;
    missingKeys: string[];
    requiredAllowlist: string[];
    environmentVariable: string;
    historicalBackfill: boolean;
  };
  scope: {
    startAt: string;
    endAt: string;
    timezone: string;
    testMode: boolean;
    totalSignals: number;
    identifiedSignals: number;
    activationInputSignals: number;
    activatedUsers: number;
  };
  series: {
    hourly: ActivationPoint[];
    daily: ActivationPoint[];
    weekly: ActivationPoint[];
    monthly: ActivationPoint[];
  };
  patterns: {
    hourOfDay: ActivationPoint[];
    hourOfDay4Weeks: ActivationPoint[];
    dayOfWeek: ActivationPoint[];
    dayOfWeek4Weeks: ActivationPoint[];
    weekpart: ActivationPoint[];
    weekpart4Weeks: ActivationPoint[];
  };
  breakdowns: {
    device: ActivationPoint[];
    platform: ActivationPoint[];
    region: ActivationPoint[];
    language: ActivationPoint[];
  };
};

const number = new Intl.NumberFormat();
const activationDocumentation = 'https://telemetrydeck.com/docs/basics/activation/';
const dayLabels = ['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday', 'Sunday'];

export default function ActivationScreen({ websiteId }: { websiteId: string }) {
  const query = useAnalyticsQuery<ActivationReport>(
    'app-reports/activation',
    { websiteId },
    { staleTime: 60_000 },
  );

  return (
    <Column gap="5" minWidth="0">
      <PageHeader
        title="Activation"
        description="Monitor when identified app users meet the documented activation condition for the first time."
        showBorder={false}
      >
        <a
          className={styles.docsLink}
          href={activationDocumentation}
          target="_blank"
          rel="noreferrer"
        >
          Open documentation
        </a>
      </PageHeader>

      {query.isLoading ? (
        <LoadingState label="Loading activation report" />
      ) : query.error ? (
        <ErrorState report="Activation" onRetry={() => query.refetch()} />
      ) : query.data ? (
        <Report report={query.data} />
      ) : null}
    </Column>
  );
}

function Report({ report }: { report: ActivationReport }) {
  const { configuration, scope } = report;

  if (configuration.status === 'empty') {
    return (
      <StateCard
        title="No signals in this range"
        body="Try a wider date range or switch Test Mode to view a different signal set."
      />
    );
  }

  if (configuration.status === 'identity-missing') {
    return (
      <StateCard
        title="Activated users cannot be counted"
        body="Signals exist in this range, but none include an SDK clientUser identity. Maap will not substitute sessions, first-seen devices, or network fingerprints for users."
      />
    );
  }

  if (configuration.status === 'configuration-missing') {
    return <ConfigurationState report={report} />;
  }

  const coverage = scope.identifiedSignals
    ? Math.round((scope.activationInputSignals / scope.identifiedSignals) * 100)
    : 0;

  return (
    <Column gap="6">
      <section className={styles.definition} aria-labelledby="activation-definition">
        <div>
          <span className={styles.status}>Documented condition</span>
          <Heading id="activation-definition" size="sm">
            What Maap counts as an activated user
          </Heading>
          <Text color="muted">
            {report.definition.rule} Repeated qualifying signals still count the SDK user once.
          </Text>
          <Text color="muted" size="sm">
            {report.definition.calculation}
          </Text>
        </div>
        <dl className={styles.summary}>
          <div>
            <dt>Activated users</dt>
            <dd>{number.format(scope.activatedUsers)}</dd>
          </div>
          <div>
            <dt>Activation data coverage</dt>
            <dd>{coverage}%</dd>
          </div>
        </dl>
      </section>

      {(configuration.status === 'partial' || configuration.missingKeys.length > 0) && (
        <CoverageNotice report={report} />
      )}

      <CardGrid>
        <SeriesCard title="Hourly Activated Users (in last 24h)" points={report.series.hourly} />
        <SeriesCard title="Daily Activated Users" points={report.series.daily} />
        <SeriesCard
          title="Weekly Activated Users (in past 3 months)"
          points={report.series.weekly}
        />
        <SeriesCard title="Monthly Activated Users (in past year)" points={report.series.monthly} />
      </CardGrid>

      <ReportSection title="Activation Time Patterns">
        <CardGrid>
          <SeriesCard
            title="Activation by Hour of Day (user-local time)"
            points={withHourLabels(report.patterns.hourOfDay, true)}
            missingMessage="User-local hour was not reported for activated users."
          />
          <SeriesCard
            title="Activation by Hour of Day (past 4 weeks)"
            subtitle={scope.timezone}
            points={withHourLabels(report.patterns.hourOfDay4Weeks, false)}
          />
          <SeriesCard
            title="Activated Users by Day of Week"
            points={withDayLabels(report.patterns.dayOfWeek)}
            missingMessage="User-local day was not reported for activated users."
          />
          <SeriesCard
            title="Activation by Day (past 4 weeks)"
            subtitle={scope.timezone}
            points={withDayLabels(report.patterns.dayOfWeek4Weeks)}
          />
          <BreakdownCard
            title="Activated Users on Weekends vs Weekdays"
            points={report.patterns.weekpart}
            missingMessage="The SDK did not report a user-local weekend value."
          />
          <SeriesCard
            title="Activated Users Weekend/Weekdays (past 4 weeks)"
            subtitle={scope.timezone}
            points={report.patterns.weekpart4Weeks}
          />
        </CardGrid>
      </ReportSection>

      <ReportSection title="Activation by Device & Platform">
        <CardGrid>
          <BreakdownCard title="Activated Users by Device Type" points={report.breakdowns.device} />
          <BreakdownCard title="Activated Users by Platform" points={report.breakdowns.platform} />
        </CardGrid>
      </ReportSection>

      <ReportSection title="Activation by Geography & Language">
        <CardGrid>
          <BreakdownCard
            title="Activated Users by Preferred Region"
            points={report.breakdowns.region}
            missingMessage="Preferred region was not reported. Maap does not substitute IP location."
          />
          <BreakdownCard
            title="Activated Users by Preferred Language"
            points={report.breakdowns.language}
          />
        </CardGrid>
      </ReportSection>
    </Column>
  );
}

function ConfigurationState({ report }: { report: ActivationReport }) {
  return (
    <section className={styles.configuration} aria-labelledby="activation-configuration">
      <div>
        <span className={styles.status}>Configuration required</span>
        <Heading id="activation-configuration" size="base">
          Retain the SDK activation fields
        </Heading>
        <Text color="muted">
          Maap received identified signals, but the two fields needed to apply the documented
          five-minute, first-five-sessions condition are not stored. Add the fields below to the
          existing <code>{report.configuration.environmentVariable}</code> allowlist. This setting
          replaces Maap&apos;s defaults, so keep every SDK field used by your other reports.
        </Text>
      </div>
      <code className={styles.allowlist} data-test="activation-allowlist">
        {report.configuration.requiredAllowlist.join(',')}
      </code>
      <Text color="muted" size="sm">
        Previously discarded values cannot be recovered. Reports begin when newly ingested signals
        carry these fields.
      </Text>
      <a
        className={styles.docsLink}
        href={activationDocumentation}
        target="_blank"
        rel="noreferrer"
      >
        Read the activation definition
      </a>
    </section>
  );
}

function CoverageNotice({ report }: { report: ActivationReport }) {
  return (
    <aside className={styles.notice}>
      <Heading size="sm">Some activation detail is unavailable</Heading>
      <Text color="muted">
        Signals without activation inputs are excluded instead of being counted as not activated.
        Missing optional fields leave their user-local time or preferred-region charts empty.
      </Text>
      {report.configuration.missingKeys.length > 0 && (
        <code>{report.configuration.missingKeys.join(', ')}</code>
      )}
    </aside>
  );
}

function SeriesCard(props: SeriesProps & { missingMessage?: string }) {
  return <CountSeriesCard missingMessage="No activated users in this period." {...props} />;
}

function BreakdownCard({
  title,
  points,
  missingMessage = 'No activated users for this breakdown.',
}: {
  title: string;
  points: ActivationPoint[];
  missingMessage?: string;
}) {
  const visible = useMemo(() => points.filter(point => point.count > 0).slice(0, 6), [points]);
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
        <div className={styles.cardValue}>{number.format(total)}</div>
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
        <CardEmpty message={missingMessage} />
      )}
    </article>
  );
}

function CardEmpty({ message }: { message: string }) {
  return <div className={styles.cardEmpty}>{message}</div>;
}

function withHourLabels(points: ActivationPoint[], sdkHour: boolean) {
  return [...points]
    .sort((a, b) => Number(a.key) - Number(b.key))
    .map(point => {
      const hour = Number(point.key);
      const displayed = sdkHour ? (hour + 23) % 24 : hour;
      return { ...point, label: `${String(displayed).padStart(2, '0')}:00` };
    });
}

function withDayLabels(points: ActivationPoint[]) {
  return [...points]
    .sort((a, b) => Number(a.key) - Number(b.key))
    .map(point => ({ ...point, label: dayLabels[Number(point.key) - 1] || point.label }));
}
