import { z } from 'zod';
import prisma from '@/lib/prisma';
import { getQueryFilters, parseRequest } from '@/lib/request';
import { json, serverError, unauthorized } from '@/lib/response';
import { canViewWebsiteSection } from '@/permissions';
import { analyticsQuerySchema } from '../../analytics-schema';

const querySchema = analyticsQuerySchema.extend({
  testMode: z.enum(['true', 'false']).optional(),
});

type ReportRow = {
  section: 'summary' | 'series' | 'pattern' | 'breakdown';
  metric: string;
  bucket: string;
  activeUsers: bigint | number | null;
  newUsers: bigint | number | null;
  users: bigint | number | null;
};

function count(value: bigint | number | null) {
  return Number(value || 0);
}

function clampToFloor(date: Date, minimumDate?: Date) {
  return minimumDate && date < minimumDate ? minimumDate : date;
}

function subtractUtcMonths(date: Date, months: number) {
  const result = new Date(date);
  const day = result.getUTCDate();
  result.setUTCDate(1);
  result.setUTCMonth(result.getUTCMonth() - months);
  const lastDay = new Date(
    Date.UTC(result.getUTCFullYear(), result.getUTCMonth() + 1, 0),
  ).getUTCDate();
  result.setUTCDate(Math.min(day, lastDay));
  return result;
}

function response(rows: ReportRow[], scope: Record<string, unknown>, queryMilliseconds: number) {
  const series: Record<string, Record<string, unknown>[]> = {};
  const patterns: Record<string, Record<string, unknown>[]> = {};
  const breakdowns: Record<string, Record<string, unknown>[]> = {};
  const summary: Record<string, number> = {};

  for (const row of rows) {
    if (row.section === 'series') {
      series[row.metric] ||= [];
      series[row.metric].push({
        bucket: row.bucket,
        activeUsers: count(row.activeUsers),
        newUsers: count(row.newUsers),
      });
    } else if (row.section === 'pattern') {
      patterns[row.metric] ||= [];
      patterns[row.metric].push({ bucket: row.bucket, users: count(row.users) });
    } else if (row.section === 'breakdown') {
      breakdowns[row.metric] ||= [];
      breakdowns[row.metric].push({ name: row.bucket, users: count(row.users) });
    } else if (row.section === 'summary') {
      summary[row.metric] = count(row.users);
    }
  }

  return {
    identity: {
      unit: 'appUser',
      source: 'clientUser',
      unidentifiedEventsExcluded: true,
    },
    range: {
      ...scope,
      totalSignals: summary.totalSignals || 0,
      identifiedSignals: summary.identifiedSignals || 0,
    },
    series: {
      hourly: series.hourly || [],
      daily: series.daily || [],
      weekly: series.weekly || [],
      monthly: series.monthly || [],
    },
    patterns: {
      hourOfDay: patterns.hourOfDay || [],
      hourOfDay4Weeks: patterns.hourOfDay4Weeks || [],
      dayOfWeek: patterns.dayOfWeek || [],
      dayOfWeek4Weeks: patterns.dayOfWeek4Weeks || [],
      weekpart: patterns.weekpart || [],
      weekpart4Weeks: patterns.weekpart4Weeks || [],
    },
    breakdowns: {
      device: breakdowns.device || [],
      platform: breakdowns.platform || [],
      region: breakdowns.region || [],
      language: breakdowns.language || [],
    },
    availability: {
      userLocalTime: false,
      reason: "The SDK user's local timezone is not stored per signal.",
    },
    query: {
      milliseconds: queryMilliseconds,
      grains: {
        hourly: { unit: 'hour', window: 'last24Hours' },
        daily: { unit: 'day', window: 'selectedRange' },
        weekly: { unit: 'week', window: 'last3CalendarMonths' },
        monthly: { unit: 'month', window: 'last12CalendarMonths' },
      },
    },
  };
}

export async function GET(
  request: Request,
  { params }: { params: Promise<{ websiteId: string }> },
) {
  const { auth, query, error } = await parseRequest(request, querySchema);
  if (error) return error();

  const { websiteId } = await params;
  if (!(await canViewWebsiteSection(auth, websiteId, 'overview'))) {
    return unauthorized();
  }

  try {
    const filters = await getQueryFilters(query, websiteId);
    const { startDate, endDate, minimumDate, timezone = 'UTC' } = filters;
    const testMode =
      query.testMode === 'true' || filters.tag === 'test' || filters.tag === 'eq.test';
    const releasePredicate = testMode ? `e.tag = 'test'` : `coalesce(e.tag, '') <> 'test'`;
    const historyFloorPredicate = minimumDate ? 'and e.created_at >= {{minimumDate}}' : '';
    const hourlyStart = clampToFloor(
      new Date(endDate.getTime() - 24 * 60 * 60 * 1000),
      minimumDate,
    );
    const fourWeeksStart = clampToFloor(
      new Date(endDate.getTime() - 28 * 24 * 60 * 60 * 1000),
      minimumDate,
    );
    const weeklyStart = clampToFloor(subtractUtcMonths(endDate, 3), minimumDate);
    const monthlyStart = clampToFloor(subtractUtcMonths(endDate, 12), minimumDate);

    if (startDate > endDate) {
      return json(
        response(
          [],
          {
            startAt: startDate,
            endAt: endDate,
            timezone,
            testMode,
          },
          0,
        ),
      );
    }

    const started = performance.now();
    const rows = (await prisma.rawQuery(
      `
      with identity_candidates as (
        select website_id, session_id, nullif(distinct_id, '') as user_id
        from session
        where website_id = {{websiteId::uuid}} and nullif(distinct_id, '') is not null
        union all
        select website_id, session_id, nullif(distinct_id, '') as user_id
        from session_link
        where website_id = {{websiteId::uuid}} and nullif(distinct_id, '') is not null
      ),
      identity_sessions as (
        select website_id, session_id, min(user_id) as user_id
        from identity_candidates
        group by website_id, session_id
        having count(distinct user_id) = 1
      ),
      identified_events as materialized (
        select
          identities.user_id,
          e.event_id,
          e.session_id,
          e.created_at,
          e.event_type,
          e.tag,
          s.device,
          case
            when lower(coalesce(s.os, '')) like '%android%' then 'Android'
            when lower(coalesce(s.os, '')) like '%ios%' then 'iOS'
            when lower(coalesce(s.os, '')) like '%mac%' then 'macOS'
            when nullif(s.os, '') is not null then s.os
            else 'Unknown'
          end as platform,
          s.language,
          s.country,
          s.region
        from website_event e
        join identity_sessions identities
          on identities.website_id = e.website_id
          and identities.session_id = e.session_id
        join session s
          on s.website_id = e.website_id
          and s.session_id = e.session_id
        where e.website_id = {{websiteId::uuid}}
          ${historyFloorPredicate}
          and ${releasePredicate}
      ),
      first_seen_users as materialized (
        select distinct on (user_id)
          user_id,
          created_at as first_seen,
          device,
          platform,
          language,
          country,
          region
        from identified_events
        order by user_id, created_at, event_id
      ),
      report as (
        select
          'summary'::text as section,
          'totalSignals'::text as metric,
          ''::text as bucket,
          null::bigint as "activeUsers",
          null::bigint as "newUsers",
          count(*)::bigint as users
        from website_event e
        where e.website_id = {{websiteId::uuid}}
          and e.created_at between {{startDate}} and {{endDate}}
          and ${releasePredicate}

        union all
        select
          'summary', 'identifiedSignals', '', null::bigint, null::bigint, count(*)::bigint
        from identified_events
        where created_at between {{startDate}} and {{endDate}}

        union all
        select
          'series',
          grains.metric,
          to_char(
            date_trunc(grains.unit, events.created_at at time zone {{timezone}}),
            case when grains.unit = 'hour'
              then 'YYYY-MM-DD"T"HH24:00:00"Z"'
              else 'YYYY-MM-DD"T"00:00:00"Z"'
            end
          ),
          count(distinct events.user_id)::bigint,
          count(distinct events.user_id) filter (
            where date_trunc(grains.unit, first_seen.first_seen at time zone {{timezone}})
              = date_trunc(grains.unit, events.created_at at time zone {{timezone}})
          )::bigint,
          null::bigint
        from (
          values
            ('hourly'::text, 'hour'::text, {{hourlyStart}}::timestamptz),
            ('daily'::text, 'day'::text, {{startDate}}::timestamptz),
            ('weekly'::text, 'week'::text, {{weeklyStart}}::timestamptz),
            ('monthly'::text, 'month'::text, {{monthlyStart}}::timestamptz)
        ) as grains(metric, unit, range_start)
        join identified_events events
          on events.created_at >= grains.range_start and events.created_at <= {{endDate}}
        join first_seen_users first_seen using (user_id)
        group by grains.metric, grains.unit, 3

        union all
        select
          'pattern',
          patterns.metric,
          case
            when patterns.kind = 'hour'
              then extract(hour from first_seen at time zone {{timezone}})::integer::text
            when patterns.kind = 'day'
              then extract(isodow from first_seen at time zone {{timezone}})::integer::text
            else case
              when extract(isodow from first_seen at time zone {{timezone}}) in (6, 7)
                then 'Weekend'
              else 'Weekday'
            end
          end,
          null::bigint,
          null::bigint,
          count(*)::bigint
        from (
          values
            ('hourOfDay'::text, 'hour'::text, {{startDate}}::timestamptz),
            ('hourOfDay4Weeks'::text, 'hour'::text, {{fourWeeksStart}}::timestamptz),
            ('dayOfWeek'::text, 'day'::text, {{startDate}}::timestamptz),
            ('dayOfWeek4Weeks'::text, 'day'::text, {{fourWeeksStart}}::timestamptz),
            ('weekpart'::text, 'weekpart'::text, {{startDate}}::timestamptz),
            ('weekpart4Weeks'::text, 'weekpart'::text, {{fourWeeksStart}}::timestamptz)
        ) as patterns(metric, kind, range_start)
        join first_seen_users on first_seen >= patterns.range_start and first_seen <= {{endDate}}
        group by patterns.metric, patterns.kind, 3

        union all
        select
          'breakdown',
          dimensions.metric,
          coalesce(nullif(
            case dimensions.metric
              when 'device' then device
              when 'platform' then platform
              when 'region' then region
              when 'language' then language
            end,
            ''
          ), 'Unknown'),
          null::bigint,
          null::bigint,
          count(*)::bigint
        from (
          values ('device'::text), ('platform'::text), ('region'::text), ('language'::text)
        ) as dimensions(metric)
        cross join first_seen_users
        where first_seen between {{startDate}} and {{endDate}}
        group by dimensions.metric, 3
      )
      select section, metric, bucket, "activeUsers", "newUsers", users
      from report
      order by section, metric, bucket
      `,
      {
        websiteId,
        startDate,
        endDate,
        minimumDate,
        timezone,
        hourlyStart,
        fourWeeksStart,
        weeklyStart,
        monthlyStart,
      },
      'getAppAcquisitionReport',
    )) as ReportRow[];

    return json(
      response(
        rows,
        {
          startAt: startDate,
          endAt: endDate,
          timezone,
          testMode,
        },
        performance.now() - started,
      ),
    );
  } catch (error) {
    return serverError(error);
  }
}
