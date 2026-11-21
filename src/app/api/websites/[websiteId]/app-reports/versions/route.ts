import { z } from 'zod';
import prisma from '@/lib/prisma';
import { getQueryFilters, parseRequest } from '@/lib/request';
import { badRequest, json, serverError, unauthorized } from '@/lib/response';
import { withDateRange } from '@/lib/schema';
import { canViewWebsiteSection } from '@/permissions';

const DIMENSIONS = {
  appVersion: ['TelemetryDeck.AppInfo.version', 'appVersion'],
  buildNumber: ['TelemetryDeck.AppInfo.buildNumber', 'buildNumber'],
  sdkVersion: ['TelemetryDeck.SDK.version', 'sdkVersion'],
} as const;

type Dimension = keyof typeof DIMENSIONS;
type VersionRow = {
  value: string | null;
  events: bigint | number;
};

export const versionsQuerySchema = withDateRange({
  dimension: z.enum(['appVersion', 'buildNumber', 'sdkVersion']),
  tag: z.enum(['test', 'eq.test', 'neq.test']).optional(),
  testMode: z.enum(['true', 'false']).optional(),
}).superRefine((data, context) => {
  if (data.tag !== undefined && data.testMode !== undefined) {
    context.addIssue({
      code: z.ZodIssueCode.custom,
      message: 'testMode cannot be combined with tag',
    });
  }
});

function number(value: bigint | number) {
  return Number(value);
}

function modePredicate(isTest: boolean) {
  return isTest ? "e.tag = 'test'" : "coalesce(e.tag, '') != 'test'";
}

function validRange(startDate: Date, endDate: Date) {
  return (
    Number.isFinite(startDate.getTime()) &&
    Number.isFinite(endDate.getTime()) &&
    startDate <= endDate
  );
}

export async function GET(
  request: Request,
  { params }: { params: Promise<{ websiteId: string }> },
) {
  const { auth, query, error } = await parseRequest(request, versionsQuerySchema);
  if (error) return error();

  const { websiteId } = await params;
  if (!(await canViewWebsiteSection(auth, websiteId, 'events'))) return unauthorized();

  try {
    const filters = await getQueryFilters(query, websiteId);
    const { startDate, endDate } = filters;
    if (!validRange(startDate, endDate)) {
      return badRequest({ message: 'The start date must not be after the end date' });
    }

    const dimension = query.dimension as Dimension;
    const [primaryKey, fallbackKey] = DIMENSIONS[dimension];
    const isTest = filters.tag === 'test' || filters.tag === 'eq.test';
    const rows = (await prisma.rawQuery(
      `
      with scoped as (
        select e.event_id
        from website_event e
        where e.website_id = {{websiteId::uuid}}
          and e.created_at between {{startDate}} and {{endDate}}
          and ${modePredicate(isTest)}
      ),
      dimension_events as (
        select (
          select nullif(btrim(d.string_value), '')
          from event_data d
          where d.website_id = {{websiteId::uuid}}
            and d.website_event_id = scoped.event_id
            and d.data_key in ({{primaryKey}}, {{fallbackKey}})
          order by case when d.data_key = {{primaryKey}} then 0 else 1 end
          limit 1
        ) as value
        from scoped
      )
      select value, count(*) as events
      from dimension_events
      group by value
      order by events desc, value asc nulls last
      `,
      { websiteId, startDate, endDate, primaryKey, fallbackKey },
      'getAppVersions',
    )) as VersionRow[];

    const values = rows.map(row => ({ value: row.value, events: number(row.events) }));
    const totalEvents = values.reduce((sum, row) => sum + row.events, 0);
    const reportedEvents = values.reduce(
      (sum, row) => sum + (row.value === null ? 0 : row.events),
      0,
    );

    return json({
      dimension,
      totalEvents,
      reportedEvents,
      values,
    });
  } catch (error) {
    return serverError(error);
  }
}
