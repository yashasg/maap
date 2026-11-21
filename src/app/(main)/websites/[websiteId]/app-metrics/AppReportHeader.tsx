'use client';
import { useTranslations } from 'next-intl';
import { useAnalyticsQuery } from '@/components/hooks/queries/useAnalyticsQuery';
import { formatLongNumber } from '@/lib/format';
import styles from './AppReports.module.css';

export function AppReportHeader({ websiteId, name }: { websiteId: string; name: string }) {
  const t = useTranslations('appReportsNav');
  const { data, error } = useAnalyticsQuery<{
    summary: { users: number; mostUsedSdk: string | null; errors: number | null };
  }>('app-reports/overview', { websiteId }, { staleTime: 60_000 });
  const summary = data?.summary;
  return (
    <header className={styles.masthead}>
      <h1>{name}</h1>
      {error ? (
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
      )}
    </header>
  );
}
