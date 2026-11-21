import { beforeEach, expect, test, vi } from 'vitest';
import { render, screen } from '@/test/render';
import ActivationScreen, { type ActivationReport } from './Screen';

const mocks = vi.hoisted(() => ({
  report: null as ActivationReport | null,
  refetch: vi.fn(),
}));

vi.mock('@/components/hooks/queries/useAnalyticsQuery', () => ({
  useAnalyticsQuery: () => ({
    data: mocks.report,
    error: null,
    isLoading: false,
    refetch: mocks.refetch,
  }),
}));

function report(overrides: Partial<ActivationReport> = {}): ActivationReport {
  return {
    definition: {
      rule: 'At least 5 minutes accumulated total usage time within the first 5 sessions.',
      thresholdSeconds: 300,
      sessionLimit: 5,
      source: 'TelemetryDeck Activation Analytics documentation',
      configurable: false,
      calculation: 'Uses the earliest stored SDK signal that proves the threshold was reached.',
    },
    configuration: {
      status: 'ready',
      coreReady: true,
      missingKeys: [],
      requiredAllowlist: [
        'TelemetryDeck.Retention.totalSessionsCount',
        'TelemetryDeck.Retention.averageSessionSeconds',
      ],
      environmentVariable: 'TD_ALLOWED_KEYS',
      historicalBackfill: false,
    },
    scope: {
      startAt: '2026-11-01T00:00:00.000Z',
      endAt: '2026-11-20T23:59:59.999Z',
      timezone: 'UTC',
      testMode: false,
      totalSignals: 9,
      identifiedSignals: 8,
      activationInputSignals: 8,
      activatedUsers: 1,
    },
    series: {
      hourly: [{ key: '2026-11-20T12:00', label: '2026-11-20T12:00', count: 1 }],
      daily: [{ key: '2026-11-20', label: '2026-11-20', count: 1 }],
      weekly: [{ key: '2026-11-17', label: '2026-11-17', count: 1 }],
      monthly: [{ key: '2026-11-01', label: '2026-11-01', count: 1 }],
    },
    patterns: {
      hourOfDay: [{ key: '13', label: '13', count: 1 }],
      hourOfDay4Weeks: [{ key: '12', label: '12', count: 1 }],
      dayOfWeek: [{ key: '5', label: '5', count: 1 }],
      dayOfWeek4Weeks: [{ key: '5', label: '5', count: 1 }],
      weekpart: [{ key: 'Weekday', label: 'Weekday', count: 1 }],
      weekpart4Weeks: [{ key: 'Weekday', label: 'Weekday', count: 1 }],
    },
    breakdowns: {
      device: [{ key: 'mobile', label: 'mobile', count: 1 }],
      platform: [{ key: 'iOS', label: 'iOS', count: 1 }],
      region: [{ key: 'US', label: 'US', count: 1 }],
      language: [{ key: 'en', label: 'en', count: 1 }],
    },
    ...overrides,
  };
}

beforeEach(() => {
  mocks.report = report();
  mocks.refetch.mockReset();
});

test('renders the documented activation rule and deduplicated report count', () => {
  render(<ActivationScreen websiteId="website-1" />);

  expect(screen.getByText(/At least 5 minutes accumulated total usage time/)).toBeInTheDocument();
  expect(screen.getByText('Activated users')).toBeInTheDocument();
  expect(screen.getAllByText('Friday')).toHaveLength(2);
  expect(
    screen.getByRole('heading', { name: 'Activated Users by Preferred Region' }),
  ).toBeInTheDocument();
  expect(screen.queryByText(/first seen/i)).not.toBeInTheDocument();
});

test('shows the real ingestion configuration path instead of zero-valued charts', () => {
  mocks.report = report({
    configuration: {
      status: 'configuration-missing',
      coreReady: false,
      missingKeys: [
        'TelemetryDeck.Retention.totalSessionsCount',
        'TelemetryDeck.Retention.averageSessionSeconds',
      ],
      requiredAllowlist: [
        'TelemetryDeck.Retention.totalSessionsCount',
        'TelemetryDeck.Retention.averageSessionSeconds',
      ],
      environmentVariable: 'TD_ALLOWED_KEYS',
      historicalBackfill: false,
    },
    scope: {
      ...report().scope,
      activationInputSignals: 0,
      activatedUsers: 0,
    },
  });

  render(<ActivationScreen websiteId="website-1" />);

  expect(
    screen.getByRole('heading', { name: 'Retain the SDK activation fields' }),
  ).toBeInTheDocument();
  expect(screen.getByText('TD_ALLOWED_KEYS')).toBeInTheDocument();
  expect(screen.getByTestId('activation-allowlist')).toHaveTextContent(
    'TelemetryDeck.Retention.totalSessionsCount',
  );
  expect(screen.queryByRole('heading', { name: 'Daily Activated Users' })).not.toBeInTheDocument();
});
