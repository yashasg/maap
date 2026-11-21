import { beforeEach, expect, test, vi } from 'vitest';
import { render, screen } from '@/test/render';
import OverviewScreen from './Screen';

const refetch = vi.fn();
const settings = vi.hoisted(() => ({
  overviewLayout: 'application' as 'application' | 'website',
  colorScheme: 'default' as 'default' | 'vibrant' | 'high-contrast' | 'emo',
}));

vi.mock('@/components/hooks/queries/useAppSettingsQuery', () => ({
  useAppSettingsQuery: () => ({
    data: {
      websiteId: 'website-1',
      canWrite: true,
      revenueDisplayMode: 'none',
      ...settings,
    },
    error: null,
    isLoading: false,
    isFetching: false,
  }),
}));

vi.mock('@/components/hooks/queries/useAnalyticsQuery', () => ({
  useAnalyticsQuery: () => ({
    data: {
      identity: 'clientUser',
      range: {
        startAt: '2026-11-18T00:00:00.000Z',
        endAt: '2026-11-20T23:59:59.999Z',
        timezone: 'America/Los_Angeles',
      },
      summary: {
        users: 3,
        usersLastHour: 1,
        events: 11,
        errors: null,
        mostUsedSdk: 'TelemetryDeckSwift',
      },
      dailyUsers: [
        { x: '2026-11-18', y: 1 },
        { x: '2026-11-19', y: 2 },
        { x: '2026-11-20', y: 2 },
      ],
      retention: [
        { day: 1, eligibleUsers: 2, returningUsers: 1, percentage: 50 },
        { day: 7, eligibleUsers: 0, returningUsers: 0, percentage: null },
        { day: 14, eligibleUsers: 0, returningUsers: 0, percentage: null },
      ],
      dimensions: {
        sdkName: [{ value: 'TelemetryDeckSwift', users: 3 }],
        sdkVersion: [{ value: '2.5.0', users: 3 }],
        appVersion: [
          { value: '2.0.0', users: 2 },
          { value: '1.9.0', users: 1 },
        ],
        buildNumber: [
          { value: '200', users: 2 },
          { value: '190', users: 1 },
        ],
        systemVersion: [
          { value: 'iOS 18.4.1', users: 2 },
          { value: 'iOS 18.3.2', users: 1 },
        ],
      },
    },
    error: null,
    isLoading: false,
    isFetching: false,
    refetch,
  }),
}));

beforeEach(() => {
  settings.overviewLayout = 'application';
  settings.colorScheme = 'default';
});

test('renders identified-user metrics and never labels sessions as users', () => {
  render(<OverviewScreen websiteId="website-1" />);

  expect(screen.getByRole('heading', { name: 'Daily Active Users' })).toBeInTheDocument();
  expect(screen.getByText('Identified users in the last hour')).toBeInTheDocument();
  expect(screen.getByText('Rolling 60 minutes')).toBeInTheDocument();
  expect(screen.queryByText('Most used SDK')).not.toBeInTheDocument();
  expect(screen.queryByText(/^Sessions$/)).not.toBeInTheDocument();
  expect(screen.getByText('50.0%')).toBeInTheDocument();
});

test('switches app and system version groupings without changing identity totals', async () => {
  const { user } = render(<OverviewScreen websiteId="website-1" />);

  await user.click(screen.getByRole('button', { name: 'Builds' }));
  expect(screen.getByText('200')).toBeInTheDocument();

  await user.click(screen.getByRole('button', { name: 'Minor' }));
  expect(screen.getByText('iOS 18.4')).toBeInTheDocument();
  expect(screen.getByText('iOS 18.3')).toBeInTheDocument();
  expect(screen.getAllByText('3')).not.toHaveLength(0);
});

test('applies the persisted overview layout and color scheme as presentation only', () => {
  settings.overviewLayout = 'website';
  settings.colorScheme = 'emo';

  render(<OverviewScreen websiteId="website-1" />);

  const overview = screen.getByRole('region', { name: 'Overview' });
  expect(overview).toHaveAttribute('data-overview-layout', 'website');
  expect(overview).toHaveAttribute('data-color-scheme', 'emo');
  expect(screen.getByText('Identified users in the last hour')).toBeInTheDocument();
  expect(screen.queryByText(/^Sessions$/)).not.toBeInTheDocument();
});
