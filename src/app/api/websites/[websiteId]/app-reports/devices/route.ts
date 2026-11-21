import prisma from '@/lib/prisma';
import { getQueryFilters, parseRequest } from '@/lib/request';
import { badRequest, json, serverError, unauthorized } from '@/lib/response';
import { canViewWebsiteSection } from '@/permissions';
import { analyticsQuerySchema } from '../../analytics-schema';

const DIMENSIONS = [
  ['models', 'TelemetryDeck.Device.modelName', 'modelName'],
  ['systemVersions', 'TelemetryDeck.Device.systemVersion', 'systemVersion'],
  ['types', 'TelemetryDeck.Device.modelName', 'modelName'],
  ['platforms', 'TelemetryDeck.Device.platform', 'TelemetryDeck.Device.operatingSystem'],
  ['screenWidths', 'TelemetryDeck.Device.screenResolutionWidth', 'screenResolutionWidth'],
  ['screenHeights', 'TelemetryDeck.Device.screenResolutionHeight', 'screenResolutionHeight'],
  ['colorSchemes', 'TelemetryDeck.UserPreference.colorScheme', 'colorScheme'],
  ['orientations', 'TelemetryDeck.Device.orientation', 'orientation'],
] as const;

type BreakdownRow = {
  metricIndex: number;
  value: string | null;
  signals: bigint | number;
};

function isHardwareIdentifier(value: string) {
  return !/\s/.test(value) && /^[A-Za-z][A-Za-z0-9-]*\d+(?:,\d+)?$/.test(value);
}

export function buildDevicesResponse(rows: BreakdownRow[]) {
  return {
    reports: DIMENSIONS.map(([id], metricIndex) => {
      const counts = new Map<string | null, number>();

      for (const row of rows.filter(row => Number(row.metricIndex) === metricIndex)) {
        const trimmed = row.value?.trim() || null;
        const value = id === 'types' && trimmed && !isHardwareIdentifier(trimmed) ? null : trimmed;
        counts.set(value, (counts.get(value) ?? 0) + Number(row.signals));
      }

      const values = [...counts]
        .map(([value, signals]) => ({ value, signals }))
        .sort((a, b) => {
          const countOrder = b.signals - a.signals;
          if (countOrder) return countOrder;
          if (a.value === null) return 1;
          if (b.value === null) return -1;
          return a.value.localeCompare(b.value);
        });
      const totalSignals = values.reduce((sum, value) => sum + value.signals, 0);
      const missingSignals = values.find(value => value.value === null)?.signals ?? 0;

      return {
        id,
        totalSignals,
        reportedSignals: totalSignals - missingSignals,
        values,
      };
    }),
  };
}

export async function GET(
  request: Request,
  { params }: { params: Promise<{ websiteId: string }> },
) {
  const { auth, query, error } = await parseRequest(request, analyticsQuerySchema);
  if (error) return error();

  const { websiteId } = await params;
  if (!(await canViewWebsiteSection(auth, websiteId, 'events'))) return unauthorized();

  try {
    const filters = await getQueryFilters(query, websiteId);
    const { startDate, endDate } = filters;
    if (startDate > endDate) {
      return badRequest({ message: 'The start date must not be after the end date' });
    }
    const testMode = filters.tag === 'test' || filters.tag === 'eq.test';
    const modePredicate = testMode ? "e.tag = 'test'" : "coalesce(e.tag, '') != 'test'";
    const rows = (await prisma.rawQuery(
      `
      with scoped as (
        select e.event_id, e.website_id
        from website_event e
        where e.website_id = {{websiteId::uuid}}
          and e.created_at between {{startDate}} and {{endDate}}
          and e.event_type in (1, 2)
          and ${modePredicate}
      ),
      metrics(metric_index, primary_key, fallback_key) as (
        values
          (0, 'TelemetryDeck.Device.modelName', 'modelName'),
          (1, 'TelemetryDeck.Device.systemVersion', 'systemVersion'),
          (2, 'TelemetryDeck.Device.modelName', 'modelName'),
          (3, 'TelemetryDeck.Device.platform', 'TelemetryDeck.Device.operatingSystem'),
          (4, 'TelemetryDeck.Device.screenResolutionWidth', 'screenResolutionWidth'),
          (5, 'TelemetryDeck.Device.screenResolutionHeight', 'screenResolutionHeight'),
          (6, 'TelemetryDeck.UserPreference.colorScheme', 'colorScheme'),
          (7, 'TelemetryDeck.Device.orientation', 'orientation')
      )
      select
        metrics.metric_index as "metricIndex",
        value.raw_value as "value",
        count(*) as "signals"
      from metrics
      cross join scoped e
      left join lateral (
        select nullif(btrim(d.string_value), '') as raw_value
        from event_data d
        where d.website_id = e.website_id
          and d.website_event_id = e.event_id
          and d.data_key in (metrics.primary_key, metrics.fallback_key)
        order by case when d.data_key = metrics.primary_key then 0 else 1 end,
                 d.created_at desc nulls last,
                 d.event_data_id desc
        limit 1
      ) value on true
      group by metrics.metric_index, value.raw_value
      order by metrics.metric_index
      `,
      { websiteId, startDate, endDate },
      'getAppDeviceBreakdowns',
    )) as BreakdownRow[];

    return json(buildDevicesResponse(rows));
  } catch (error) {
    return serverError(error);
  }
}
