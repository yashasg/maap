import type { ReactNode } from 'react';
import { expect, test, vi } from 'vitest';
import { render, screen } from '@/test/render';
import { SpaRouteView } from './SpaRouteView';

vi.mock('./websites/[websiteId]/WebsiteLayout', () => ({
  WebsiteLayout: ({ children }: { children: ReactNode }) => children,
}));
vi.mock('./websites/[websiteId]/app-metrics/AppReportPage', () => ({
  PropertyHome: () => null,
  AppReportPage: ({ screen }: { screen: string }) => <h1>App report {screen}</h1>,
}));
vi.mock('./websites/[websiteId]/errors/[issueId]/ErrorIssuePage', () => ({
  ErrorIssuePage: ({ issueId }: { issueId: string }) => <h1>Issue {issueId}</h1>,
}));
vi.mock('./websites/[websiteId]/(reports)/performance/PerformancePage', () => ({
  PerformancePage: () => <h1>Core Web Vitals</h1>,
}));

const website = '/websites/aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa';
test.each([
  ['performance', 'Core Web Vitals'],
  ['errors/issue-1', 'Issue issue-1'],
  ['app-reports/accessibility', 'App report accessibility'],
  ['app-reports/recent-events', 'App report recent-events'],
])('renders the requested %s view from the static shell', (path, title) => {
  render(<SpaRouteView>Website list</SpaRouteView>, { route: `${website}/${path}` });
  expect(screen.getByRole('heading', { name: title })).toBeVisible();
});
