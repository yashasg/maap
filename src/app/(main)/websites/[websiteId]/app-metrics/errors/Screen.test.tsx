import { render, screen } from '@testing-library/react';
import { beforeEach, expect, test, vi } from 'vitest';
import type { EventDataSeriesPoint } from '@/lib/types';
import ErrorsScreen, { buildErrorReport } from './Screen';

const usePropertySeriesQueryMock = vi.fn();

vi.mock('@/components/hooks', () => ({
  useDateParameters: () => ({
    startAt: Date.parse('2026-11-16T00:00:00Z'),
    endAt: Date.parse('2026-11-18T00:00:00Z'),
    startDate: new Date('2026-11-16T00:00:00Z'),
    endDate: new Date('2026-11-18T00:00:00Z'),
    unit: 'day',
    timezone: 'UTC',
  }),
  useLocale: () => ({ locale: 'en-US', dateLocale: undefined }),
  usePropertySeriesQuery: (...args: unknown[]) => usePropertySeriesQueryMock(...args),
}));

vi.mock('@/components/charts/BarChart', () => ({
  BarChart: ({ chartData }: any) => (
    <div data-testid="history-total">
      {chartData.datasets[0].data.reduce((sum: number, point: { y: number }) => sum + point.y, 0)}
    </div>
  ),
}));

vi.mock('@/components/metrics/ListTable', () => ({
  ListTable: ({ data }: any) => (
    <div data-testid="frequency-total">
      {data.reduce((sum: number, row: { count: number }) => sum + row.count, 0)}
    </div>
  ),
}));

const rowsByCategory: Record<string, EventDataSeriesPoint[]> = {
  overall: [
    { x: 'Import.decode', t: '2026-11-16T00:00:00Z', y: 2 },
    { x: 'Import.decode', t: '2026-11-17T00:00:00Z', y: 1 },
    { x: 'Form.invalid', t: '2026-11-17T00:00:00Z', y: 2 },
    { x: 'State.invalid', t: '2026-11-17T00:00:00Z', y: 1 },
    { x: 'Legacy.uncategorized', t: '2026-11-17T00:00:00Z', y: 1 },
  ],
  'thrown-exception': [
    { x: 'Import.decode', t: '2026-11-16T00:00:00Z', y: 2 },
    { x: 'Import.decode', t: '2026-11-17T00:00:00Z', y: 1 },
  ],
  'user-input': [{ x: 'Form.invalid', t: '2026-11-17T00:00:00Z', y: 2 }],
  'app-state': [{ x: 'State.invalid', t: '2026-11-17T00:00:00Z', y: 1 }],
};

beforeEach(() => {
  usePropertySeriesQueryMock.mockReset();
  usePropertySeriesQueryMock.mockImplementation(
    (
      _source: string,
      _websiteId: string,
      _propertyName: string,
      filters: Array<{ value: string }>,
    ) => {
      const category = filters[0]?.value ?? 'overall';
      return {
        data: rowsByCategory[category],
        error: null,
        isLoading: false,
        isFetching: false,
        refetch: vi.fn(),
      };
    },
  );
});

test('reconciles frequency and history totals from the same complete aggregate', () => {
  const report = buildErrorReport(rowsByCategory.overall);

  expect(report.total).toBe(7);
  expect(report.frequency).toEqual([
    { label: 'Import.decode', count: 3, percent: (3 / 7) * 100 },
    { label: 'Form.invalid', count: 2, percent: (2 / 7) * 100 },
    { label: 'Legacy.uncategorized', count: 1, percent: (1 / 7) * 100 },
    { label: 'State.invalid', count: 1, percent: (1 / 7) * 100 },
  ]);
  expect(report.history.reduce((sum, point) => sum + point.y, 0)).toBe(report.total);
});

test('queries only the documented native signal and category values', () => {
  render(<ErrorsScreen websiteId="website-1" />);

  expect(usePropertySeriesQueryMock).toHaveBeenCalledTimes(4);
  expect(usePropertySeriesQueryMock.mock.calls).toEqual([
    ['event', 'website-1', 'TelemetryDeck.Error.id', [], 'TelemetryDeck.Error.occurred'],
    [
      'event',
      'website-1',
      'TelemetryDeck.Error.id',
      [
        {
          propertyName: 'TelemetryDeck.Error.category',
          dataType: 1,
          operator: 'eq',
          value: 'thrown-exception',
        },
      ],
      'TelemetryDeck.Error.occurred',
    ],
    [
      'event',
      'website-1',
      'TelemetryDeck.Error.id',
      [
        {
          propertyName: 'TelemetryDeck.Error.category',
          dataType: 1,
          operator: 'eq',
          value: 'user-input',
        },
      ],
      'TelemetryDeck.Error.occurred',
    ],
    [
      'event',
      'website-1',
      'TelemetryDeck.Error.id',
      [
        {
          propertyName: 'TelemetryDeck.Error.category',
          dataType: 1,
          operator: 'eq',
          value: 'app-state',
        },
      ],
      'TelemetryDeck.Error.occurred',
    ],
  ]);

  expect(screen.getAllByText('7 occurrences')).toHaveLength(2);
  expect(screen.getAllByText('3 occurrences')).toHaveLength(2);
  expect(screen.getAllByText('2 occurrences')).toHaveLength(2);
  expect(screen.getAllByText('1 occurrence')).toHaveLength(2);
  expect(screen.getByText(/Browser JavaScript issue reports are not included/)).toBeInTheDocument();
});
