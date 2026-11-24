import { isBefore, startOfMinute, subMinutes } from 'date-fns';
import { useMemo, useRef } from 'react';
import { useTimezone } from '@/components/hooks';
import { DEFAULT_ANIMATION_DURATION, REALTIME_RANGE } from '@/lib/constants';
import type { RealtimeData } from '@/lib/types';
import { PageviewsChart } from './PageviewsChart';

export interface RealtimeChartProps {
  data: RealtimeData;
  unit: string;
  className?: string;
}

export function RealtimeChart({ data, unit, ...props }: RealtimeChartProps) {
  const { formatSeriesTimezone, fromUtc, timezone } = useTimezone();
  const minuteBucket = Math.floor(Date.now() / 60_000);
  const { endDate, minDate, maxDate } = useMemo(() => {
    const end = startOfMinute(new Date(minuteBucket * 60_000));
    const start = subMinutes(end, REALTIME_RANGE);
    return {
      endDate: end,
      minDate: fromUtc(start),
      maxDate: fromUtc(end),
    };
  }, [minuteBucket, fromUtc]);
  const prevEndDate = useRef(endDate);
  const prevData = useRef<string | null>(null);

  const chartData = useMemo(() => {
    if (!data) {
      return { pageviews: [], sessions: [] };
    }

    return {
      pageviews: formatSeriesTimezone(data.series.views, 'x', timezone),
      sessions: formatSeriesTimezone(data.series.visitors, 'x', timezone),
    };
  }, [data, formatSeriesTimezone, timezone]);

  const animationDuration = useMemo(() => {
    // Don't animate the bars shifting over because it looks weird
    if (isBefore(prevEndDate.current, endDate)) {
      prevEndDate.current = endDate;
      return 0;
    }

    // Don't animate when data hasn't changed
    const serialized = JSON.stringify(chartData);
    if (prevData.current === serialized) {
      return 0;
    }
    prevData.current = serialized;

    return DEFAULT_ANIMATION_DURATION;
  }, [endDate, chartData]);

  return (
    <PageviewsChart
      {...props}
      minDate={minDate}
      maxDate={maxDate}
      unit={unit}
      data={chartData}
      animationDuration={animationDuration}
    />
  );
}
