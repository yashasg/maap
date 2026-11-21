import { expect, test, vi } from 'vitest';
import { render, screen, within } from '@/test/render';
import { AppReportNav } from './AppReportNav';
import { AppReportHeader } from './AppReportHeader';
import { isNativeApp, isAppReportPath } from './report-layout';

vi.mock('@/components/hooks/queries/useAnalyticsQuery', () => ({
  useAnalyticsQuery: () => ({
    data: { summary: { users: 12, mostUsedSdk: 'Swift', errors: null } },
  }),
}));

test('the app home and report deep links use the same shell, not website or legacy routes', () => {
  const id = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa';
  const appShell = (path: string, domain: string) =>
    isAppReportPath(path, id) && isNativeApp(domain);
  expect(appShell(`/websites/${id}`, 'app.ios')).toBe(true);
  expect(appShell(`/teams/${id}/websites/${id}/app-reports/versions`, 'app.android')).toBe(true);
  expect(appShell(`/websites/${id}`, 'example.org')).toBe(false);
  expect(appShell(`/websites/${id}/events`, 'app.ios')).toBe(false);
});

test('the app identity and real summary precede the report navigation', () => {
  render(
    <AppReportNav
      websiteId="app"
      screen="overview"
      header={<AppReportHeader websiteId="app" name="Field Notes" />}
    >
      Daily Active Users
    </AppReportNav>,
  );
  const header = screen.getByRole('banner');
  expect(within(header).getByRole('heading', { name: 'Field Notes' })).toBeVisible();
  expect(within(header).getByText('Swift')).toBeVisible();
  expect(within(header).getByText('Not available')).toBeVisible();
  expect(header.nextElementSibling).toBe(screen.getByRole('navigation', { name: 'App reports' }));
});

test('every Metrics sidebar item is a scoped link that preserves Test Mode and date', () => {
  const team = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa';
  const root = `/teams/${team}/websites/example/app-reports/`;
  render(
    <AppReportNav websiteId="example" screen="versions">
      Report
    </AppReportNav>,
    {
      route: `${root}versions?testMode=true&date=30day&page=3`,
    },
  );
  const sidebar = within(screen.getByRole('navigation', { name: 'Metrics reports' }));
  for (const name of ['Devices', 'Versions', 'Errors', 'Localization', 'Accessibility']) {
    const link = sidebar.getByRole('link', { name });
    const url = new URL(link.getAttribute('href')!, 'http://localhost');
    expect(url.pathname).toBe(root + name.toLowerCase());
    expect(Object.fromEntries(url.searchParams)).toEqual({ testMode: 'true', date: '30day' });
    expect(link.getAttribute('aria-current')).toBe(name === 'Versions' ? 'page' : null);
  }
  const top = within(screen.getByRole('navigation', { name: 'App reports' }));
  expect(top.getAllByRole('link')).toHaveLength(8);
  expect(top.getByRole('link', { name: 'Explore' }).getAttribute('href')).toContain(
    '/recent-events?',
  );
});
