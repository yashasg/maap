import { beforeEach, describe, expect, test, vi } from 'vitest';
import { getTestRouter, resetTestNavigation } from '@/test/navigation';
import { render, screen, waitFor, within } from '@/test/render';
import RecentEventsScreen from './Screen';

const { useWebsiteEventsQuery } = vi.hoisted(() => ({
  useWebsiteEventsQuery: vi.fn(),
}));

vi.mock('@/components/hooks', async importOriginal => ({
  ...(await importOriginal<typeof import('@/components/hooks')>()),
  useWebsiteEventsQuery,
}));

const event = {
  id: 'event-1',
  websiteId: 'website-1',
  sessionId: 'session-1',
  createdAt: '2026-11-20T09:00:00.000Z',
  hostname: 'app.example.com',
  urlPath: '/checkout',
  urlQuery: '',
  referrerPath: '',
  referrerQuery: '',
  referrerDomain: '',
  country: 'US',
  city: 'Austin',
  device: 'mobile',
  os: 'iOS',
  browser: 'safari',
  pageTitle: '',
  eventType: 2,
  eventName: 'checkout.completed',
  hasData: false,
};

function mockQuery(overrides = {}) {
  return {
    data: {
      data: [event],
      count: 1,
      page: 1,
      pageSize: 50,
    },
    error: null,
    isLoading: false,
    isFetching: false,
    refetch: vi.fn(),
    ...overrides,
  };
}

describe('RecentEventsScreen', () => {
  beforeEach(() => {
    resetTestNavigation();
    useWebsiteEventsQuery.mockReset();
    useWebsiteEventsQuery.mockReturnValue(mockQuery());
  });

  test('shows scoped event data and lets the user inspect one event', async () => {
    const { user } = render(<RecentEventsScreen websiteId="website-1" />, {
      route: '/websites/website-1/app-reports/recent-events?range=30d&timezone=UTC&testMode=true',
    });

    expect(screen.getByRole('heading', { name: 'Recent Events' })).toBeInTheDocument();
    expect(screen.getByText('checkout.completed')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Inspect' }));

    expect(await screen.findByRole('heading', { name: 'checkout.completed' })).toBeInTheDocument();
    const dialog = screen.getByRole('dialog', { name: 'Event details: checkout.completed' });
    expect(within(dialog).getByText('Austin, United States')).toBeInTheDocument();
    expect(within(dialog).getByText('iOS')).toBeInTheDocument();
  });

  test('filters the paged feed without changing website scope', async () => {
    const { user } = render(<RecentEventsScreen websiteId="website-1" />, {
      route:
        '/websites/website-1/app-reports/recent-events?range=30d&timezone=UTC&testMode=true&page=4',
    });

    await user.click(screen.getByRole('button', { name: 'Events' }));

    await waitFor(() => {
      expect(useWebsiteEventsQuery).toHaveBeenLastCalledWith('website-1', { view: 'events' });
    });

    const pushedUrl = getTestRouter().push.mock.lastCall?.[0] as string;
    expect(pushedUrl).toContain('/websites/website-1/app-reports/recent-events?');
    expect(pushedUrl).toContain('range=30d');
    expect(pushedUrl).toContain('timezone=UTC');
    expect(pushedUrl).toContain('testMode=true');
    expect(pushedUrl).toContain('page=1');
  });

  test('offers recovery when the feed request fails', async () => {
    const refetch = vi.fn();
    useWebsiteEventsQuery.mockReturnValue(
      mockQuery({ data: undefined, error: new Error('nope'), refetch }),
    );
    const { user } = render(<RecentEventsScreen websiteId="website-1" />);

    expect(screen.getByText('Events could not be loaded')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Try again' }));
    expect(refetch).toHaveBeenCalledOnce();
  });
});
