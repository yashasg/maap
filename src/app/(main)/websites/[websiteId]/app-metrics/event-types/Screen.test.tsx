import { beforeEach, expect, test, vi } from 'vitest';
import { render, screen, within } from '@/test/render';
import EventTypesScreen from './Screen';

const mocks = vi.hoisted(() => ({
  get: vi.fn(),
  queryOptions: undefined as any,
}));

vi.mock('@/components/hooks/useApi', () => ({
  useApi: () => ({
    get: mocks.get,
    useQuery: (options: any) => {
      mocks.queryOptions = options;
      return {
        data: [
          { name: 'cart.added', users: 2, events: 4 },
          { name: 'checkout.started', users: 1, events: 1 },
        ],
        isLoading: false,
        isFetching: false,
        error: null,
      };
    },
  }),
}));

vi.mock('@/components/hooks/useDateParameters', () => ({
  useDateParameters: () => ({ startAt: 1000, endAt: 2000 }),
}));

beforeEach(() => {
  mocks.get.mockReset();
  mocks.queryOptions = undefined;
});

test('shows SDK-user reach and event frequency with a scoped drill-down', async () => {
  const teamId = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa';
  render(<EventTypesScreen websiteId="app-1" />, {
    route: `/teams/${teamId}/websites/app-1/app-reports/event-types?date=30day&timezone=Europe%2FBerlin&testMode=true`,
  });

  const table = within(screen.getByRole('table'));
  const row = within(table.getByRole('row', { name: /cart\.added/ }));
  expect(row.getByRole('cell', { name: '2' })).toBeInTheDocument();
  expect(row.getByRole('cell', { name: '4' })).toBeInTheDocument();

  const drillDown = row.getByRole('link', { name: 'cart.added' });
  const href = drillDown.getAttribute('href');
  expect(href).not.toBeNull();
  const url = new URL(href ?? '', 'http://localhost');
  expect(url.pathname).toBe(`/teams/${teamId}/websites/app-1/app-reports/recent-events`);
  expect(Object.fromEntries(url.searchParams)).toEqual({
    date: '30day',
    timezone: 'Europe/Berlin',
    testMode: 'true',
    search: 'cart.added',
  });

  await mocks.queryOptions.queryFn();
  expect(mocks.get).toHaveBeenCalledWith('/websites/app-1/app-reports/event-types', {
    startAt: 1000,
    endAt: 2000,
    testMode: 'true',
  });
});
