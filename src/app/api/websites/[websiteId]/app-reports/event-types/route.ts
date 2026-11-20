import prisma from '@/lib/prisma';
import { getQueryFilters, parseRequest } from '@/lib/request';
import { json, serverError, unauthorized } from '@/lib/response';
import { canViewWebsiteSection } from '@/permissions';
import { analyticsQuerySchema } from '../../analytics-schema';

type EventTypeRow = {
  name: string;
  users: bigint | number;
  events: bigint | number;
};

function modePredicate(isTest: boolean) {
  return isTest ? "e.tag = 'test'" : "coalesce(e.tag, '') != 'test'";
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
    const isTest = filters.tag === 'test' || filters.tag === 'eq.test';
    const rows = (await prisma.rawQuery(
      `
      select
        e.event_name as name,
        count(distinct nullif(s.distinct_id, '')) as users,
        count(*) as events
      from website_event e
      left join session s
        on s.website_id = e.website_id
       and s.session_id = e.session_id
      where e.website_id = {{websiteId::uuid}}
        and e.created_at between {{startDate}} and {{endDate}}
        and e.event_type = 2
        and e.event_name is not null
        and ${modePredicate(isTest)}
      group by e.event_name
      order by events desc, lower(e.event_name), e.event_name
      `,
      { websiteId, startDate, endDate },
      'getAppEventTypes',
    )) as EventTypeRow[];

    return json({
      data: rows.map(row => ({
        name: row.name,
        users: Number(row.users),
        events: Number(row.events),
      })),
    });
  } catch (error) {
    return serverError(error);
  }
}
