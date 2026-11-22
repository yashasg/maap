'use client';
import { type ComponentType, lazy, Suspense } from 'react';
import NotFound from '@/app/not-found';
import { useWebsite } from '@/components/hooks';
import { WebsitePage } from '../WebsitePage';
import { AppReportControls } from './AppReportControls';
import { AppReportHeader } from './AppReportHeader';
import { AppReportNav } from './AppReportNav';
import { ReportSkeleton } from './ReportSkeleton';
import { isNativeApp } from './report-layout';
import type { AppReport } from './sections';

const screens = {
  overview: lazy(() => import('./overview/Screen')),
  acquisition: lazy(() => import('./acquisition/Screen')),
  activation: lazy(() => import('./activation/Screen')),
  retention: lazy(() => import('./retention/Screen')),
  revenue: lazy(() => import('./revenue/Screen')),
  devices: lazy(() => import('./devices/Screen')),
  versions: lazy(() => import('./versions/Screen')),
  errors: lazy(() => import('./errors/Screen')),
  localization: lazy(() => import('./localization/Screen')),
  accessibility: lazy(() => import('./accessibility/Screen')),
  'recent-events': lazy(() => import('./recent-events/Screen')),
  'event-types': lazy(() => import('./event-types/Screen')),
  parameters: lazy(() => import('./parameters/Screen')),
  playground: lazy(() => import('./playground/Screen')),
  dashboards: lazy(() => import('./dashboards/Screen')),
  notebooks: lazy(() => import('./notebooks/Screen')),
  settings: lazy(() => import('./settings/Screen')),
  setup: lazy(() => import('./setup/Screen')),
} satisfies Record<AppReport, ComponentType<{ websiteId: string }>>;

export function PropertyHome({ websiteId }: { websiteId: string }) {
  const { domain } = useWebsite();
  return isNativeApp(domain) ? (
    <AppReportPage websiteId={websiteId} screen="overview" />
  ) : (
    <WebsitePage websiteId={websiteId} />
  );
}

export function AppReportPage({ websiteId, screen }: { websiteId: string; screen: AppReport }) {
  const { domain, name } = useWebsite();
  if (!isNativeApp(domain)) return <NotFound />;
  const Screen = screens[screen];
  return (
    <AppReportNav
      header={<AppReportHeader websiteId={websiteId} name={name} />}
      websiteId={websiteId}
      screen={screen}
      controls={
        !['setup', 'settings'].includes(screen) && <AppReportControls websiteId={websiteId} />
      }
    >
      <Suspense key={`${websiteId}:${screen}`} fallback={<ReportSkeleton screen={screen} />}>
        <Screen websiteId={websiteId} />
      </Suspense>
    </AppReportNav>
  );
}
