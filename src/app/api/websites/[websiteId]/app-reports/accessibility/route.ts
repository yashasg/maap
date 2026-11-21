import prisma from '@/lib/prisma';
import { getQueryFilters, parseRequest } from '@/lib/request';
import { json, serverError, unauthorized } from '@/lib/response';
import { canViewWebsiteSection } from '@/permissions';
import { analyticsQuerySchema } from '../../analytics-schema';

const METRICS = [
  {
    id: 'preferredContentSize',
    property: 'TelemetryDeck.Accessibility.preferredContentSizeCategory',
    kind: 'contentSize',
  },
  {
    id: 'boldText',
    property: 'TelemetryDeck.Accessibility.isBoldTextEnabled',
    kind: 'boolean',
  },
  {
    id: 'reduceMotion',
    property: 'TelemetryDeck.Accessibility.isReduceMotionEnabled',
    kind: 'boolean',
  },
  {
    id: 'reduceTransparency',
    property: 'TelemetryDeck.Accessibility.isReduceTransparencyEnabled',
    kind: 'boolean',
  },
  {
    id: 'darkerSystemColors',
    property: 'TelemetryDeck.Accessibility.isDarkerSystemColorsEnabled',
    kind: 'boolean',
  },
  {
    id: 'differentiateWithoutColor',
    property: 'TelemetryDeck.Accessibility.shouldDifferentiateWithoutColor',
    kind: 'boolean',
  },
  {
    id: 'invertedColors',
    property: 'TelemetryDeck.Accessibility.isInvertColorsEnabled',
    kind: 'boolean',
  },
] as const;

type Metric = (typeof METRICS)[number];

type BreakdownRow = {
  metricIndex: number;
  present: boolean;
  rawValue: string | null;
  signals: bigint | number;
};

function safeLabel(metric: Metric, present: boolean, rawValue: string | null) {
  if (!present) return 'Not reported';

  const value = rawValue?.trim() ?? '';
  if (metric.kind === 'boolean') {
    if (/^(true|1)$/i.test(value)) return 'On';
    if (/^(false|0)$/i.test(value)) return 'Off';
    return 'Other reported value';
  }

  return (
    {
      XS: 'Extra small',
      S: 'Small',
      M: 'Medium',
      L: 'Large',
      XL: 'Extra large',
      XXL: '2× extra large',
      XXXL: '3× extra large',
      AccessibilityM: 'Accessibility medium',
      AccessibilityL: 'Accessibility large',
      AccessibilityXL: 'Accessibility extra large',
      AccessibilityXXL: 'Accessibility 2× extra large',
      AccessibilityXXXL: 'Accessibility 3× extra large',
    }[value] ?? 'Other reported size'
  );
}

function labelOrder(metric: Metric, label: string) {
  const booleanOrder = ['On', 'Off', 'Other reported value', 'Not reported'];
  const contentSizeOrder = [
    'Extra small',
    'Small',
    'Medium',
    'Large',
    'Extra large',
    '2× extra large',
    '3× extra large',
    'Accessibility medium',
    'Accessibility large',
    'Accessibility extra large',
    'Accessibility 2× extra large',
    'Accessibility 3× extra large',
    'Other reported size',
    'Not reported',
  ];
  const index = (metric.kind === 'boolean' ? booleanOrder : contentSizeOrder).indexOf(label);
  return index < 0 ? Number.MAX_SAFE_INTEGER : index;
}

export function buildAccessibilityResponse(rows: BreakdownRow[]) {
  return {
    reports: METRICS.map((metric, metricIndex) => {
      const counts = new Map<string, number>();
      for (const row of rows.filter(row => Number(row.metricIndex) === metricIndex)) {
        const label = safeLabel(metric, row.present, row.rawValue);
        counts.set(label, (counts.get(label) ?? 0) + Number(row.signals));
      }

      if (metric.kind === 'boolean') {
        for (const label of ['On', 'Off', 'Not reported']) {
          if (!counts.has(label)) counts.set(label, 0);
        }
      }

      const values = [...counts]
        .sort(([a], [b]) => labelOrder(metric, a) - labelOrder(metric, b))
        .map(([label, signals]) => ({ label, signals }));
      const totalSignals = values.reduce((sum, value) => sum + value.signals, 0);
      const missingSignals = values.find(value => value.label === 'Not reported')?.signals ?? 0;

      return {
        id: metric.id,
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
    const testMode =
      query.testMode === 'true' || filters.tag === 'test' || filters.tag === 'eq.test';
    const modePredicate = testMode ? "e.tag = 'test'" : "coalesce(e.tag, '') != 'test'";
    const rows = (await prisma.rawQuery(
      `
      with filtered_events as (
        select e.event_id, e.website_id
        from website_event e
        where e.website_id = {{websiteId::uuid}}
          and e.created_at between {{startDate}} and {{endDate}}
          and e.event_type in (1, 2)
          and ${modePredicate}
      ),
      metrics(metric_index, property_name) as (
        values
          (0, 'TelemetryDeck.Accessibility.preferredContentSizeCategory'),
          (1, 'TelemetryDeck.Accessibility.isBoldTextEnabled'),
          (2, 'TelemetryDeck.Accessibility.isReduceMotionEnabled'),
          (3, 'TelemetryDeck.Accessibility.isReduceTransparencyEnabled'),
          (4, 'TelemetryDeck.Accessibility.isDarkerSystemColorsEnabled'),
          (5, 'TelemetryDeck.Accessibility.shouldDifferentiateWithoutColor'),
          (6, 'TelemetryDeck.Accessibility.isInvertColorsEnabled')
      )
      select
        metrics.metric_index as "metricIndex",
        coalesce(value.present, false) as "present",
        value.raw_value as "rawValue",
        count(*) as "signals"
      from metrics
      cross join filtered_events e
      left join lateral (
        select true as present, d.string_value as raw_value
        from event_data d
        where d.website_id = e.website_id
          and d.website_event_id = e.event_id
          and d.data_key = metrics.property_name
        order by d.created_at desc nulls last, d.event_data_id desc
        limit 1
      ) value on true
      group by metrics.metric_index, value.present, value.raw_value
      order by metrics.metric_index
      `,
      {
        websiteId,
        startDate: filters.startDate,
        endDate: filters.endDate,
      },
      'getAppAccessibilityBreakdowns',
    )) as BreakdownRow[];

    return json(buildAccessibilityResponse(rows));
  } catch (error) {
    return serverError(error);
  }
}
