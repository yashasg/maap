import prisma from '@/lib/prisma';
import { getQueryFilters, parseRequest } from '@/lib/request';
import { json, serverError, unauthorized } from '@/lib/response';
import { canViewWebsite } from '@/permissions';
import { analyticsQuerySchema } from '../../analytics-schema';

const PROPERTY_DIMENSIONS = [
  ['TelemetryDeck.SDK.name', 'sdkName'],
  ['TelemetryDeck.SDK.version', 'sdkVersion'],
  ['TelemetryDeck.AppInfo.version', 'appVersion'],
  ['TelemetryDeck.AppInfo.buildNumber', 'buildNumber'],
  ['TelemetryDeck.Device.systemVersion', 'systemVersion'],
] as const;

type SummaryRow = {
  events: bigint | number;
  totalUsers: bigint | number;
  usersLastHour: bigint | number;
};

type DailyRow = { x: string; y: bigint | number };
type DimensionRow = { dimension: string; value: string; users: bigint | number };
type RetentionRow = {
  day: number;
  eligibleUsers: bigint | number;
  returningUsers: bigint | number;
  percentage: number | string | null;
};

function number(value: bigint | number | string | null | undefined) {
  return value == null ? 0 : Number(value);
}

function modePredicate(isTest: boolean, alias = 'e') {
  return isTest ? `${alias}.tag = 'test'` : `coalesce(${alias}.tag, '') != 'test'`;
}

export async function GET(
  request: Request,
  { params }: { params: Promise<{ websiteId: string }> },
) {
  const { auth, query, error } = await parseRequest(request, analyticsQuerySchema);
  if (error) return error();

  const { websiteId } = await params;
  if (!auth.user || !(await canViewWebsite({ user: auth.user }, websiteId))) return unauthorized();

  try {
    const filters = await getQueryFilters(query, websiteId);
    const { startDate, endDate, minimumDate, timezone = 'UTC' } = filters;
    const isTest = filters.tag === 'test' || filters.tag === 'eq.test';
    const now = new Date();
    const rollingHourStart = new Date(now.getTime() - 60 * 60 * 1000);
    const lastHourStart =
      minimumDate && minimumDate > rollingHourStart ? minimumDate : rollingHourStart;
    const eventMode = modePredicate(isTest);
    const historyFloorPredicate = minimumDate ? 'and e.created_at >= {{minimumDate}}' : '';

    const [summaryRows, dailyRows, dimensionRows, retentionRows] = await Promise.all([
      prisma.rawQuery(
        `
        with scoped as (
          select e.event_id, e.session_id, e.created_at, s.distinct_id
          from website_event e
          join session s
            on s.website_id = e.website_id and s.session_id = e.session_id
          where e.website_id = {{websiteId::uuid}}
            and e.created_at between {{startDate}} and {{endDate}}
            and ${eventMode}
        ),
        last_hour as (
          select count(distinct s.distinct_id) filter (
            where nullif(s.distinct_id, '') is not null
          ) as users
          from website_event e
          join session s
            on s.website_id = e.website_id and s.session_id = e.session_id
          where e.website_id = {{websiteId::uuid}}
            and e.created_at >= {{lastHourStart}}
            and e.created_at <= {{now}}
            and ${eventMode}
        )
        select
          count(*) as events,
          count(distinct distinct_id) filter (
            where nullif(distinct_id, '') is not null
          ) as "totalUsers",
          (select users from last_hour) as "usersLastHour"
        from scoped
        `,
        { websiteId, startDate, endDate, lastHourStart, now },
        'getAppOverviewSummary',
      ) as Promise<SummaryRow[]>,
      prisma.rawQuery(
        `
        with bounds as (
          select
            ({{startDate}}::timestamptz at time zone {{timezone}})::date as start_day,
            ({{endDate}}::timestamptz at time zone {{timezone}})::date as end_day
        ),
        days as (
          select generate_series(start_day, end_day, interval '1 day')::date as day
          from bounds
        ),
        counts as (
          select
            (e.created_at at time zone {{timezone}})::date as day,
            count(distinct s.distinct_id) filter (
              where nullif(s.distinct_id, '') is not null
            ) as users
          from website_event e
          join session s
            on s.website_id = e.website_id and s.session_id = e.session_id
          where e.website_id = {{websiteId::uuid}}
            and e.created_at between {{startDate}} and {{endDate}}
            and ${eventMode}
          group by 1
        )
        select to_char(days.day, 'YYYY-MM-DD') as x, coalesce(counts.users, 0) as y
        from days
        left join counts using (day)
        order by days.day
        `,
        { websiteId, startDate, endDate, timezone },
        'getAppOverviewDailyUsers',
      ) as Promise<DailyRow[]>,
      prisma.rawQuery(
        `
        with scoped as (
          select e.event_id, e.created_at, s.distinct_id
          from website_event e
          join session s
            on s.website_id = e.website_id and s.session_id = e.session_id
          where e.website_id = {{websiteId::uuid}}
            and e.created_at between {{startDate}} and {{endDate}}
            and nullif(s.distinct_id, '') is not null
            and ${eventMode}
        ),
        identities as (
          select distinct distinct_id from scoped
        ),
        keys(data_key, dimension) as (
          values
            ('TelemetryDeck.SDK.name', 'sdkName'),
            ('TelemetryDeck.SDK.version', 'sdkVersion'),
            ('TelemetryDeck.AppInfo.version', 'appVersion'),
            ('TelemetryDeck.AppInfo.buildNumber', 'buildNumber'),
            ('TelemetryDeck.Device.systemVersion', 'systemVersion')
        ),
        ranked as (
          select
            scoped.distinct_id,
            keys.dimension,
            d.string_value,
            row_number() over (
              partition by scoped.distinct_id, keys.dimension
              order by scoped.created_at desc, scoped.event_id desc
            ) as row_number
          from scoped
          join event_data d
            on d.website_id = {{websiteId::uuid}}
           and d.website_event_id = scoped.event_id
          join keys on keys.data_key = d.data_key
        ),
        latest as (
          select distinct_id, dimension, string_value
          from ranked
          where row_number = 1
        )
        select
          keys.dimension,
          coalesce(nullif(latest.string_value, ''), 'Unknown') as value,
          count(*) as users
        from identities
        cross join keys
        left join latest
          on latest.distinct_id = identities.distinct_id
         and latest.dimension = keys.dimension
        group by keys.dimension, coalesce(nullif(latest.string_value, ''), 'Unknown')
        order by keys.dimension, users desc, value
        `,
        { websiteId, startDate, endDate },
        'getAppOverviewDimensions',
      ) as Promise<DimensionRow[]>,
      prisma.rawQuery(
        `
        with first_seen as (
          select
            s.distinct_id,
            min((e.created_at at time zone {{timezone}})::date) as cohort_day
          from website_event e
          join session s
            on s.website_id = e.website_id and s.session_id = e.session_id
          where e.website_id = {{websiteId::uuid}}
            and e.created_at <= {{endDate}}
            ${historyFloorPredicate}
            and nullif(s.distinct_id, '') is not null
            and ${eventMode}
          group by s.distinct_id
        ),
        cohort as (
          select distinct_id, cohort_day
          from first_seen
          where cohort_day between
            ({{startDate}}::timestamptz at time zone {{timezone}})::date
            and ({{endDate}}::timestamptz at time zone {{timezone}})::date
        ),
        activity as (
          select distinct
            s.distinct_id,
            (e.created_at at time zone {{timezone}})::date as activity_day
          from website_event e
          join session s
            on s.website_id = e.website_id and s.session_id = e.session_id
          where e.website_id = {{websiteId::uuid}}
            and e.created_at between {{startDate}} and {{endDate}}
            and nullif(s.distinct_id, '') is not null
            and ${eventMode}
        ),
        intervals(day) as (values (1), (7), (14))
        select
          intervals.day,
          count(*) filter (
            where cohort.cohort_day + intervals.day
              <= ({{endDate}}::timestamptz at time zone {{timezone}})::date
          ) as "eligibleUsers",
          count(*) filter (
            where cohort.cohort_day + intervals.day
              <= ({{endDate}}::timestamptz at time zone {{timezone}})::date
              and exists (
                select 1 from activity
                where activity.distinct_id = cohort.distinct_id
                  and activity.activity_day = cohort.cohort_day + intervals.day
              )
          ) as "returningUsers",
          case
            when count(*) filter (
              where cohort.cohort_day + intervals.day
                <= ({{endDate}}::timestamptz at time zone {{timezone}})::date
            ) = 0 then null
            else round(
              100.0 * count(*) filter (
                where cohort.cohort_day + intervals.day
                  <= ({{endDate}}::timestamptz at time zone {{timezone}})::date
                  and exists (
                    select 1 from activity
                    where activity.distinct_id = cohort.distinct_id
                      and activity.activity_day = cohort.cohort_day + intervals.day
                  )
              )
              / count(*) filter (
                where cohort.cohort_day + intervals.day
                  <= ({{endDate}}::timestamptz at time zone {{timezone}})::date
              ),
              1
            )
          end as percentage
        from intervals
        left join cohort on true
        group by intervals.day
        order by intervals.day
        `,
        { websiteId, startDate, endDate, minimumDate, timezone },
        'getAppOverviewRetention',
      ) as Promise<RetentionRow[]>,
    ]);

    const summary = summaryRows[0] ?? { events: 0, totalUsers: 0, usersLastHour: 0 };
    const dimensions = Object.fromEntries(
      PROPERTY_DIMENSIONS.map(([, dimension]) => [
        dimension,
        dimensionRows
          .filter(row => row.dimension === dimension)
          .map(row => ({ value: row.value, users: number(row.users) })),
      ]),
    );

    return json({
      identity: 'clientUser',
      range: {
        startAt: startDate.toISOString(),
        endAt: endDate.toISOString(),
        timezone,
      },
      summary: {
        users: number(summary.totalUsers),
        usersLastHour: number(summary.usersLastHour),
        events: number(summary.events),
        errors: null,
        mostUsedSdk: dimensions.sdkName?.[0]?.value ?? null,
      },
      dailyUsers: dailyRows.map(row => ({ x: row.x, y: number(row.y) })),
      retention: retentionRows.map(row => ({
        day: row.day,
        eligibleUsers: number(row.eligibleUsers),
        returningUsers: number(row.returningUsers),
        percentage: row.percentage == null ? null : number(row.percentage),
      })),
      dimensions,
    });
  } catch (error) {
    return serverError(error);
  }
}
