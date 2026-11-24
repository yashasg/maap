import { memo, useCallback, useState } from 'react';
import { Chart, type ChartProps } from '@/components/charts/Chart';
import { ChartTooltip } from '@/components/charts/ChartTooltip';

const MemoChart = memo(Chart);

export interface BubbleChartProps extends ChartProps {
  type?: 'bubble';
}

function BubbleChartComponent({ type = 'bubble', ...props }: BubbleChartProps) {
  const [tooltip, setTooltip] = useState<{ color?: string; value: string } | null>(null);

  const handleTooltip = useCallback(({ tooltip }: { tooltip: any }) => {
    const { opacity, labelColors, title, dataPoints } = tooltip;
    const next = opacity
      ? {
          color: labelColors?.[0]?.backgroundColor,
          value: `${title}: ${dataPoints[0].raw}`,
        }
      : null;

    setTooltip(prev => (prev?.color === next?.color && prev?.value === next?.value ? prev : next));
  }, []);

  return (
    <>
      <MemoChart {...props} type={type} onTooltip={handleTooltip} />
      {tooltip && <ChartTooltip {...tooltip} />}
    </>
  );
}

export const BubbleChart = memo(BubbleChartComponent);

BubbleChart.displayName = 'BubbleChart';
