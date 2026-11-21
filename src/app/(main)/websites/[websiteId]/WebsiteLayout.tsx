'use client';
import { Column } from '@umami/react-zen';
import type { ReactNode } from 'react';
import { WebsiteProvider } from '@/app/(main)/websites/WebsiteProvider';
import { PageBody } from '@/components/common/PageBody';
import { useNavigation, useWebsite } from '@/components/hooks';
import { isNativeApp, isAppReportPath } from './app-metrics/report-layout';
import { WebsiteHeader } from './WebsiteHeader';

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
    <PageBody gap>
      <WebsiteHeader showActions />
      <Column>{children}</Column>
    </PageBody>
  );
}
