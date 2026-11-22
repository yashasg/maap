import { hashKey, type UseQueryOptions, type UseQueryResult } from '@tanstack/react-query';
import type { PageResult } from '@/lib/types';
import { useApi } from './useApi';
import { useNavigation } from './useNavigation';

export function usePagedQuery<TData = any, TError = Error>({
  queryKey,
  queryFn,
  ...options
}: Omit<
  UseQueryOptions<PageResult<TData>, TError, PageResult<TData>, readonly unknown[]>,
  'queryFn' | 'queryKey'
> & {
  queryKey: readonly unknown[];
  queryFn: (params?: object) => Promise<PageResult<TData>> | PageResult<TData>;
}): UseQueryResult<PageResult<TData>, TError> {
  const {
    query: { page, search, orderBy, sortDescending },
  } = useNavigation();
  const { useQuery } = useApi();

  return useQuery<PageResult<TData>, TError>({
    queryKey: [...queryKey, page, search, orderBy, sortDescending] as const,
    queryFn: () => queryFn({ page, search, orderBy, sortDescending }),
    placeholderData: (data, previous) =>
      previous && queryScope(previous.queryKey.slice(0, -4)) === queryScope(queryKey)
        ? data
        : undefined,
    ...options,
  });
}

// Filter objects from callers can duplicate the pagination fields appended above.
function queryScope(key: readonly unknown[]) {
  return hashKey(
    key.map(part =>
      part && typeof part === 'object' && !Array.isArray(part)
        ? Object.fromEntries(
            Object.entries(part).filter(
              ([key]) => !['page', 'pageSize', 'search', 'orderBy', 'sortDescending'].includes(key),
            ),
          )
        : part,
    ),
  );
}
