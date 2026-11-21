import type { ReactNode } from 'react';
import { beforeEach, expect, test, vi } from 'vitest';
import { render, screen } from '@/test/render';
import LocalizationScreen, { LOCALIZATION_METRICS, normalizeValues } from './Screen';

const values = {
  'TelemetryDeck.UserPreference.language': [{ value: 'fr', total: 3 }],
  'TelemetryDeck.RunContext.language': [{ value: 'en', total: 2 }],
  'TelemetryDeck.UserPreference.region': [{ value: 'CA', total: 2 }],
  'TelemetryDeck.UserPreference.layoutDirection': [],
};

const mocks = vi.hoisted(() => ({
  usePropertyValuesQuery: vi.fn(),
}));

mocks.usePropertyValuesQuery.mockImplementation(
  (_source: string, _websiteId: string, propertyName: keyof typeof values) => ({
    data: values[propertyName],
    error: null,
    isLoading: false,
    isFetching: false,
  }),
);

vi.mock('@umami/react-zen', async importOriginal => {
  const actual = await importOriginal<typeof import('@umami/react-zen')>();

  return {
    ...actual,
    Column: ({ children }: { children: ReactNode }) => <div>{children}</div>,
    Grid: ({ children }: { children: ReactNode }) => <div>{children}</div>,
    Heading: ({ children }: { children: ReactNode }) => <h2>{children}</h2>,
    Row: ({ children }: { children: ReactNode }) => <div>{children}</div>,
    Text: ({ children }: { children: ReactNode }) => <span>{children}</span>,
  };
});

vi.mock('@/components/hooks', () => ({
  usePropertyValuesQuery: mocks.usePropertyValuesQuery,
}));

vi.mock('@/components/common/Panel', () => ({
  Panel: ({ title, children }: { title: string; children: ReactNode }) => (
    <section>
      <h3>{title}</h3>
      {children}
    </section>
  ),
}));

vi.mock('@/components/common/LoadingPanel', () => ({
  LoadingPanel: ({
    data,
    renderEmpty,
    children,
  }: {
    data: unknown[];
    renderEmpty: () => ReactNode;
    children: ReactNode;
  }) => (data.length ? children : renderEmpty()),
}));

vi.mock('@/components/charts/PieChart', () => ({
  PieChart: ({ chartData }: { chartData: { labels: string[] } }) => (
    <div>Chart: {chartData.labels.join(', ')}</div>
  ),
}));

vi.mock('@/components/metrics/ListTable', () => ({
  ListTable: ({ data }: { data: { label: string; count: number }[] }) => (
    <ul>
      {data.map(item => (
        <li key={item.label}>{`${item.label}: ${item.count}`}</li>
      ))}
    </ul>
  ),
}));

beforeEach(() => {
  mocks.usePropertyValuesQuery.mockClear();
});

test('queries and renders the four localization settings independently', () => {
  render(<LocalizationScreen websiteId="app-1" />);

  expect(screen.getByText('fr: 3')).toBeInTheDocument();
  expect(screen.getByText('en: 2')).toBeInTheDocument();
  expect(screen.getByText('CA: 2')).toBeInTheDocument();
  expect(screen.getByText('Not reported in this range')).toBeInTheDocument();
  expect(
    screen.getByText(
      'The configured device region reported by the SDK, not an IP-derived location.',
    ),
  ).toBeInTheDocument();

  expect(mocks.usePropertyValuesQuery).toHaveBeenCalledTimes(4);
  expect(mocks.usePropertyValuesQuery.mock.calls).toEqual(
    LOCALIZATION_METRICS.map(metric => ['event', 'app-1', metric.propertyName]),
  );
});

test('keeps not-reported values out of totals and labels layout direction', () => {
  expect(
    normalizeValues(
      [
        { value: 'leftToRight', total: 2 },
        { value: null, total: 9 },
        { value: 'rightToLeft', total: 1 },
      ],
      value => ({ leftToRight: 'Left to right', rightToLeft: 'Right to left' })[value] ?? value,
    ),
  ).toEqual([
    { label: 'Left to right', count: 2 },
    { label: 'Right to left', count: 1 },
  ]);
});
