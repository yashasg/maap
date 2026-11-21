import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { renderHook, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { beforeEach, expect, test, vi } from 'vitest';
import { useWorkspaceCardMetrics } from './useWorkspaceHome';

const mocks = vi.hoisted(() => ({
  post: vi.fn(),
}));

vi.mock('@/components/hooks/useApi', async () => {
  const { useMutation, useQuery } = await import('@tanstack/react-query');

  return {
    useApi: () => ({
      post: mocks.post,
      useMutation,
      useQuery,
    }),
  };
});

function createWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });

  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>;
  };
}

beforeEach(() => {
  mocks.post.mockReset();
});

test('loads card metrics in backend-sized batches', async () => {
  mocks.post.mockImplementation(async (_path: string, body: { websiteIds: string[] }) => ({
    data: body.websiteIds.map(websiteId => ({ websiteId, users: 1, errors: 0 })),
  }));
  const websiteIds = Array.from({ length: 45 }, (_, index) => `website-${index}`);
  const { result } = renderHook(
    () =>
      useWorkspaceCardMetrics({
        websiteIds,
        startAt: 1,
        endAt: 2,
        testMode: false,
      }),
    { wrapper: createWrapper() },
  );

  await waitFor(() => expect(result.current.data?.data).toHaveLength(45));

  expect(mocks.post).toHaveBeenCalledTimes(3);
  expect(mocks.post.mock.calls.map(([, body]) => body.websiteIds.length)).toEqual([20, 20, 5]);
});

test('does not retain release metrics while test metrics load', async () => {
  let resolveTestMetrics = (_value: unknown) => {};
  mocks.post
    .mockResolvedValueOnce({
      data: [{ websiteId: 'website-1', users: 8, errors: 0 }],
    })
    .mockImplementationOnce(
      () =>
        new Promise(resolve => {
          resolveTestMetrics = resolve;
        }),
    );
  const { result, rerender } = renderHook(
    ({ testMode }) =>
      useWorkspaceCardMetrics({
        websiteIds: ['website-1'],
        startAt: 1,
        endAt: 2,
        testMode,
      }),
    { wrapper: createWrapper(), initialProps: { testMode: false } },
  );

  await waitFor(() => expect(result.current.data?.data[0].users).toBe(8));

  rerender({ testMode: true });

  await waitFor(() => expect(result.current.isPending).toBe(true));
  expect(result.current.data).toBeUndefined();

  resolveTestMetrics({ data: [] });
});
