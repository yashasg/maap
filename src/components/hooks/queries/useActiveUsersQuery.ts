import type { ReactQueryOptions } from '@/lib/types';
import { useApi } from '../useApi';
import { useFilterParameters } from '../useFilterParameters';

export function useActiveUsersQuery(websiteId: string, options?: ReactQueryOptions) {
  const { get, useQuery } = useApi();
  const params = useFilterParameters({ includePagination: false });
  return useQuery<any>({
    queryKey: ['websites:active', websiteId, params.tag],
    queryFn: () =>
      get(`/websites/${websiteId}/active`, params.tag ? { tag: params.tag } : undefined),
    enabled: !!websiteId,
    ...options,
  });
}
