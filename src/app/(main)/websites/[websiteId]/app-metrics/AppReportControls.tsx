'use client';
import { useTranslations } from 'next-intl';
import { useNavigation } from '@/components/hooks';
import { WebsiteDateFilter } from '@/components/input/WebsiteDateFilter';
import styles from './AppReports.module.css';

export function AppReportControls({ websiteId }: { websiteId: string }) {
  const { query, router, updateParams } = useNavigation();
  const t = useTranslations('appReportsNav');
  const testTag = query.tag === 'test' || query.tag === 'eq.test';
  const enabled = query.testMode === 'true' || testTag;
  return (
    <div className={styles.controls}>
      <WebsiteDateFilter websiteId={websiteId} />
      <button
        type="button"
        aria-label={t('test-mode')}
        aria-pressed={enabled}
        onClick={() =>
          router.push(
            updateParams({
              testMode: enabled ? undefined : 'true',
              tag: testTag ? undefined : query.tag,
              page: undefined,
            }),
          )
        }
      >
        {enabled ? t('test-mode-on') : `+ ${t('test-mode')}`}
      </button>
    </div>
  );
}
