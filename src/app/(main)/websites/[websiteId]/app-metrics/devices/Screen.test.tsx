import type { ReactNode } from 'react';
import { expect, test, vi } from 'vitest';
import { render, screen } from '@/test/render';
import DevicesScreen, { normalizeDeviceReport } from './Screen';

const report = {
  reports: [
    {
      id: 'models',
      totalSignals: 4,
      reportedSignals: 4,
      values: [{ value: 'iPad14,3', signals: 4 }],
    },
    {
      id: 'systemVersions',
      totalSignals: 4,
      reportedSignals: 4,
      values: [{ value: 'iOS 18.4.1', signals: 4 }],
    },
    {
      id: 'types',
      totalSignals: 4,
      reportedSignals: 4,
      values: [{ value: 'iPad14,3', signals: 4 }],
    },
    {
      id: 'platforms',
      totalSignals: 4,
      reportedSignals: 4,
      values: [{ value: 'iPadOS', signals: 4 }],
    },
    {
      id: 'screenWidths',
      totalSignals: 4,
      reportedSignals: 4,
      values: [{ value: '834', signals: 4 }],
    },
    {
      id: 'screenHeights',
      totalSignals: 4,
      reportedSignals: 4,
      values: [{ value: '1194', signals: 4 }],
    },
    {
      id: 'colorSchemes',
      totalSignals: 4,
      reportedSignals: 3,
      values: [
        { value: 'dark', signals: 3 },
        { value: null, signals: 1 },
      ],
    },
    {
      id: 'orientations',
      totalSignals: 4,
      reportedSignals: 2,
      values: [
        { value: 'landscape', signals: 2 },
        { value: null, signals: 2 },
      ],
    },
  ],
};

const mocks = vi.hoisted(() => ({
  useDevicesReport: vi.fn(),
}));

mocks.useDevicesReport.mockReturnValue({
  data: report,
  error: null,
  isLoading: false,
  isFetching: false,
});

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

vi.mock('./useDevicesReport', () => ({
  useDevicesReport: mocks.useDevicesReport,
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

test('renders a reported landscape tablet without inventing missing dimensions', () => {
  render(<DevicesScreen websiteId="app-1" />);

  expect(screen.getAllByText('iPad14,3: 4')).toHaveLength(2);
  expect(screen.getByText('iOS 18.4.1: 4')).toBeInTheDocument();
  expect(screen.getByText('iPadOS: 4')).toBeInTheDocument();
  expect(screen.getByText('834 pt: 4')).toBeInTheDocument();
  expect(screen.getByText('1,194 pt: 4')).toBeInTheDocument();
  expect(screen.getByText('Dark: 3')).toBeInTheDocument();
  expect(screen.getByText('Landscape: 2')).toBeInTheDocument();
  expect(screen.getAllByText('Not reported: 2')).toHaveLength(1);
  expect(mocks.useDevicesReport).toHaveBeenCalledWith('app-1');
});

test('keeps missing values visible and excludes invalid counts', () => {
  expect(
    normalizeDeviceReport({
      id: 'orientations',
      totalSignals: 13,
      reportedSignals: 3,
      values: [
        { value: 'landscape', signals: 3 },
        { value: null, signals: 10 },
        { value: 'portrait', signals: 0 },
      ],
    }),
  ).toEqual([
    { label: 'Not reported', count: 10 },
    { label: 'landscape', count: 3 },
  ]);
});
