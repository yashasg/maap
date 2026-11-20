import { z } from 'zod';
import { SHARE_CONTEXT_HEADER, SHARE_TOKEN_HEADER } from '@/lib/constants';
import { fetchAccount, getBillingAccess } from '@/lib/load';
import prisma from '@/lib/prisma';
import { getCloudDataDateFloor, parseRequest } from '@/lib/request';
import { json, serverError, unauthorized } from '@/lib/response';
import { canViewTeam } from '@/permissions';

const MAX_WEBSITES = 20;
const MAX_RANGE_MS = 366 * 24 * 60 * 60 * 1000;
const MAX_DATE_MS = 8_640_000_000_000_000;
const NATIVE_ERROR_EVENT = 'TelemetryDeck.Error.occurred';

const requestSchema = z
  .strictObject({
    websiteIds: z.array(z.uuid()).min(1).max(MAX_WEBSITES),
    startAt: z.number().int().min(0).max(MAX_DATE_MS),
    endAt: z.number().int().min(0).max(MAX_DATE_MS),
    testMode: z.boolean(),
    teamId: z.uuid().optional(),
  })
  .superRefine((data, context) => {
    if (new Set(data.websiteIds).size !== data.websiteIds.length) {
      context.addIssue({
        code: 'custom',
        path: ['websiteIds'],
        message: 'Website IDs must be unique',
      });
    }
    if (data.startAt > data.endAt) {
      context.addIssue({
        code: 'custom',
        path: ['endAt'],
        message: 'The end date must not be before the start date',
      });
    } else if (data.endAt - data.startAt > MAX_RANGE_MS) {
      context.addIssue({
        code: 'custom',
        path: ['endAt'],
        message: 'The date range must not exceed 366 days',
      });
    }
  });

type MetricRow = {
  websiteId: string;
  users: bigint | number;
  errors: bigint | number;
};

function isNativeAppDomain(domain: string | null) {
  if (!domain) return false;
  const value = domain.toLowerCase();
  return (
    ['app.ios', 'app.macos', 'app.android'].includes(value) ||
    value.includes('apps.apple.com/') ||
    value.includes('itunes.apple.com/') ||
    value.includes('play.google.com/')
  );
}

function emptyMetrics(websiteIds: string[]) {
  return {
    data: websiteIds.map(websiteId => ({ websiteId, users: 0, errors: 0 })),
  };
}

export async function POST(request: Request) {
  const { auth, body, error } = await parseRequest(request, requestSchema);
  if (error) return error();

  if (
    !auth.user ||
    auth.shareToken ||
    request.headers.has(SHARE_TOKEN_HEADER) ||
    request.headers.has(SHARE_CONTEXT_HEADER)
  ) {
    return unauthorized();
  }

  const { websiteIds, startAt, endAt, testMode, teamId } = body;

  try {
    let minimumDate: Date | undefined;
    if (teamId) {
      if (!(await canViewTeam(auth, teamId))) {
        return unauthorized();
      }
    } else {
      if (
        !auth.user.isAdmin &&
        (await getBillingAccess({ accountId: auth.user.id }, auth.user.id)).isPastDue
      ) {
        return unauthorized();
      }
      minimumDate = getCloudDataDateFloor(await fetchAccount(auth.user.id));
    }

    const websites = await prisma.client.website.findMany({
      where: {
        id: { in: websiteIds },
        deletedAt: null,
        ...(teamId
          ? { teamId }
          : {
              userId: auth.user.id,
              teamId: null,
            }),
      },
      select: {
        id: true,
        domain: true,
      },
    });

    if (websites.length !== websiteIds.length) {
      return unauthorized();
    }

    const nativeWebsiteIds = websites
      .filter(website => isNativeAppDomain(website.domain))
      .map(website => website.id);
    const requestedStartDate = new Date(startAt);
    const startDate =
      minimumDate && minimumDate > requestedStartDate ? minimumDate : requestedStartDate;
    const endDate = new Date(endAt);
    if (startDate > endDate) {
      return json(emptyMetrics(websiteIds));
    }
    const rows = (await prisma.rawQuery(
      `
      with identity_candidates as (
        select website_id, session_id, nullif(distinct_id, '') as user_id
        from session
        where website_id = any({{websiteIds}}::uuid[])
          and nullif(distinct_id, '') is not null
        union all
        select website_id, session_id, nullif(distinct_id, '') as user_id
        from session_link
        where website_id = any({{websiteIds}}::uuid[])
          and nullif(distinct_id, '') is not null
      ),
      identity_sessions as (
        select website_id, session_id, min(user_id) as user_id
        from identity_candidates
        group by website_id, session_id
        having count(distinct user_id) = 1
      ),
      event_metrics as (
        select
          e.website_id,
          count(distinct case
            when e.website_id = any({{nativeWebsiteIds}}::uuid[]) then identities.user_id
            when e.event_type not in (2, 5) then e.session_id::text
          end)::bigint as users,
          count(*) filter (
            where e.website_id = any({{nativeWebsiteIds}}::uuid[])
              and e.event_name = '${NATIVE_ERROR_EVENT}'
          )::bigint as app_errors
        from website_event e
        join website w on w.website_id = e.website_id
        left join identity_sessions identities
          on identities.website_id = e.website_id
         and identities.session_id = e.session_id
        where e.website_id = any({{websiteIds}}::uuid[])
          and e.created_at between
            greatest({{startDate}}, coalesce(w.reset_at, 'epoch'::timestamptz))
            and {{endDate}}
          and (
            ({{testMode}} and e.tag = 'test')
            or (not {{testMode}} and coalesce(e.tag, '') != 'test')
          )
        group by e.website_id
      ),
      website_errors as (
        select ee.website_id, count(*)::bigint as errors
        from error_event ee
        join website w on w.website_id = ee.website_id
        where ee.website_id = any({{websiteIds}}::uuid[])
          and not (ee.website_id = any({{nativeWebsiteIds}}::uuid[]))
          and ee.created_at between {{startDate}} and {{endDate}}
          and ee.created_at > coalesce(w.reset_at, 'epoch'::timestamptz)
        group by ee.website_id
      )
      select
        w.website_id as "websiteId",
        coalesce(metrics.users, 0)::bigint as users,
        case
          when w.website_id = any({{nativeWebsiteIds}}::uuid[])
            then coalesce(metrics.app_errors, 0)
          else coalesce(website_errors.errors, 0)
        end::bigint as errors
      from website w
      left join event_metrics metrics using (website_id)
      left join website_errors using (website_id)
      where w.website_id = any({{websiteIds}}::uuid[])
      `,
      { websiteIds, nativeWebsiteIds, startDate, endDate, testMode },
      'getWorkspaceCardMetrics',
    )) as MetricRow[];

    const metrics = new Map(
      rows.map(row => [
        row.websiteId,
        {
          websiteId: row.websiteId,
          users: Number(row.users),
          errors: Number(row.errors),
        },
      ]),
    );

    return json({
      data: websiteIds.map(
        websiteId => metrics.get(websiteId) ?? { websiteId, users: 0, errors: 0 },
      ),
    });
  } catch (error) {
    return serverError(error);
  }
}
