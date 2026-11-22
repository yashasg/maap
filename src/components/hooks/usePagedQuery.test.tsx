import { QueryClientProvider, useQuery } from '@tanstack/react-query';
import { renderHook, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { expect, test, vi } from 'vitest';
import { setTestUrl } from '@/test/navigation';
import { createTestQueryClient } from '@/test/render';
import { useNavigation } from './useNavigation';
import { usePagedQuery } from './usePagedQuery';

vi.mock('./useApi', () => ({ useApi: () => ({ useQuery }) }));

test('keeps results while paging/searching, but never across properties or report filters', async () => {
  const client = createTestQueryClient();
  const firstPage = { data: [{ id: 'event-1' }], count: 2, page: 1, pageSize: 1 };
  const queryFn = vi
    .fn()
    .mockResolvedValueOnce(firstPage)
    .mockImplementation(() => new Promise(() => {}));
  const { result, rerender } = renderHook(
    ({ websiteId, testMode }) => {
      const { query } = useNavigation();
      return usePagedQuery({ queryKey: ['events', { websiteId, testMode, ...query }], queryFn });
    },
    {
      initialProps: { websiteId: 'first', testMode: false },
      wrapper: ({ children }: { children: ReactNode }) => (
        <QueryClientProvider client={client}>{children}</QueryClientProvider>
      ),
    },
  );
  await waitFor(() => expect(result.current.data).toEqual(firstPage));

  for (const query of ['page=2', 'search=checkout', 'orderBy=createdAt&sortDescending=true']) {
    setTestUrl(`/websites/first/events?${query}`);
    rerender({ websiteId: 'first', testMode: false });
    expect(result.current.data).toEqual(firstPage);
    expect(result.current.isPlaceholderData).toBe(true);
    expect(result.current.isFetching).toBe(true);
  }

  rerender({ websiteId: 'second', testMode: false });
  expect(result.current.data).toBeUndefined();
  rerender({ websiteId: 'first', testMode: true });
  expect(result.current.data).toBeUndefined();
  client.clear();
});
