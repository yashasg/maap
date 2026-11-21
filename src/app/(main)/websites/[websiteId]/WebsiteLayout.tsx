'use client';
import type { ReactNode } from 'react';
import { WebsiteProvider } from '@/app/(main)/websites/WebsiteProvider';
import { useNavigation, useWebsite } from '@/components/hooks';
import { AppReportHeader } from './app-metrics/AppReportHeader';
import { WebsiteReportNav } from './app-metrics/AppReportNav';
import { isAppReportPath, isNativeApp } from './app-metrics/report-layout';

export function WebsiteLayout({ websiteId, children }: { websiteId: string; children: ReactNode }) {
  return (
    <WebsiteProvider websiteId={websiteId}>
      <PropertyBody>{children}</PropertyBody>
    </WebsiteProvider>
  );
}

function PropertyBody({ children }: { children: ReactNode }) {
  const website = useWebsite();
  const { pathname } = useNavigation();
  if (isNativeApp(website.domain) && isAppReportPath(pathname, website.id)) {
    return <main style={{ width: '100%', minWidth: 0 }}>{children}</main>;
  }
  return (
    <main style={{ width: '100%', minWidth: 0 }}>
      <WebsiteReportNav
        websiteId={website.id}
        header={
          <AppReportHeader
            websiteId={website.id}
            name={website.name}
            nativeApp={isNativeApp(website.domain)}
          />
        }
      >
        {children}
      </WebsiteReportNav>
    </main>
  );
}
