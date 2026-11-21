'use client';
import { useTranslations } from 'next-intl';
import type { ReactNode } from 'react';
import Link from '@/components/common/Link';
import { useNavigation } from '@/components/hooks';
import {
  BookOpen,
  ChartNoAxesCombined,
  CircleGauge,
  Compass,
  LayoutDashboard,
  Settings,
  Users,
  Wrench,
} from '@/components/icons';
import styles from './AppReports.module.css';
import { type AppReport, reportGroups } from './sections';

const icons = {
  Overview: CircleGauge,
  Customers: Users,
  Metrics: ChartNoAxesCombined,
  Explore: Compass,
  Dashboards: LayoutDashboard,
  Notebooks: BookOpen,
  Settings,
  Setup: Wrench,
};

export function AppReportNav({
  websiteId,
  screen,
  children,
  controls,
  header,
}: {
  websiteId: string;
  screen: AppReport;
  children: ReactNode;
  controls?: ReactNode;
  header?: ReactNode;
}) {
  const { renderUrl } = useNavigation();
  const t = useTranslations('appReportsNav');
  const groups = Object.entries(reportGroups);
  const [activeGroup, siblings] = groups.find(([, items]) => items.some(slug => slug === screen))!;
  const href = (slug: string) =>
    renderUrl(`/websites/${websiteId}/app-reports/${slug}`, {
      page: undefined,
      search: undefined,
    });
  const hasSidebar = siblings.length > 1;
  return (
    <div className={styles.shell}>
      {header}
      <nav className={styles.tabs} aria-label={t('navigation')}>
        {groups.map(([group, items]) => {
          const Icon = icons[group as keyof typeof icons];
          return (
            <Link
              key={group}
              href={href(group === activeGroup ? screen : items[0])}
              aria-current={group === activeGroup ? 'location' : undefined}
            >
              <Icon size={16} aria-hidden />
              {t(group)}
            </Link>
          );
        })}
      </nav>
      {controls}
      <div className={`${styles.reportBody} ${hasSidebar ? styles.columns : ''}`}>
        {hasSidebar && (
          <nav
            aria-label={t('section-navigation', { group: t(activeGroup) })}
            className={styles.side}
          >
            {siblings.map(slug => (
              <Link
                key={slug}
                href={href(slug)}
                aria-current={slug === screen ? 'page' : undefined}
              >
                {t(slug)}
              </Link>
            ))}
          </nav>
        )}
        <div className={styles.content} key={screen}>
          {children}
        </div>
      </div>
    </div>
  );
}
