import { beforeEach, expect, test, vi } from 'vitest';
import { render, screen } from '@/test/render';
import VersionsScreen from './Screen';

const mocks = vi.hoisted(() => ({
  queries: [] as string[],
  refetch: vi.fn(),
  mode: 'data' as 'data' | 'empty' | 'error' | 'loading',
}));

vi.mock('@/components/hooks/queries/useAnalyticsQuery', () => ({
  useAnalyticsQuery: (_path: string, params: { dimension: string }) => {
    mocks.queries.push(params.dimension);
    const values = {
      appVersion: [
        { value: '2.0', events: 6 },
        { value: '1.9', events: 3 },
        { value: null, events: 1 },
      ],
      buildNumber: [
        { value: '200', events: 4 },
        { value: '201', events: 6 },
      ],
      sdkVersion: [{ value: '1.8.0', events: 10 }],
    }[params.dimension];
    if (mocks.mode === 'loading') {
      return {
        data: undefined,
        error: null,
        isLoading: true,
        isFetching: true,
        refetch: mocks.refetch,
      };
    }
    if (mocks.mode === 'error') {
      return {
        data: undefined,
        error: new Error('network'),
        isLoading: false,
        isFetching: false,
        refetch: mocks.refetch,
      };
    }
    return {
      data: {
        dimension: params.dimension,
        totalEvents: mocks.mode === 'empty' ? 0 : 10,
        reportedEvents: mocks.mode === 'empty' ? 0 : params.dimension === 'appVersion' ? 9 : 10,
        values: mocks.mode === 'empty' ? [] : values,
      },
      error: null,
      isLoading: false,
      isFetching: false,
      refetch: mocks.refetch,
    };
  },
}));

beforeEach(() => {
  mocks.queries = [];
  mocks.refetch.mockClear();
  mocks.mode = 'data';
});

test('queries and renders each per-event version dimension independently', () => {
  render(<VersionsScreen websiteId="website-1" />);

  expect(mocks.queries).toEqual(['appVersion', 'buildNumber', 'sdkVersion']);
  expect(screen.getByRole('heading', { name: 'App Versions' })).toBeInTheDocument();
  expect(screen.getByRole('heading', { name: 'Build Numbers' })).toBeInTheDocument();
  expect(screen.getByRole('heading', { name: 'SDK Versions' })).toBeInTheDocument();
  expect(screen.getByText('2.0')).toBeInTheDocument();
  expect(screen.getByText('Not reported')).toBeInTheDocument();
  expect(screen.getByText('1.8.0')).toBeInTheDocument();
  expect(screen.getByText('9 of 10 events reported a value')).toBeInTheDocument();
});

test('refreshes only the selected panel query', async () => {
  const { user } = render(<VersionsScreen websiteId="website-1" />);

  await user.click(screen.getByRole('button', { name: 'Refresh SDK Versions' }));

  expect(mocks.refetch).toHaveBeenCalledTimes(1);
});

test('distinguishes empty results from a failed request', () => {
  mocks.mode = 'empty';
  const { rerender } = render(<VersionsScreen websiteId="website-1" />);

  expect(screen.getAllByText('No version data')).toHaveLength(3);
  expect(screen.queryByText('Something went wrong.')).not.toBeInTheDocument();

  mocks.mode = 'error';
  rerender(<VersionsScreen websiteId="website-1" />);

  expect(screen.getAllByText('Something went wrong.')).toHaveLength(3);
  expect(screen.queryByText('0 of 0 events reported a value')).not.toBeInTheDocument();
});
