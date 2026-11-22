import { beforeEach, expect, test, vi } from 'vitest';
import { render, screen } from '@/test/render';
import AcquisitionScreen, { type AcquisitionReport } from './Screen';

const mocks = vi.hoisted(() => ({
  mode: 'data' as 'data' | 'empty' | 'unidentified' | 'error' | 'loading',
  refetch: vi.fn(),
  request: vi.fn(),
}));

const report: AcquisitionReport = {
  identity: {
    unit: 'appUser',
    source: 'clientUser',
    unidentifiedEventsExcluded: true,
  },
  range: {
    startAt: '2026-11-01T00:00:00Z',
    endAt: '2026-11-20T00:00:00Z',
    timezone: 'UTC',
    testMode: false,
    totalSignals: 5,
    identifiedSignals: 4,
  },
  series: {
    hourly: [
      { bucket: '2026-11-19T23:00:00Z', activeUsers: 2, newUsers: 0 },
      { bucket: '2026-11-20T00:00:00Z', activeUsers: 2, newUsers: 1 },
    ],
    daily: [{ bucket: '2026-11-20T00:00:00Z', activeUsers: 2, newUsers: 1 }],
    weekly: [{ bucket: '2026-11-16T00:00:00Z', activeUsers: 2, newUsers: 1 }],
    monthly: [{ bucket: '2026-11-01T00:00:00Z', activeUsers: 2, newUsers: 1 }],
  },
  patterns: {
    hourOfDay: [],
    hourOfDay4Weeks: [{ bucket: '9', users: 1 }],
    dayOfWeek: [],
    dayOfWeek4Weeks: [{ bucket: '2', users: 1 }],
    weekpart: [{ bucket: 'Weekday', users: 1 }],
    weekpart4Weeks: [{ bucket: 'Weekday', users: 1 }],
  },
  breakdowns: {
    device: [{ name: 'mobile', users: 1 }],
    platform: [{ name: 'iOS', users: 1 }],
    region: [{ name: 'US-CA', users: 1 }],
    language: [{ name: 'en', users: 1 }],
  },
  availability: {
    userLocalTime: false,
    reason: "The SDK user's local timezone is not stored per signal.",
  },
  query: {
    milliseconds: 12.4,
    grains: {
      hourly: { unit: 'hour', window: 'last24Hours' },
      daily: { unit: 'day', window: 'selectedRange' },
    },
  },
};

vi.mock('@tanstack/react-query', async importOriginal => {
  const original = await importOriginal<typeof import('@tanstack/react-query')>();
  return {
    ...original,
    useQuery: (options: { queryFn: () => unknown }) => {
      mocks.request(options.queryFn);
      if (mocks.mode === 'loading') {
        return { isLoading: true, error: null, data: undefined, refetch: mocks.refetch };
      }
      if (mocks.mode === 'error') {
        return {
          isLoading: false,
          error: new Error('network'),
          data: undefined,
          refetch: mocks.refetch,
        };
      }
      const scoped = structuredClone(report);
      if (mocks.mode === 'empty') {
        scoped.range.totalSignals = 0;
        scoped.range.identifiedSignals = 0;
      }
      if (mocks.mode === 'unidentified') {
        scoped.range.identifiedSignals = 0;
      }
      return { isLoading: false, error: null, data: scoped, refetch: mocks.refetch };
    },
  };
});

vi.mock('@/components/hooks/useApi', () => ({
  useApi: () => ({ get: vi.fn() }),
}));

vi.mock('@/components/hooks/useDateParameters', () => ({
  useDateParameters: () => ({
    startAt: 1,
    endAt: 2,
    timezone: 'UTC',
  }),
}));

vi.mock('@/components/hooks/useNavigation', () => ({
  useNavigation: () => ({ query: {} }),
}));

beforeEach(() => {
  mocks.mode = 'data';
  mocks.refetch.mockClear();
  mocks.request.mockClear();
});

test('renders first-seen app-user reports without relabeling active users as new', () => {
  render(<AcquisitionScreen websiteId="website-1" />);

  expect(screen.getByText('First-seen app users')).toBeInTheDocument();
  expect(screen.getByRole('heading', { name: 'Hourly New Users' })).toBeInTheDocument();
  expect(screen.getByRole('heading', { name: 'Hourly Active vs New Users' })).toBeInTheDocument();
  expect(screen.getAllByText('User-local time is unavailable')).toHaveLength(2);
  expect(screen.getByText('80%')).toBeInTheDocument();
  expect(screen.getByText('12 ms')).toBeInTheDocument();
  expect(screen.getByText('iOS')).toBeInTheDocument();
});

test('reserves chart and summary structure without presenting invented values while loading', () => {
  mocks.mode = 'loading';
  const { container, rerender } = render(<AcquisitionScreen websiteId="website-1" />);
  expect(screen.getByRole('status', { name: 'Loading acquisition report' })).toBeVisible();
  expect(container.querySelectorAll('[data-slot="skeleton"]')).not.toHaveLength(0);
  expect(screen.queryByText('0%')).not.toBeInTheDocument();
  expect(screen.queryByText('No signals in this range')).not.toBeInTheDocument();

  mocks.mode = 'data';
  rerender(<AcquisitionScreen websiteId="website-1" />);
  expect(screen.queryByRole('status')).not.toBeInTheDocument();
  expect(screen.getByText('80%')).toBeVisible();
});

test('distinguishes no signals, missing identity, and request failure', () => {
  mocks.mode = 'empty';
  const { rerender } = render(<AcquisitionScreen websiteId="website-1" />);
  expect(screen.getByRole('heading', { name: 'No signals in this range' })).toBeInTheDocument();

  mocks.mode = 'unidentified';
  rerender(<AcquisitionScreen websiteId="website-1" />);
  expect(screen.getByRole('heading', { name: 'New users cannot be counted' })).toBeInTheDocument();
  expect(screen.getByText(/will not substitute sessions/i)).toBeInTheDocument();

  mocks.mode = 'error';
  rerender(<AcquisitionScreen websiteId="website-1" />);
  expect(
    screen.getByRole('heading', { name: 'Acquisition could not be loaded' }),
  ).toBeInTheDocument();
  expect(screen.queryByText('No signals in this range')).not.toBeInTheDocument();
});

test('offers a retry after an acquisition request fails', async () => {
  mocks.mode = 'error';
  const { user } = render(<AcquisitionScreen websiteId="website-1" />);

  await user.click(screen.getByRole('button', { name: 'Try again' }));

  expect(mocks.refetch).toHaveBeenCalledOnce();
});
