import { expect, test, vi } from 'vitest';
import { getTestRouter } from '@/test/navigation';
import { render, screen, within } from '@/test/render';
import { WebsiteControls } from '../WebsiteControls';
import { AppReportHeader } from './AppReportHeader';
import { AppReportNav, WebsiteReportNav } from './AppReportNav';
import { isPropertyPath } from './report-layout';

vi.mock('@/components/hooks/queries/useWebsiteStatsQuery', () => ({
  useWebsiteStatsQuery: () => ({
    data: { visitors: 23, visits: 42, pageviews: 87 },
  }),
}));

vi.mock('@/components/input/WebsiteDateFilter', () => ({
  WebsiteDateFilter: () => <button type="button">Report date</button>,
}));

vi.mock('@/components/hooks/queries/useAnalyticsQuery', () => ({
  useAnalyticsQuery: () => ({
    data: { summary: { users: 12, mostUsedSdk: 'Swift', errors: null } },
  }),
}));

test('every property route uses the shared shell, without matching admin, share or another property', () => {
  const id = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa';
  expect(isPropertyPath(`/websites/${id}`, id)).toBe(true);
  expect(isPropertyPath(`/teams/${id}/websites/${id}/app-reports/versions`, id)).toBe(true);
  expect(isPropertyPath(`/websites/${id}/events`, id)).toBe(true);
  expect(isPropertyPath(`/websites/${id}/sessions/session-id`, id)).toBe(true);
  expect(isPropertyPath(`/websites/${id}/settings/`, id)).toBe(true);
  expect(isPropertyPath(`/settings/websites/${id}`, id)).toBe(false);
  expect(isPropertyPath(`/share/${id}`, id)).toBe(false);
  expect(isPropertyPath(`/websites/${id}-other`, id)).toBe(false);
  expect(isPropertyPath(`/websites/${id}`)).toBe(false);
});

test('website reports share the app shell with real website metrics and scoped navigation', async () => {
  const team = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa';
  const root = `/teams/${team}/websites/example`;
  const { user } = render(
    <WebsiteReportNav
      websiteId="example"
      header={<AppReportHeader websiteId="example" name="Kashi Works" nativeApp={false} />}
    >
      Website events
    </WebsiteReportNav>,
    { route: `${root}/events?date=30day&testMode=true&page=3&search=old` },
  );
  const header = screen.getByRole('banner');
  expect(within(header).getByRole('heading', { name: 'Kashi Works' })).toBeVisible();
  expect(within(header).getByText('Visitors')).toBeVisible();
  expect(within(header).getByText('23')).toBeVisible();
  expect(within(header).queryByText('Most used SDK')).not.toBeInTheDocument();
  const tabs = screen.getByRole('navigation', { name: 'Reports' });
  expect(header.nextElementSibling).toBe(tabs);
  expect(within(tabs).getByRole('link', { name: 'Traffic' })).toHaveAttribute(
    'aria-current',
    'location',
  );
  const sidebar = within(screen.getByRole('navigation', { name: 'Traffic reports' }));
  expect(sidebar.getByRole('link', { name: 'Events' })).toHaveAttribute('aria-current', 'page');
  for (const [name, path] of [
    ['Overview', ''],
    ['Events', '/events'],
    ['Sessions', '/sessions'],
  ]) {
    const url = new URL(
      sidebar.getByRole('link', { name }).getAttribute('href')!,
      'http://localhost',
    );
    expect(url.pathname).toBe(`${root}${path}`);
    expect(Object.fromEntries(url.searchParams)).toEqual({ date: '30day', testMode: 'true' });
  }
  expect(within(tabs).getByRole('link', { name: 'Settings' })).toHaveAttribute(
    'href',
    `${root}/settings?date=30day&testMode=true`,
  );
  expect(screen.getByRole('button', { name: 'Test Mode' })).toHaveAttribute('aria-pressed', 'true');
  await user.click(screen.getByRole('button', { name: 'Test Mode' }));
  expect(getTestRouter().push).toHaveBeenLastCalledWith(`${root}/events?date=30day&search=old`);
});

test('website session deep links retain their section navigation without a misleading date filter', () => {
  render(
    <WebsiteReportNav websiteId="example" header={null}>
      Session activity
    </WebsiteReportNav>,
    { route: '/websites/example/sessions/session-id' },
  );
  const sidebar = within(screen.getByRole('navigation', { name: 'Traffic reports' }));
  expect(sidebar.getByRole('link', { name: 'Sessions' })).toHaveAttribute('aria-current', 'page');
  expect(screen.queryByRole('button', { name: 'Report date' })).not.toBeInTheDocument();
});

test.each([
  ['/websites/example/events', false],
  ['/share/example', true],
])('date controls are not duplicated or removed from public shares: %s', (route, visible) => {
  render(<WebsiteControls websiteId="example" allowFilter={false} />, { route });
  expect(Boolean(screen.queryByRole('button', { name: 'Report date' }))).toBe(visible);
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
