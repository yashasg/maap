'use client';

import { Button, Heading, Icon, Text } from '@umami/react-zen';
import { type CSSProperties, useMemo, useState } from 'react';
import { LoadingPanel } from '@/components/common/LoadingPanel';
import { Panel } from '@/components/common/Panel';
import { useAnalyticsQuery } from '@/components/hooks/queries/useAnalyticsQuery';
import { useAppSettingsQuery } from '@/components/hooks/queries/useAppSettingsQuery';
import { RefreshCw } from '@/components/icons';
import { APP_COLOR_PALETTES, DEFAULT_APP_SETTINGS } from '@/lib/app-settings';
import { formatLongNumber } from '@/lib/format';
import { OverviewSkeleton } from '../ReportSkeleton';
import styles from './Overview.module.css';

interface BreakdownValue {
  value: string;
  users: number;
}

interface AppOverviewData {
  identity: 'clientUser';
  range: {
    startAt: string;
    endAt: string;
    timezone: string;
  };
  summary: {
    users: number;
    usersLastHour: number;
    events: number;
    errors: number | null;
    mostUsedSdk: string | null;
  };
  dailyUsers: Array<{ x: string; y: number }>;
  retention: Array<{
    day: number;
    eligibleUsers: number;
    returningUsers: number;
    percentage: number | null;
  }>;
  dimensions: {
    sdkName: BreakdownValue[];
    sdkVersion: BreakdownValue[];
    appVersion: BreakdownValue[];
    buildNumber: BreakdownValue[];
    systemVersion: BreakdownValue[];
  };
}

function formatDate(value: string) {
  if (!value) return '—';
  return new Intl.DateTimeFormat(undefined, {
    month: 'short',
    day: 'numeric',
    timeZone: 'UTC',
  }).format(new Date(`${value}T00:00:00Z`));
}

function DailyUsersChart({ values }: { values: Array<{ x: string; y: number }> }) {
  const max = Math.max(1, ...values.map(value => value.y));
  const yTicks = Array.from(new Set([0, 0.25, 0.5, 0.75, 1].map(ratio => Math.round(max * ratio))));
  const points = values.map((value, index) => ({
    ...value,
    xPct: values.length <= 1 ? 50 : (index / (values.length - 1)) * 100,
    yPct: (value.y / max) * 100,
  }));
  const line = points
    .map(({ xPct, yPct }, index) => `${index ? 'L' : 'M'} ${xPct} ${100 - yPct}`)
    .join(' ');
  const area =
    points.length > 1 ? `${line} L ${points.at(-1)?.xPct} 100 L ${points[0].xPct} 100 Z` : '';
  const labelIndexes = Array.from(
    new Set([0, Math.floor((values.length - 1) / 2), values.length - 1]),
  ).filter(index => index >= 0);

  return (
    <div className={styles.chart} role="img" aria-label="Daily active identified users">
      <div className={styles.chartBody}>
        <div className={styles.yAxis} aria-hidden="true">
          {yTicks.map(tick => (
            <span
              key={tick}
              className={styles.yAxisLabel}
              style={{ bottom: `${(tick / max) * 100}%` }}
            >
              {formatLongNumber(tick)}
            </span>
          ))}
        </div>
        <div className={styles.chartPlot}>
          {/* ponytail: SVG stretches only area/line/gridlines; HTML renders crisp text & round dots */}
          <svg
            className={styles.chartSvg}
            viewBox="0 0 100 100"
            preserveAspectRatio="none"
            aria-hidden="true"
          >
            {yTicks.map(tick => {
              const y = 100 - (tick / max) * 100;
              return <line key={tick} className={styles.gridLine} x1={0} x2={100} y1={y} y2={y} />;
            })}
            {area && <path className={styles.area} d={area} />}
            {line && <path className={styles.line} d={line} />}
          </svg>
          {points.map(point => (
            <span
              key={point.x}
              className={styles.point}
              style={{ left: `${point.xPct}%`, bottom: `${point.yPct}%` }}
              title={`${formatDate(point.x)}: ${formatLongNumber(point.y)}`}
            />
          ))}
        </div>
        <div className={styles.xAxis} aria-hidden="true">
          {labelIndexes.map(index => {
            const point = points[index];
            if (!point) return null;
            const align =
              values.length === 1 || (index > 0 && index < values.length - 1)
                ? styles.xLabelCenter
                : index === 0
                  ? styles.xLabelStart
                  : styles.xLabelEnd;
            return (
              <span
                key={point.x}
                className={`${styles.xAxisLabel} ${align}`}
                style={{ left: `${point.xPct}%` }}
              >
                {formatDate(point.x)}
              </span>
            );
          })}
        </div>
      </div>
      {!values.some(value => value.y > 0) && (
        <div className={styles.chartEmpty}>No identified-user activity in this period</div>
      )}
    </div>
  );
}

function Fact({ label, value, detail }: { label: string; value: string; detail?: string }) {
  return (
    <div className={styles.fact}>
      <span className={styles.factLabel}>{label}</span>
      <strong className={styles.factValue}>{value}</strong>
      {detail && <span className={styles.factDetail}>{detail}</span>}
    </div>
  );
}

function SegmentedControl({
  value,
  options,
  onChange,
  label,
}: {
  value: string;
  options: Array<{ value: string; label: string }>;
  onChange: (value: string) => void;
  label: string;
}) {
  return (
    <div className={styles.segmented} aria-label={label}>
      {options.map(option => (
        <button
          key={option.value}
          type="button"
          aria-pressed={value === option.value}
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

function Breakdown({ values, total }: { values: BreakdownValue[]; total: number }) {
  if (!values.length || total === 0) {
    return <div className={styles.empty}>No identified-user data for this period</div>;
  }

  return (
    <div className={styles.breakdown}>
      {values.slice(0, 5).map(item => {
        const percentage = (item.users / total) * 100;
        return (
          <div className={styles.breakdownRow} key={item.value}>
            <div className={styles.breakdownLabel}>
              <span title={item.value}>{item.value}</span>
              <strong>
                {formatLongNumber(item.users)} · {percentage.toFixed(1)}%
              </strong>
            </div>
            <div className={styles.track}>
              <span style={{ width: `${percentage}%` }} />
            </div>
          </div>
        );
      })}
    </div>
  );
}

function systemVersionLabel(value: string, mode: 'major' | 'minor') {
  if (value === 'Unknown') return value;
  const match = value.match(/^(.*?)(\d+)(?:\.(\d+))?/);
  if (!match) return value;
  const platform = match[1].trim();
  const version = mode === 'minor' && match[3] ? `${match[2]}.${match[3]}` : match[2];
  return `${platform} ${version}`.trim();
}

function aggregateSystemVersions(values: BreakdownValue[], mode: 'major' | 'minor') {
  const totals = new Map<string, number>();
  for (const item of values) {
    const label = systemVersionLabel(item.value, mode);
    totals.set(label, (totals.get(label) ?? 0) + item.users);
  }
  return [...totals.entries()]
    .map(([value, users]) => ({ value, users }))
    .sort((a, b) => b.users - a.users || a.value.localeCompare(b.value));
}

export default function Screen({ websiteId }: { websiteId: string }) {
  const query = useAnalyticsQuery<AppOverviewData>(
    'app-reports/overview',
    { websiteId },
    { staleTime: 60_000 },
  );
  const settingsQuery = useAppSettingsQuery(websiteId, { staleTime: 60_000 });
  const [appMode, setAppMode] = useState<'versions' | 'builds'>('versions');
  const [systemMode, setSystemMode] = useState<'major' | 'minor'>('major');
  const data = query.data;
  const settings = settingsQuery.data ?? DEFAULT_APP_SETTINGS;
  const palette = APP_COLOR_PALETTES[settings.colorScheme];
  const systemVersions = useMemo(
    () => aggregateSystemVersions(data?.dimensions.systemVersion ?? [], systemMode),
    [data?.dimensions.systemVersion, systemMode],
  );

  return (
    <section
      className={`${styles.screen} ${
        settings.overviewLayout === 'website' ? styles.websiteLayout : ''
      }`}
      style={
        {
          '--overview-accent': palette[0],
          '--overview-accent-soft': palette[1] ?? palette[0],
        } as CSSProperties
      }
      data-overview-layout={settings.overviewLayout}
      data-color-scheme={settings.colorScheme}
      aria-label="Overview"
    >
      <LoadingPanel
        data={data}
        error={query.error ?? settingsQuery.error}
        isLoading={query.isLoading || settingsQuery.isLoading}
        isFetching={
          (query.isFetching && !data) || (settingsQuery.isFetching && !settingsQuery.data)
        }
        minHeight="32rem"
        loadingContent={<OverviewSkeleton />}
      >
        {data && (
          <>
            <Panel className={styles.dailyPanel}>
              <div className={styles.panelHeading}>
                <div>
                  <Heading size="base">Daily Active Users</Heading>
                  <Text color="muted" size="sm">
                    Distinct SDK identities active per calendar day in {data.range.timezone}
                  </Text>
                </div>
                <Text color="muted" size="sm">
                  {formatDate(data.dailyUsers[0]?.x ?? '')} –{' '}
                  {formatDate(data.dailyUsers.at(-1)?.x ?? '')}
                </Text>
              </div>
              <DailyUsersChart values={data.dailyUsers} />
              <Button
                size="sm"
                variant="quiet"
                aria-label="Refresh overview"
                style={{ alignSelf: 'flex-start' }}
                isDisabled={query.isFetching}
                onPress={() => query.refetch()}
              >
                <Icon>
                  <RefreshCw />
                </Icon>
                Refresh
              </Button>
            </Panel>

            <div className={styles.metricStrip}>
              <Fact
                label="Identified users in the last hour"
                value={formatLongNumber(data.summary.usersLastHour)}
                detail="Rolling 60 minutes"
              />
              <Fact
                label="Total identified users"
                value={formatLongNumber(data.summary.users)}
                detail="Selected range"
              />
              <Fact
                label="Events"
                value={formatLongNumber(data.summary.events)}
                detail="Selected range"
              />
            </div>

            <div className={styles.detailGrid}>
              <Panel className={styles.detailPanel}>
                <div className={styles.panelHeading}>
                  <div>
                    <Heading size="base">User Retention</Heading>
                    <Text color="muted" size="sm">
                      New identified users returning on the exact day
                    </Text>
                  </div>
                </div>
                <div className={styles.retention}>
                  {data.retention.map(item => (
                    <div className={styles.retentionRow} key={item.day}>
                      <span>Day {item.day}</span>
                      <div className={styles.track}>
                        <span style={{ width: `${item.percentage ?? 0}%` }} />
                      </div>
                      <strong>
                        {item.percentage == null ? '—' : `${item.percentage.toFixed(1)}%`}
                      </strong>
                    </div>
                  ))}
                </div>
                {!data.retention.some(item => item.eligibleUsers > 0) && (
                  <div className={styles.empty}>
                    The selected range has no identified-user cohort old enough to measure.
                  </div>
                )}
              </Panel>

              <Panel className={styles.detailPanel}>
                <div className={styles.panelHeading}>
                  <Heading size="base">App Versions</Heading>
                  <SegmentedControl
                    label="App version grouping"
                    value={appMode}
                    options={[
                      { value: 'versions', label: 'Versions' },
                      { value: 'builds', label: 'Builds' },
                    ]}
                    onChange={value => setAppMode(value as 'versions' | 'builds')}
                  />
                </div>
                <Breakdown
                  total={data.summary.users}
                  values={
                    appMode === 'versions'
                      ? data.dimensions.appVersion
                      : data.dimensions.buildNumber
                  }
                />
              </Panel>

              <Panel className={styles.detailPanel}>
                <div className={styles.panelHeading}>
                  <Heading size="base">System Versions</Heading>
                  <SegmentedControl
                    label="System version grouping"
                    value={systemMode}
                    options={[
                      { value: 'major', label: 'Major' },
                      { value: 'minor', label: 'Minor' },
                    ]}
                    onChange={value => setSystemMode(value as 'major' | 'minor')}
                  />
                </div>
                <Breakdown total={data.summary.users} values={systemVersions} />
              </Panel>
            </div>
          </>
        )}
      </LoadingPanel>
    </section>
  );
}
