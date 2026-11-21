import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, expect, test, vi } from 'vitest';
import RevenueScreen, { getRevenueViewState } from './Screen';

const statsQuery = {
  data: undefined,
  error: null,
  isLoading: false,
  refetch: vi.fn(),
};
const settingsQuery = {
  data: {
    websiteId: 'website-1',
    canWrite: true,
    overviewLayout: 'application',
    colorScheme: 'default',
    revenueDisplayMode: 'none',
  },
  error: null,
  isLoading: false,
  refetch: vi.fn(),
};
const mutate = vi.fn();
const touch = vi.fn();
const push = vi.fn();

vi.mock('@/components/hooks', () => ({
  useNavigation: () => ({
    router: { push },
    renderUrl: (path: string) => `${path}?date=30d&timezone=UTC&testMode=true`,
  }),
  useAppSettingsQuery: () => settingsQuery,
  useAppSettingsMutation: () => ({
    mutate,
    touch,
    isPending: false,
    error: null,
  }),
  useRevenueStatsQuery: () => statsQuery,
  useTimezone: () => ({ timezone: 'UTC' }),
  useMessages: () => ({
    t: (value: string) => value,
    labels: { maximize: 'Maximize' },
  }),
}));

vi.mock('@/components/hooks/useDateRange', () => ({
  useDateRange: () => ({
    dateRange: {
      startDate: new Date('2026-10-01T00:00:00Z'),
      endDate: new Date('2026-10-31T23:59:59Z'),
      unit: 'day',
    },
  }),
}));

vi.mock('@/components/input/CurrencySelect', () => ({
  CurrencySelect: ({ value }: { value: string }) => <div>Currency: {value}</div>,
}));

vi.mock('../../(reports)/revenue/Revenue', () => ({
  Revenue: () => <div>Real revenue report</div>,
}));

beforeEach(() => {
  localStorage.clear();
  statsQuery.data = undefined;
  statsQuery.error = null;
  statsQuery.isLoading = false;
  statsQuery.refetch.mockReset();
  settingsQuery.data.canWrite = true;
  settingsQuery.data.revenueDisplayMode = 'none';
  settingsQuery.error = null;
  settingsQuery.isLoading = false;
  settingsQuery.refetch.mockReset();
  mutate.mockReset();
  touch.mockReset();
  push.mockReset();
});

test('keeps setup, loading, failure, zero and populated states distinct', () => {
  expect(getRevenueViewState({ enabled: false, isLoading: true })).toBe('loading');
  expect(
    getRevenueViewState({ enabled: false, isLoading: false, error: new Error('failed') }),
  ).toBe('error');
  expect(getRevenueViewState({ enabled: false, isLoading: false, data: { count: 1 } })).toBe(
    'setup',
  );
  expect(getRevenueViewState({ enabled: true, isLoading: false, data: { count: 0 } })).toBe('zero');
  expect(getRevenueViewState({ enabled: true, isLoading: false, data: { count: 1 } })).toBe(
    'report',
  );
});

test('shows the genuine SDK setup path before revenue is configured', () => {
  statsQuery.data = {
    sum: 0,
    count: 0,
    unique_count: 0,
  };

  render(<RevenueScreen websiteId="website-1" />);

  expect(screen.getByText('Set up revenue tracking')).toBeInTheDocument();
  expect(screen.getByText(/TelemetryDeck.signal/)).toBeInTheDocument();
  expect(screen.queryByText('Real revenue report')).not.toBeInTheDocument();

  fireEvent.click(screen.getByRole('button', { name: 'Enable revenue report' }));
  expect(mutate).toHaveBeenCalledWith(
    {
      overviewLayout: 'application',
      colorScheme: 'default',
      revenueDisplayMode: 'revenuecat',
    },
    expect.objectContaining({ onSuccess: expect.any(Function) }),
  );
  const [, options] = mutate.mock.calls[0];
  options.onSuccess();
  expect(touch).toHaveBeenCalledWith('app-settings:website-1');

  fireEvent.click(screen.getByRole('button', { name: 'Review app SDK setup' }));
  expect(push).toHaveBeenCalledWith(
    '/websites/website-1/app-reports/setup?date=30d&timezone=UTC&testMode=true',
  );
});

test('shows an explicit zero when persisted revenue reporting is enabled', () => {
  settingsQuery.data.revenueDisplayMode = 'revenuecat';
  statsQuery.data = {
    sum: 0,
    count: 0,
    unique_count: 0,
  };

  render(<RevenueScreen websiteId="website-1" />);

  expect(screen.getByText('Revenue reporting is enabled')).toBeInTheDocument();
  expect(screen.getByText(/This is a real zero/)).toBeInTheDocument();
  expect(screen.getByText('Real revenue report')).toBeInTheDocument();
});

test('does not present a failed request as zero revenue', () => {
  settingsQuery.data.revenueDisplayMode = 'revenuecat';
  statsQuery.error = new Error('unavailable');

  render(<RevenueScreen websiteId="website-1" />);

  expect(screen.getByText('Revenue could not be loaded')).toBeInTheDocument();
  expect(screen.queryByText(/This is a real zero/)).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
  expect(settingsQuery.refetch).toHaveBeenCalledOnce();
  expect(statsQuery.refetch).toHaveBeenCalledOnce();
});

test('respects app-settings write permission', () => {
  settingsQuery.data.canWrite = false;

  render(<RevenueScreen websiteId="website-1" />);

  expect(screen.getByRole('button', { name: 'Enable revenue report' })).toBeDisabled();
  expect(screen.getByText(/do not have permission/)).toBeInTheDocument();
});
