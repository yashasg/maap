import { subHours, subMonths } from 'date-fns';
import { SHARE_CONTEXT_HEADER, SHARE_TOKEN_HEADER } from '@/lib/constants';
import prisma from '@/lib/prisma';
import { getQueryFilters, parseRequest } from '@/lib/request';
import { json, serverError, unauthorized } from '@/lib/response';
import { canViewWebsite } from '@/permissions';
import { analyticsQuerySchema } from '../../analytics-schema';

const TOTAL_SESSIONS_KEY = 'TelemetryDeck.Retention.totalSessionsCount';
const AVERAGE_SECONDS_KEY = 'TelemetryDeck.Retention.averageSessionSeconds';
const HOUR_OF_DAY_KEY = 'TelemetryDeck.Calendar.hourOfDay';
const DAY_OF_WEEK_KEY = 'TelemetryDeck.Calendar.dayOfWeek';
const IS_WEEKEND_KEY = 'TelemetryDeck.Calendar.isWeekend';
const PREFERRED_REGION_KEY = 'TelemetryDeck.UserPreference.region';

const requiredAllowlist = [
  TOTAL_SESSIONS_KEY,
  AVERAGE_SECONDS_KEY,
  HOUR_OF_DAY_KEY,
  DAY_OF_WEEK_KEY,
  IS_WEEKEND_KEY,
  PREFERRED_REGION_KEY,
];

type ReportRow = {
  section: string;
  metric: string;
  bucket: string;
  users: bigint | number;
};

function number(value: bigint | number | undefined) {
  return value == null ? 0 : Number(value);
}

function modePredicate(isTest: boolean, alias = 'e') {
  return isTest ? `${alias}.tag = 'test'` : `coalesce(${alias}.tag, '') != 'test'`;
}

function maxDate(left: Date, right: Date) {
  return left > right ? left : right;
}

export async function GET(
  request: Request,
  { params }: { params: Promise<{ websiteId: string }> },
) {
  const { auth, query, error } = await parseRequest(request, analyticsQuerySchema);
  if (error) return error();

  const { websiteId } = await params;
  if (
    !auth.user ||
    request.headers.has(SHARE_CONTEXT_HEADER) ||
    request.headers.has(SHARE_TOKEN_HEADER) ||
    !(await canViewWebsite({ user: auth.user }, websiteId))
  ) {
    return unauthorized();
  }

  try {
    const filters = await getQueryFilters(query, websiteId);
    const { startDate, endDate, minimumDate, timezone = 'UTC' } = filters;
    const isTest = query.testMode === 'true' || filters.tag === 'test' || filters.tag === 'eq.test';
    const eventMode = modePredicate(isTest);
    const historyFloorPredicate = minimumDate ? 'and e.created_at >= {{minimumDate}}' : '';
    const hourlyStart = maxDate(startDate, subHours(endDate, 24));
    const fourWeeksStart = maxDate(startDate, subHours(endDate, 24 * 28));
    const weeklyStart = maxDate(startDate, subMonths(endDate, 3));
    const monthlyStart = maxDate(startDate, subMonths(endDate, 12));

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
          s.language
        from website_event e
        join identity_sessions identities
          on identities.website_id = e.website_id and identities.session_id = e.session_id
        join session s
          on s.website_id = e.website_id and s.session_id = e.session_id
        where e.website_id = {{websiteId::uuid}}
          ${historyFloorPredicate}
          and ${eventMode}
      ),
      activation_inputs as materialized (
        select
          ie.user_id,
          ie.event_id,
          ie.created_at,
          ie.device,
          ie.platform,
          ie.language,
          max(ed.string_value) filter (
            where ed.data_key = '${TOTAL_SESSIONS_KEY}'
          ) as total_sessions_text,
          max(ed.string_value) filter (
            where ed.data_key = '${AVERAGE_SECONDS_KEY}'
          ) as average_seconds_text,
          max(ed.string_value) filter (
            where ed.data_key = '${HOUR_OF_DAY_KEY}'
          ) as user_hour_text,
          max(ed.string_value) filter (
            where ed.data_key = '${DAY_OF_WEEK_KEY}'
          ) as user_day_text,
          max(ed.string_value) filter (
            where ed.data_key = '${IS_WEEKEND_KEY}'
          ) as user_weekend_text,
          max(ed.string_value) filter (
            where ed.data_key = '${PREFERRED_REGION_KEY}'
          ) as preferred_region
        from identified_events ie
        left join event_data ed
          on ed.website_id = {{websiteId::uuid}}
         and ed.website_event_id = ie.event_id
         and ed.data_key in (
           '${TOTAL_SESSIONS_KEY}',
           '${AVERAGE_SECONDS_KEY}',
           '${HOUR_OF_DAY_KEY}',
           '${DAY_OF_WEEK_KEY}',
           '${IS_WEEKEND_KEY}',
           '${PREFERRED_REGION_KEY}'
         )
        group by ie.user_id, ie.event_id, ie.created_at, ie.device, ie.platform, ie.language
      ),
      activation_observations as materialized (
        select
          *,
          case
            when total_sessions_text ~ '^[0-9]+$' then total_sessions_text::integer
          end as total_sessions,
          case
            when average_seconds_text ~ '^-?[0-9]+([.][0-9]+)?$'
              then average_seconds_text::numeric
          end as average_seconds,
          case when user_hour_text ~ '^[0-9]+$' then user_hour_text::integer end as user_hour,
          case when user_day_text ~ '^[0-9]+$' then user_day_text::integer end as user_day,
          case lower(user_weekend_text)
            when 'true' then true
            when 'false' then false
          end as user_weekend
        from activation_inputs
      ),
      activation_candidates as materialized (
        select
          *,
          case
            when total_sessions = 1 then average_seconds
            when total_sessions between 2 and 6 then average_seconds * (total_sessions - 1)
          end as accumulated_seconds
        from activation_observations
        where average_seconds >= 0
      ),
      activated_users as materialized (
        select distinct on (user_id)
          user_id,
          created_at as activation_at,
          device,
          platform,
          language,
          preferred_region,
          user_hour,
          user_day,
          user_weekend
        from activation_candidates
        where total_sessions between 1 and 6 and accumulated_seconds >= 300
        order by user_id, created_at, event_id
      )
      select 'summary' as section, 'totalSignals' as metric, '' as bucket, count(*) as users
      from website_event e
      where e.website_id = {{websiteId::uuid}}
        and e.created_at between {{startDate}} and {{endDate}}
        and ${eventMode}
      union all
      select 'summary', 'identifiedSignals', '', count(*)
      from identified_events
      where created_at between {{startDate}} and {{endDate}}
      union all
      select 'summary', 'inputSignals', '', count(*)
      from activation_observations
      where created_at between {{startDate}} and {{endDate}}
        and total_sessions is not null and average_seconds is not null
      union all
      select 'summary', 'activatedUsers', '', count(*)
      from activated_users
      where activation_at between {{startDate}} and {{endDate}}
      union all
      select 'availability', 'hasTotalSessions', '',
        count(*) filter (where nullif(total_sessions_text, '') is not null)
      from activation_inputs where created_at between {{startDate}} and {{endDate}}
      union all
      select 'availability', 'hasAverageSeconds', '',
        count(*) filter (where nullif(average_seconds_text, '') is not null)
      from activation_inputs where created_at between {{startDate}} and {{endDate}}
      union all
      select 'availability', 'hasHourOfDay', '',
        count(*) filter (where nullif(user_hour_text, '') is not null)
      from activation_inputs where created_at between {{startDate}} and {{endDate}}
      union all
      select 'availability', 'hasDayOfWeek', '',
        count(*) filter (where nullif(user_day_text, '') is not null)
      from activation_inputs where created_at between {{startDate}} and {{endDate}}
      union all
      select 'availability', 'hasIsWeekend', '',
        count(*) filter (where nullif(user_weekend_text, '') is not null)
      from activation_inputs where created_at between {{startDate}} and {{endDate}}
      union all
      select 'availability', 'hasPreferredRegion', '',
        count(*) filter (where nullif(preferred_region, '') is not null)
      from activation_inputs where created_at between {{startDate}} and {{endDate}}
      union all
      select 'series', 'hourly',
        to_char(date_trunc('hour', activation_at at time zone {{timezone}}), 'YYYY-MM-DD"T"HH24:00'),
        count(*)
      from activated_users
      where activation_at between {{hourlyStart}} and {{endDate}}
      group by 3
      union all
      select 'series', 'daily',
        to_char(date_trunc('day', activation_at at time zone {{timezone}}), 'YYYY-MM-DD'),
        count(*)
      from activated_users
      where activation_at between {{startDate}} and {{endDate}}
      group by 3
      union all
      select 'series', 'weekly',
        to_char(date_trunc('week', activation_at at time zone {{timezone}}), 'YYYY-MM-DD'),
        count(*)
      from activated_users
      where activation_at between {{weeklyStart}} and {{endDate}}
      group by 3
      union all
      select 'series', 'monthly',
        to_char(date_trunc('month', activation_at at time zone {{timezone}}), 'YYYY-MM-DD'),
        count(*)
      from activated_users
      where activation_at between {{monthlyStart}} and {{endDate}}
      group by 3
      union all
      select 'pattern', 'hourOfDay', user_hour::text, count(*)
      from activated_users
      where activation_at between {{startDate}} and {{endDate}} and user_hour between 1 and 24
      group by 3
      union all
      select 'pattern', 'hourOfDay4Weeks',
        extract(hour from activation_at at time zone {{timezone}})::integer::text, count(*)
      from activated_users
      where activation_at between {{fourWeeksStart}} and {{endDate}}
      group by 3
      union all
      select 'pattern', 'dayOfWeek', user_day::text, count(*)
      from activated_users
      where activation_at between {{startDate}} and {{endDate}} and user_day between 1 and 7
      group by 3
      union all
      select 'pattern', 'dayOfWeek4Weeks',
        extract(isodow from activation_at at time zone {{timezone}})::integer::text, count(*)
      from activated_users
      where activation_at between {{fourWeeksStart}} and {{endDate}}
      group by 3
      union all
      select 'pattern', 'weekpart',
        case when user_weekend then 'Weekend' else 'Weekday' end, count(*)
      from activated_users
      where activation_at between {{startDate}} and {{endDate}} and user_weekend is not null
      group by 3
      union all
      select 'pattern', 'weekpart4Weeks',
        case when extract(isodow from activation_at at time zone {{timezone}}) in (6, 7)
          then 'Weekend' else 'Weekday' end,
        count(*)
      from activated_users
      where activation_at between {{fourWeeksStart}} and {{endDate}}
      group by 3
      union all
      select 'breakdown', 'device', coalesce(nullif(device, ''), 'Unknown'), count(*)
      from activated_users
      where activation_at between {{startDate}} and {{endDate}}
      group by 3
      union all
      select 'breakdown', 'platform', coalesce(nullif(platform, ''), 'Unknown'), count(*)
      from activated_users
      where activation_at between {{startDate}} and {{endDate}}
      group by 3
      union all
      select 'breakdown', 'region', preferred_region, count(*)
      from activated_users
      where activation_at between {{startDate}} and {{endDate}}
        and nullif(preferred_region, '') is not null
      group by 3
      union all
      select 'breakdown', 'language', language, count(*)
      from activated_users
      where activation_at between {{startDate}} and {{endDate}} and nullif(language, '') is not null
      group by 3
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
      'getAppActivationReport',
    )) as ReportRow[];

    return json(toResponse(rows, { startDate, endDate, timezone, isTest }));
  } catch (error) {
    return serverError(error);
  }
}

function toResponse(
  rows: ReportRow[],
  scope: { startDate: Date; endDate: Date; timezone: string; isTest: boolean },
) {
  const summary = new Map<string, number>();
  const availability = new Map<string, number>();
  const series = new Map<string, { key: string; label: string; count: number }[]>();
  const patterns = new Map<string, { key: string; label: string; count: number }[]>();
  const breakdowns = new Map<string, { key: string; label: string; count: number }[]>();

  for (const row of rows) {
    if (row.section === 'summary') summary.set(row.metric, number(row.users));
    if (row.section === 'availability') availability.set(row.metric, number(row.users));
    if (['series', 'pattern', 'breakdown'].includes(row.section)) {
      const target =
        row.section === 'series' ? series : row.section === 'pattern' ? patterns : breakdowns;
      const points = target.get(row.metric) ?? [];
      points.push({ key: row.bucket, label: row.bucket, count: number(row.users) });
      target.set(row.metric, points);
    }
  }

  const totalSignals = summary.get('totalSignals') ?? 0;
  const identifiedSignals = summary.get('identifiedSignals') ?? 0;
  const inputSignals = summary.get('inputSignals') ?? 0;
  const missingKeys = [
    ['hasTotalSessions', TOTAL_SESSIONS_KEY],
    ['hasAverageSeconds', AVERAGE_SECONDS_KEY],
    ['hasHourOfDay', HOUR_OF_DAY_KEY],
    ['hasDayOfWeek', DAY_OF_WEEK_KEY],
    ['hasIsWeekend', IS_WEEKEND_KEY],
    ['hasPreferredRegion', PREFERRED_REGION_KEY],
  ]
    .filter(([metric]) => !availability.get(metric))
    .map(([, key]) => key);
  const coreReady =
    !missingKeys.includes(TOTAL_SESSIONS_KEY) && !missingKeys.includes(AVERAGE_SECONDS_KEY);
  const status =
    totalSignals === 0
      ? 'empty'
      : identifiedSignals === 0
        ? 'identity-missing'
        : !coreReady || inputSignals === 0
          ? 'configuration-missing'
          : inputSignals < identifiedSignals
            ? 'partial'
            : 'ready';

  return {
    definition: {
      rule: 'At least 5 minutes accumulated total usage time within the first 5 sessions.',
      thresholdSeconds: 300,
      sessionLimit: 5,
      source: 'TelemetryDeck Activation Analytics documentation',
      configurable: false,
      calculation:
        'Maap uses the earliest stored SDK signal that proves the threshold was reached. For sessions 2–5, completed-session averages provide a lower bound; a fifth-session activation can become provable when session 6 starts.',
    },
    configuration: {
      status,
      coreReady,
      missingKeys,
      requiredAllowlist,
      environmentVariable: 'TD_ALLOWED_KEYS',
      historicalBackfill: false,
    },
    scope: {
      startAt: scope.startDate.toISOString(),
      endAt: scope.endDate.toISOString(),
      timezone: scope.timezone,
      testMode: scope.isTest,
      totalSignals,
      identifiedSignals,
      activationInputSignals: inputSignals,
      activatedUsers: summary.get('activatedUsers') ?? 0,
    },
    series: {
      hourly: series.get('hourly') ?? [],
      daily: series.get('daily') ?? [],
      weekly: series.get('weekly') ?? [],
      monthly: series.get('monthly') ?? [],
    },
    patterns: {
      hourOfDay: patterns.get('hourOfDay') ?? [],
      hourOfDay4Weeks: patterns.get('hourOfDay4Weeks') ?? [],
      dayOfWeek: patterns.get('dayOfWeek') ?? [],
      dayOfWeek4Weeks: patterns.get('dayOfWeek4Weeks') ?? [],
      weekpart: patterns.get('weekpart') ?? [],
      weekpart4Weeks: patterns.get('weekpart4Weeks') ?? [],
    },
    breakdowns: {
      device: breakdowns.get('device') ?? [],
      platform: breakdowns.get('platform') ?? [],
      region: breakdowns.get('region') ?? [],
      language: breakdowns.get('language') ?? [],
    },
    query: {
      identity: 'clientUser',
      unidentifiedSignalsExcluded: true,
    },
  };
}
