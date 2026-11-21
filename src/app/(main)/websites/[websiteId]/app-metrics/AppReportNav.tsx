'use client';
import { useTranslations } from 'next-intl';
import type { ReactNode } from 'react';
import Link from '@/components/common/Link';
import { useMessages, useNavigation, useWebsiteNavItems } from '@/components/hooks';
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
import { AppReportControls } from './AppReportControls';
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
  const href = (slug: string) =>
    renderUrl(`/websites/${websiteId}/app-reports/${slug}`, {
      page: undefined,
      search: undefined,
    });
  const groups = Object.entries(reportGroups).map(([group, items]) => {
    const Icon = icons[group as keyof typeof icons];
    return {
      label: t(group),
      icon: <Icon size={16} aria-hidden />,
      items: items.map(slug => ({ id: slug, label: t(slug), path: href(slug) })),
    };
  });
  return (
    <ReportNav
      groups={groups}
      selectedKey={screen}
      navigationLabel={t('navigation')}
      header={header}
      controls={controls}
    >
      {children}
    </ReportNav>
  );
}

export function WebsiteReportNav({
  websiteId,
  children,
  header,
}: {
  websiteId: string;
  children: ReactNode;
  header: ReactNode;
}) {
  const { t, labels } = useMessages();
  const { pathname } = useNavigation();
  const { items, selectedKey, renderPath } = useWebsiteNavItems(websiteId);
  const settings = pathname.replace(/\/+$/, '').endsWith('/settings');
  const groups = [
    ...items.map(group => ({ ...group, icon: group.items[0].icon })),
    {
      label: t(labels.settings),
      icon: <Settings size={16} aria-hidden />,
      items: [{ id: 'settings', label: t(labels.settings), path: renderPath('/settings') }],
    },
  ];
  const sessionDetail = /\/(?:sessions|replays)\/[^/]+\/?$/.test(pathname);
  return (
    <ReportNav
      groups={groups}
      selectedKey={settings ? 'settings' : selectedKey}
      navigationLabel={t(labels.reports)}
      header={header}
      controls={
        !settings && (
          <AppReportControls
            websiteId={websiteId}
            allowDateFilter={
              !sessionDetail &&
              !['retention', 'segments', 'cohorts', 'realtime'].includes(selectedKey)
            }
            allowCompare={selectedKey === 'compare'}
          />
        )
      }
    >
      {children}
    </ReportNav>
  );
}

function ReportNav({
  groups,
  selectedKey,
  navigationLabel,
  children,
  controls,
  header,
}: {
  groups: {
    label: string;
    icon: ReactNode;
    items: { id: string; label: string; path: string }[];
  }[];
  selectedKey?: string;
  navigationLabel: string;
  children: ReactNode;
  controls?: ReactNode;
  header?: ReactNode;
}) {
  const t = useTranslations('appReportsNav');
  const activeGroup = groups.find(group => group.items.some(item => item.id === selectedKey));
  const siblings = activeGroup?.items ?? [];
  const hasSidebar = siblings.length > 1;
  return (
    <div className={styles.shell}>
      {header}
      <nav className={styles.tabs} aria-label={navigationLabel}>
        {groups.map(group => {
          const item = group.items.find(item => item.id === selectedKey) ?? group.items[0];
          return (
            <Link
              key={group.label}
              href={item.path}
              aria-current={group === activeGroup ? 'location' : undefined}
            >
              {group.icon}
              {group.label}
            </Link>
          );
        })}
      </nav>
      {controls}
      <div className={`${styles.reportBody} ${hasSidebar ? styles.columns : ''}`}>
        {hasSidebar && (
          <nav
            aria-label={t('section-navigation', { group: activeGroup.label })}
            className={styles.side}
          >
            {siblings.map(item => (
              <Link
                key={item.id}
                href={item.path}
                aria-current={item.id === selectedKey ? 'page' : undefined}
              >
                {item.label}
              </Link>
            ))}
          </nav>
        )}
        <div className={styles.content} key={selectedKey}>
          {children}
        </div>
      </div>
    </div>
  );
}
