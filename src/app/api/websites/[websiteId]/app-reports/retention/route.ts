import prisma from '@/lib/prisma';
import { getQueryFilters, parseRequest } from '@/lib/request';
import { json, serverError, unauthorized } from '@/lib/response';
import { canViewWebsiteSection } from '@/permissions';
import { analyticsQuerySchema } from '../../analytics-schema';

type ReportRow = {
  section: 'series' | 'pattern' | 'breakdown' | 'scope';
  metric: string;
  bucket: string;
  count: bigint | number;
};

function number(value: bigint | number | null | undefined) {
  return value == null ? 0 : Number(value);
}

function modePredicate(isTest: boolean, alias = 'e') {
  return isTest ? `${alias}.tag = 'test'` : `coalesce(${alias}.tag, '') != 'test'`;
}

function returningCondition(unit: 'hour' | 'day' | 'week' | 'month') {
  return `first_seen.first_seen < (
    date_trunc('${unit}', ie.created_at at time zone {{timezone}}) at time zone {{timezone}}
  )`;
}

function series(metric: string, unit: 'hour' | 'day' | 'week' | 'month', start: string) {
  const format = unit === 'hour' ? 'YYYY-MM-DD"T"HH24:00:00"Z"' : 'YYYY-MM-DD"T"00:00:00"Z"';
  return `
    select
      'series'::text as section,
      '${metric}'::text as metric,
      to_char(date_trunc('${unit}', ie.created_at at time zone {{timezone}}), '${format}') as bucket,
      count(distinct ie.user_id)::bigint as count
    from identified_events ie
    join first_seen_users first_seen using (user_id)
    where ie.created_at between {{${start}}} and {{endDate}}
      and ${returningCondition(unit)}
    group by 3
  `;
}

function pattern(metric: string, bucket: 'hour' | 'dayOfWeek' | 'weekpart', start: string) {
  const bucketSql = {
    hour: `extract(hour from ie.created_at at time zone {{timezone}})::integer::text`,
    dayOfWeek: `extract(isodow from ie.created_at at time zone {{timezone}})::integer::text`,
    weekpart: `case
      when extract(isodow from ie.created_at at time zone {{timezone}}) in (6, 7)
      then 'Weekend' else 'Weekday' end`,
  }[bucket];
  return `
    select
      'pattern'::text as section,
      '${metric}'::text as metric,
      ${bucketSql} as bucket,
      count(distinct ie.user_id)::bigint as count
    from daily_returning_events ie
    where ie.created_at between {{${start}}} and {{endDate}}
    group by 3
  `;
}

function breakdown(metric: string, field: 'device' | 'platform' | 'language') {
  return `
    select
      'breakdown'::text as section,
      '${metric}'::text as metric,
      coalesce(nullif(ie.${field}, ''), 'Unknown')::text as bucket,
      count(distinct ie.user_id)::bigint as count
    from latest_returning_users ie
    where ie.created_at between {{startDate}} and {{endDate}}
    group by 3
  `;
}

export async function GET(
  request: Request,
  { params }: { params: Promise<{ websiteId: string }> },
) {
  const { auth, query, error } = await parseRequest(request, analyticsQuerySchema);
  if (error) return error();

  const { websiteId } = await params;
  if (!(await canViewWebsiteSection(auth, websiteId, 'retention'))) return unauthorized();

  try {
    const filters = await getQueryFilters(query, websiteId);
    const { startDate, endDate, minimumDate, timezone = 'UTC' } = filters;
    const isTest = filters.tag === 'test' || filters.tag === 'eq.test';
    const eventMode = modePredicate(isTest);
    const historyFloorPredicate = minimumDate ? 'and e.created_at >= {{minimumDate}}' : '';
    const maxStart = (duration: number) =>
      new Date(Math.max(startDate.getTime(), endDate.getTime() - duration));
    const queryParams = {
      websiteId,
      startDate,
      endDate,
      minimumDate,
      timezone,
      hourStart: maxStart(24 * 60 * 60 * 1000),
      fourWeekStart: maxStart(28 * 24 * 60 * 60 * 1000),
      weekStart: maxStart(93 * 24 * 60 * 60 * 1000),
      monthStart: maxStart(366 * 24 * 60 * 60 * 1000),
    };

    const sql = `
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
          e.created_at,
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
          s.region
        from website_event e
        join identity_sessions identities
          on identities.website_id = e.website_id and identities.session_id = e.session_id
        join session s
          on s.website_id = e.website_id and s.session_id = e.session_id
        where e.website_id = {{websiteId::uuid}}
          ${historyFloorPredicate}
          and e.created_at <= {{endDate}}
          and ${eventMode}
      ),
      first_seen_users as materialized (
        select distinct on (user_id)
          user_id, created_at as first_seen
        from identified_events
        order by user_id, created_at, event_id
      ),
      daily_returning_events as materialized (
        select ie.*
        from identified_events ie
        join first_seen_users first_seen using (user_id)
        where ie.created_at between {{startDate}} and {{endDate}}
          and ${returningCondition('day')}
      ),
      latest_returning_users as materialized (
        select distinct on (user_id)
          user_id, event_id, created_at, device, platform, language
        from daily_returning_events
        order by user_id, created_at desc, event_id desc
      )
      ${[
        series('hourly', 'hour', 'hourStart'),
        series('daily', 'day', 'startDate'),
        series('weekly', 'week', 'weekStart'),
        series('monthly', 'month', 'monthStart'),
        pattern('hourOfDay', 'hour', 'startDate'),
        pattern('hourOfDayFourWeeks', 'hour', 'fourWeekStart'),
        pattern('dayOfWeek', 'dayOfWeek', 'startDate'),
        pattern('dayOfWeekFourWeeks', 'dayOfWeek', 'fourWeekStart'),
        pattern('weekpart', 'weekpart', 'startDate'),
        pattern('weekpartFourWeeks', 'weekpart', 'fourWeekStart'),
        breakdown('device', 'device'),
        breakdown('platform', 'platform'),
        breakdown('language', 'language'),
        `
          select 'scope'::text, 'totalSignals'::text, ''::text, count(*)::bigint
          from website_event e
          where e.website_id = {{websiteId::uuid}}
            and e.created_at between {{startDate}} and {{endDate}}
            and ${eventMode}
        `,
        `
          select 'scope'::text, 'identifiedSignals'::text, ''::text, count(*)::bigint
          from identified_events
          where created_at between {{startDate}} and {{endDate}}
        `,
        `
          select
            'scope'::text, 'returningUsers'::text, ''::text,
            count(distinct ie.user_id)::bigint
          from daily_returning_events ie
        `,
      ].join(' union all ')}
      order by section, metric, bucket
    `;

    const rows = (await prisma.rawQuery(sql, queryParams, 'getAppRetentionReport')) as ReportRow[];
    const seriesRows: Record<string, { bucket: string; returningUsers: number }[]> = {};
    const patternRows: Record<string, { bucket: string; users: number }[]> = {};
    const breakdownRows: Record<string, { name: string; users: number }[]> = {};
    const scope: Record<string, number> = {};

    for (const row of rows) {
      if (row.section === 'series') {
        seriesRows[row.metric] = seriesRows[row.metric] ?? [];
        seriesRows[row.metric].push({
          bucket: row.bucket,
          returningUsers: number(row.count),
        });
      } else if (row.section === 'pattern') {
        patternRows[row.metric] = patternRows[row.metric] ?? [];
        patternRows[row.metric].push({ bucket: row.bucket, users: number(row.count) });
      } else if (row.section === 'breakdown') {
        breakdownRows[row.metric] = breakdownRows[row.metric] ?? [];
        breakdownRows[row.metric].push({ name: row.bucket, users: number(row.count) });
      } else {
        scope[row.metric] = number(row.count);
      }
    }

    return json({
      criteria: {
        verified: false,
        rule: 'An identified user has activity in a time bucket and their first stored signal is before that bucket began.',
        verification: 'TelemetryDeck parity has not been independently verified.',
      },
      identity: {
        unit: 'appUser',
        source: 'clientUser',
        unidentifiedEventsExcluded: true,
      },
      range: {
        startAt: startDate.toISOString(),
        endAt: endDate.toISOString(),
        timezone,
        testMode: isTest,
      },
      scope: {
        totalSignals: scope.totalSignals ?? 0,
        identifiedSignals: scope.identifiedSignals ?? 0,
        returningUsers: scope.returningUsers ?? 0,
      },
      series: {
        hourly: seriesRows.hourly ?? [],
        daily: seriesRows.daily ?? [],
        weekly: seriesRows.weekly ?? [],
        monthly: seriesRows.monthly ?? [],
      },
      patterns: {
        hourOfDay: patternRows.hourOfDay ?? [],
        hourOfDayFourWeeks: patternRows.hourOfDayFourWeeks ?? [],
        dayOfWeek: patternRows.dayOfWeek ?? [],
        dayOfWeekFourWeeks: patternRows.dayOfWeekFourWeeks ?? [],
        weekpart: patternRows.weekpart ?? [],
        weekpartFourWeeks: patternRows.weekpartFourWeeks ?? [],
      },
      breakdowns: {
        device: breakdownRows.device ?? [],
        platform: breakdownRows.platform ?? [],
        region: breakdownRows.region ?? [],
        language: breakdownRows.language ?? [],
      },
      availability: {
        userLocalTime: false,
        preferredRegion: false,
        reason: "The SDK user's local timezone and preferred region are not stored per signal.",
      },
      query: {
        grains: {
          hourly: { unit: 'hour', window: 'last24HoursWithinSelectedRange' },
          daily: { unit: 'day', window: 'selectedRange' },
          weekly: { unit: 'week', window: 'last93DaysWithinSelectedRange' },
          monthly: { unit: 'month', window: 'last366DaysWithinSelectedRange' },
        },
      },
    });
  } catch (error) {
    return serverError(error);
  }
}
