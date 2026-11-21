import { beforeEach, expect, test, vi } from 'vitest';
import { render, screen } from '@/test/render';
import ParametersScreen, { getParameterRows } from './Screen';

const mockUsePropertyFieldsQuery = vi.fn();

vi.mock('@/components/hooks', async () => {
  const actual = await vi.importActual<typeof import('@/components/hooks')>('@/components/hooks');

  return {
    ...actual,
    usePropertyFieldsQuery: (...args: unknown[]) => mockUsePropertyFieldsQuery(...args),
  };
});

beforeEach(() => {
  mockUsePropertyFieldsQuery.mockReset();
});

test('renders observed custom and SDK parameter types without exposing values', () => {
  mockUsePropertyFieldsQuery.mockReturnValue({
    data: [
      {
        propertyName: 'score',
        dataType: 2,
        total: 2,
        propertyValue: 'private-score',
      },
      {
        propertyName: 'TelemetryDeck.AppInfo.version',
        dataType: 1,
        total: 3,
        propertyValue: '2.10',
      },
      {
        propertyName: 'accountTier',
        dataType: 1,
        total: 1,
        propertyValue: 'private-tier',
      },
    ],
    isLoading: false,
    isFetching: false,
    error: undefined,
  });

  render(<ParametersScreen websiteId="app-1" />, {
    route: '/websites/app-1/app-reports/parameters?testMode=true',
  });

  expect(mockUsePropertyFieldsQuery).toHaveBeenCalledWith('event', 'app-1', undefined);
  expect(screen.getAllByRole('heading', { name: 'Parameters' })).toHaveLength(2);
  expect(screen.getByRole('columnheader', { name: 'Name' })).toBeInTheDocument();
  expect(screen.getByRole('columnheader', { name: 'Type' })).toBeInTheDocument();

  const rows = screen.getAllByRole('row').slice(1);
  expect(rows.map(row => row.textContent)).toEqual([
    'accountTierString',
    'scoreNumber',
    'TelemetryDeck.AppInfo.versionString',
  ]);
  expect(screen.queryByText('private-score')).not.toBeInTheDocument();
  expect(screen.queryByText('private-tier')).not.toBeInTheDocument();
  expect(screen.queryByText('2.10')).not.toBeInTheDocument();
});

test('keeps distinct observed types and reports unknown stored type codes honestly', () => {
  expect(
    getParameterRows([
      { propertyName: 'amount', dataType: 1 },
      { propertyName: 'amount', dataType: 2 },
      { propertyName: 'opaque', dataType: 9 },
    ]),
  ).toEqual([
    { id: 'amount:1', propertyName: 'amount', dataType: 1, type: 'String' },
    { id: 'amount:2', propertyName: 'amount', dataType: 2, type: 'Number' },
    { id: 'opaque:9', propertyName: 'opaque', dataType: 9, type: 'Unknown (9)' },
  ]);
});

test('shows explicit empty and error states', () => {
  mockUsePropertyFieldsQuery.mockReturnValue({
    data: [],
    isLoading: false,
    isFetching: false,
    error: undefined,
  });

  const result = render(<ParametersScreen websiteId="app-1" />);

  expect(
    screen.getByText('No parameters were observed for this app in the selected date range.'),
  ).toBeInTheDocument();

  mockUsePropertyFieldsQuery.mockReturnValue({
    data: undefined,
    isLoading: false,
    isFetching: false,
    error: new Error('Request failed'),
  });
  result.rerender(<ParametersScreen websiteId="app-1" />);

  expect(screen.getByText('Something went wrong.')).toBeInTheDocument();
});
