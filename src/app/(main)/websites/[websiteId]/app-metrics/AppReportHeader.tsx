'use client';
import { useTranslations } from 'next-intl';
import { useMessages } from '@/components/hooks';
import { useAnalyticsQuery } from '@/components/hooks/queries/useAnalyticsQuery';
import { useWebsiteStatsQuery } from '@/components/hooks/queries/useWebsiteStatsQuery';
import { formatLongNumber } from '@/lib/format';
import styles from './AppReports.module.css';

export function AppReportHeader({
  websiteId,
  name,
  nativeApp = true,
}: {
  websiteId: string;
  name: string;
  nativeApp?: boolean;
}) {
  return (
    <header className={styles.masthead}>
      <h1>{name}</h1>
      {nativeApp ? <AppSummary websiteId={websiteId} /> : <WebsiteSummary websiteId={websiteId} />}
    </header>
  );
}

function WebsiteSummary({ websiteId }: { websiteId: string }) {
  const { t, labels, getErrorMessage } = useMessages();
  const { data, error } = useWebsiteStatsQuery({ websiteId });
  if (error) return <p role="status">{getErrorMessage(error)}</p>;
  return (
    <dl aria-busy={!data}>
      {[
        [t(labels.visitors), data?.visitors],
        [t(labels.visits), data?.visits],
        [t(labels.views), data?.pageviews],
      ].map(([label, value]) => (
        <div key={label}>
          <dt>{label}</dt>
          <dd>{typeof value === 'number' ? formatLongNumber(value) : '—'}</dd>
        </div>
      ))}
    </dl>
  );
}

function AppSummary({ websiteId }: { websiteId: string }) {
  const t = useTranslations('appReportsNav');
  const { data, error } = useAnalyticsQuery<{
    summary: { users: number; mostUsedSdk: string | null; errors: number | null };
  }>('app-reports/overview', { websiteId }, { staleTime: 60_000 });
  const summary = data?.summary;
  return error ? (
    <p role="status">{t('summary-error')}</p>
  ) : (
    <dl aria-busy={!summary}>
      <div>
        <dt>{t('users')}</dt>
        <dd>{summary ? formatLongNumber(summary.users) : '—'}</dd>
      </div>
      <div>
        <dt>{t('most-used-sdk')}</dt>
        <dd>{summary ? (summary.mostUsedSdk ?? t('unknown')) : '—'}</dd>
      </div>
      <div>
        <dt>{t('errors')}</dt>
        <dd>
          {summary
            ? summary.errors == null
              ? t('not-available')
              : formatLongNumber(summary.errors)
            : '—'}
        </dd>
      </div>
    </dl>
  );
}
