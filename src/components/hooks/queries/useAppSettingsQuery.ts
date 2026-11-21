import type { AppSettings, AppSettingsResponse } from '@/lib/app-settings';
import type { ApiError, ReactQueryOptions } from '@/lib/types';
import { useApi } from '../useApi';
import { useModified } from '../useModified';

export function useAppSettingsQuery(
  websiteId: string,
  options?: ReactQueryOptions<AppSettingsResponse>,
) {
  const { get, useQuery } = useApi();
  const { modified } = useModified(`app-settings:${websiteId}`);

  return useQuery({
    queryKey: ['app-settings', { websiteId, modified }],
    queryFn: () => get(`/websites/${websiteId}/app-settings`),
    enabled: !!websiteId,
    ...options,
  });
}

export function useAppSettingsMutation(websiteId: string) {
  const { post, useMutation } = useApi();
  const { touch } = useModified();
  const mutation = useMutation<AppSettingsResponse, ApiError, AppSettings>({
    mutationFn: settings => post(`/websites/${websiteId}/app-settings`, settings),
  });

  return { ...mutation, touch };
}
