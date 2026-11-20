import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { beforeEach, describe, expect, test, vi } from 'vitest';
import RetentionScreen, { type RetentionApiResponse } from './Screen';

const mocks = vi.hoisted(() => ({
  get: vi.fn(),
  date: {
    startAt: Date.UTC(2026, 9, 1),
    endAt: Date.UTC(2026, 9, 31),
    timezone: 'America/Los_Angeles',
  },
  navigation: {
    query: { testMode: 'true' },
  },
}));

vi.mock('@/components/hooks/useApi', () => ({
  useApi: () => ({ get: mocks.get }),
}));

vi.mock('@/components/hooks/useDateParameters', () => ({
  useDateParameters: () => mocks.date,
}));

vi.mock('@/components/hooks/useNavigation', () => ({
  useNavigation: () => mocks.navigation,
}));

const report: RetentionApiResponse = {
  criteria: {
    verified: false,
    rule: 'An identified user has activity in the bucket and at least one earlier signal.',
  },
  range: {
    startAt: '2026-10-01T00:00:00Z',
    endAt: '2026-10-31T23:59:59Z',
    timezone: 'America/Los_Angeles',
    testMode: true,
  },
  scope: {
    totalSignals: 10,
    identifiedSignals: 8,
    returningUsers: 2,
  },
  series: {
    hourly: [
      { bucket: '2026-10-01T09:00:00Z', returningUsers: 1 },
      { bucket: '2026-10-01T10:00:00Z', returningUsers: 2 },
    ],
    daily: [{ bucket: '2026-10-02T00:00:00Z', returningUsers: 2 }],
    weekly: [{ bucket: '2026-10-05T00:00:00Z', returningUsers: 2 }],
    monthly: [{ bucket: '2026-10-01T00:00:00Z', returningUsers: 2 }],
  },
  patterns: {
    hourOfDay: [{ bucket: '9', users: 1 }],
    hourOfDayFourWeeks: [{ bucket: '9', users: 2 }],
    dayOfWeek: [{ bucket: '1', users: 2 }],
    dayOfWeekFourWeeks: [{ bucket: '1', users: 2 }],
    weekpart: [{ bucket: 'Weekday', users: 2 }],
    weekpartFourWeeks: [{ bucket: 'Weekday', users: 2 }],
  },
  breakdowns: {
    device: [{ name: 'Mobile', users: 2 }],
    platform: [{ name: 'iOS', users: 2 }],
    region: [{ name: 'US', users: 2 }],
    language: [{ name: 'English', users: 2 }],
  },
  availability: {
    userLocalTime: false,
    preferredRegion: false,
    reason: "The SDK user's local timezone and preferred region are not stored per signal.",
  },
};

function Wrapper({ children }: { children: ReactNode }) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

beforeEach(() => {
  mocks.get.mockReset().mockResolvedValue(report);
});

describe('RetentionScreen', () => {
  test('requests real returning-user counts with the selected range and Test Mode scope', async () => {
    render(<RetentionScreen websiteId="website-1" />, { wrapper: Wrapper });

    expect(await screen.findByText('Parity criteria unverified')).toBeInTheDocument();
    expect(screen.getByText('2', { selector: 'dd' })).toBeInTheDocument();
    expect(screen.getByText(/does not reuse session-cohort percentages/i)).toBeInTheDocument();
    expect(screen.getByText(/does not substitute IP-derived geography/i)).toBeInTheDocument();

    await waitFor(() =>
      expect(mocks.get).toHaveBeenCalledWith('/websites/website-1/app-reports/retention', {
        startAt: mocks.date.startAt,
        endAt: mocks.date.endAt,
        timezone: mocks.date.timezone,
        testMode: 'true',
      }),
    );
  });
});
