'use client';
import dynamic from 'next/dynamic';
import type { ComponentType } from 'react';
import NotFound from '@/app/not-found';
import { useWebsite } from '@/components/hooks';
import { isNativeApp } from './report-layout';
import { WebsitePage } from '../WebsitePage';
import { AppReportControls } from './AppReportControls';
import { AppReportHeader } from './AppReportHeader';
import { AppReportNav } from './AppReportNav';
import type { AppReport } from './sections';

const screens = {
  overview: dynamic(() => import('./overview/Screen')),
  acquisition: dynamic(() => import('./acquisition/Screen')),
  activation: dynamic(() => import('./activation/Screen')),
  retention: dynamic(() => import('./retention/Screen')),
  revenue: dynamic(() => import('./revenue/Screen')),
  devices: dynamic(() => import('./devices/Screen')),
  versions: dynamic(() => import('./versions/Screen')),
  errors: dynamic(() => import('./errors/Screen')),
  localization: dynamic(() => import('./localization/Screen')),
  accessibility: dynamic(() => import('./accessibility/Screen')),
  'recent-events': dynamic(() => import('./recent-events/Screen')),
  'event-types': dynamic(() => import('./event-types/Screen')),
  parameters: dynamic(() => import('./parameters/Screen')),
  playground: dynamic(() => import('./playground/Screen')),
  dashboards: dynamic(() => import('./dashboards/Screen')),
  notebooks: dynamic(() => import('./notebooks/Screen')),
  settings: dynamic(() => import('./settings/Screen')),
  setup: dynamic(() => import('./setup/Screen')),
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
      <Screen websiteId={websiteId} />
    </AppReportNav>
  );
}
