import { memo, useCallback, useState } from 'react';
import { Chart, type ChartProps } from '@/components/charts/Chart';
import { ChartTooltip } from '@/components/charts/ChartTooltip';

export interface PieChartProps extends ChartProps {
  type?: 'doughnut' | 'pie';
}

function PieChartComponent({ type = 'pie', height = '300px', ...props }: PieChartProps) {
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
      <Chart {...props} type={type} height={height} onTooltip={handleTooltip} />
      {tooltip && <ChartTooltip {...tooltip} />}
    </>
  );
}

export const PieChart = memo(PieChartComponent);

PieChart.displayName = 'PieChart';
