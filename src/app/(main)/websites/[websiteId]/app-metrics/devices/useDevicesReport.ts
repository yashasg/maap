'use client';

import { useApi, useDateParameters, useFilterParameters } from '@/components/hooks';
import type { DeviceReport } from './Screen';

export function getDevicesReportFilters(filters: Record<string, unknown>) {
  return {
    ...filters,
    tag: filters.tag ?? 'neq.test',
  };
}

export function useDevicesReport(websiteId: string) {
  const { get, useQuery } = useApi();
  const { startAt, endAt, unit, timezone } = useDateParameters();
  const filters = getDevicesReportFilters(useFilterParameters({ includePagination: false }));
  const params = { ...filters, startAt, endAt, unit, timezone };

  return useQuery<{ reports: DeviceReport[] }>({
    queryKey: ['websites:app-reports:devices', { websiteId, ...params }],
    queryFn: () => get(`/websites/${websiteId}/app-reports/devices`, params),
    enabled: !!websiteId,
  });
}
