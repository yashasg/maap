import { REALTIME_INTERVAL } from '@/lib/constants';
import type { RealtimeData } from '@/lib/types';
import { useApi } from '../useApi';
import { useFilterParameters } from '../useFilterParameters';

export function useRealtimeQuery(websiteId: string) {
  const { get, useQuery } = useApi();
  const params = useFilterParameters({ includePagination: false });
  const { data, isLoading, error } = useQuery<RealtimeData>({
    queryKey: ['realtime', { websiteId, ...params }],
    queryFn: async () => {
      return get(`/realtime/${websiteId}`, params);
    },
    enabled: !!websiteId,
    refetchInterval: REALTIME_INTERVAL,
  });

  return { data, isLoading, error };
}
