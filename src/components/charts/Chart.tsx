import { Box, type BoxProps, Column } from '@umami/react-zen';
import {
  ArcElement,
  BarController,
  BarElement,
  BubbleController,
  CategoryScale,
  type ChartData,
  Chart as ChartJS,
  Legend as ChartLegend,
  type ChartOptions,
  Tooltip as ChartTooltipPlugin,
  DoughnutController,
  Filler,
  type LegendItem,
  LinearScale,
  LineController,
  LineElement,
  PieController,
  PointElement,
  TimeScale,
  TimeSeriesScale,
  type UpdateMode,
} from 'chart.js';
import { type ReactNode, useEffect, useMemo, useRef, useState } from 'react';
import {
  type AnnotationMarker,
  type ChartAnnotation,
  ChartAnnotationMarkers,
} from '@/components/charts/ChartAnnotationMarkers';
import { Legend } from '@/components/metrics/Legend';
import { getChartBucketIndex } from '@/lib/charts';
import { DEFAULT_ANIMATION_DURATION } from '@/lib/constants';

ChartJS.register(
  ArcElement,
  BarController,
  BarElement,
  BubbleController,
  CategoryScale,
  DoughnutController,
  Filler,
  ChartLegend,
  LinearScale,
  LineController,
  LineElement,
  PieController,
  PointElement,
  TimeScale,
  TimeSeriesScale,
  ChartTooltipPlugin,
);

let cachedFontFamily: string | null = null;

export interface ChartProps extends BoxProps {
  type?: 'bar' | 'bubble' | 'doughnut' | 'pie' | 'line' | 'polarArea' | 'radar' | 'scatter';
  chartData?: ChartData<any, any, unknown> & { focusLabel?: string };
  chartOptions?: ChartOptions;
  updateMode?: UpdateMode;
  animationDuration?: number;
  ariaLabel?: string;
  onTooltip?: (model: any) => void;
  hiddenLabels?: Set<string>;
  onLegendClick?: (label: string, willBeHidden: boolean) => void;
  legendActions?: ReactNode;
  annotations?: ChartAnnotation[];
  onAnnotationClick?: (annotations: ChartAnnotation[]) => void;
  onAnnotationMoreClick?: (annotations: ChartAnnotation[]) => void;
}

function isSameMarkers(a: AnnotationMarker[], b: AnnotationMarker[]) {
  return (
    a.length === b.length &&
    a.every(
      (m, i) =>
        m.x === b[i].x &&
        m.y === b[i].y &&
        m.annotations.length === b[i].annotations.length &&
        m.annotations.every((annotation, j) => annotation === b[i].annotations[j]),
    )
  );
}

export function Chart({
  type,
  chartData,
  animationDuration = DEFAULT_ANIMATION_DURATION,
  ariaLabel,
  updateMode,
  onTooltip,
  chartOptions,
  hiddenLabels,
  onLegendClick,
  legendActions,
  annotations,
  onAnnotationClick,
  onAnnotationMoreClick,
  ...props
}: ChartProps) {
  const canvas = useRef(null);
  const chart = useRef(null);
  const mounted = useRef(false);
  const hadAnnotations = useRef(false);
  const [legendItems, setLegendItems] = useState([]);
  const [markers, setMarkers] = useState<AnnotationMarker[]>([]);
  const annotationsRef = useRef<ChartAnnotation[]>(annotations);
  annotationsRef.current = annotations;
  const onTooltipRef = useRef(onTooltip);
  onTooltipRef.current = onTooltip;

  // Computes pixel positions for annotation markers whenever the chart lays out (update / resize)
  const annotationPlugin = useMemo(
    () => ({
      id: 'annotationMarkers',
      afterLayout: (instance: any) => {
        const list = annotationsRef.current;
        const { chartArea, scales } = instance;

        if (!list?.length || !chartArea || !scales?.x) {
          setMarkers(prev => (prev.length ? [] : prev));
          return;
        }

        // Group annotations that land on the same pixel column into a single marker
        const groups = new Map<number, AnnotationMarker>();

        for (const annotation of list) {
          const markerDate = annotation.markerDate || annotation.date;
          const bucketIndex = getChartBucketIndex(
            instance.data.datasets[0]?.data || [],
            markerDate,
          );
          const bucket = instance.getDatasetMeta(0)?.data[bucketIndex];
          const x = Math.round(bucket?.x ?? scales.x.getPixelForValue(markerDate.getTime()));

          if (!Number.isFinite(x) || x < chartArea.left || x > chartArea.right) {
            continue;
          }

          const group = groups.get(x);

          if (group) {
            group.annotations.push(annotation);
          } else {
            groups.set(x, { annotations: [annotation], x, y: chartArea.bottom });
          }
        }

        const next = [...groups.values()];

        setMarkers(prev => (isSameMarkers(prev, next) ? prev : next));
      },
    }),
    [],
  );

  const options = useMemo(() => {
    return {
      responsive: true,
      maintainAspectRatio: false,
      animation: {
        duration: animationDuration,
        resize: {
          duration: 0,
        },
        active: {
          duration: 0,
        },
      },
      plugins: {
        legend: {
          display: false,
        },
        tooltip: {
          enabled: false,
          intersect: true,
          external: (model: any) => onTooltipRef.current?.(model),
        },
      },
      ...chartOptions,
    };
  }, [animationDuration, chartOptions]);

  const handleLegendClick = (item: LegendItem) => {
    if (onLegendClick && type === 'bar') {
      const { datasetIndex } = item;
      const ds = chart.current.data.datasets[datasetIndex];
      onLegendClick(ds.label, !hiddenLabels?.has(ds.label));
      return;
    }

    if (type === 'bar') {
      const { datasetIndex } = item;
      const meta = chart.current.getDatasetMeta(datasetIndex);

      meta.hidden =
        meta.hidden === null ? !chart.current.data.datasets[datasetIndex]?.hidden : null;
    } else {
      const { index } = item;
      const meta = chart.current.getDatasetMeta(0);
      const hidden = !!meta?.data?.[index]?.hidden;

      meta.data[index].hidden = !hidden;
      chart.current.legend.legendItems[index].hidden = !hidden;
    }

    chart.current.update(updateMode);

    setLegendItems(chart.current.legend.legendItems);
  };

  // Create chart
  useEffect(() => {
    if (canvas.current) {
      if (!cachedFontFamily) {
        cachedFontFamily = getComputedStyle(canvas.current).fontFamily;
      }
      ChartJS.defaults.font.family = cachedFontFamily;
      chart.current = new ChartJS(canvas.current, {
        type,
        data: chartData,
        options,
        plugins: [annotationPlugin],
      });

      setLegendItems(chart.current.legend.legendItems);
      hadAnnotations.current = Boolean(annotationsRef.current?.length);
      const instance = chart.current;
      document.fonts?.ready.then(() => {
        if (instance.canvas) instance.update('none');
      });
    }

    return () => {
      mounted.current = false;
      chart.current?.destroy();
    };
  }, []);

  // Update chart (skip initial mount since new ChartJS already rendered with initial props)
  useEffect(() => {
    if (!mounted.current) {
      mounted.current = true;
      return;
    }

    if (chart.current && chartData) {
      // Replace labels and datasets *in-place*
      chart.current.data.labels = chartData.labels;
      chart.current.data.datasets = chartData.datasets;

      if (chartData.focusLabel !== null) {
        chart.current.data.datasets.forEach((ds: { hidden: boolean; label: any }) => {
          ds.hidden = chartData.focusLabel ? ds.label !== chartData.focusLabel : false;
        });
      }

      if (hiddenLabels) {
        chart.current.data.datasets.forEach((ds: { hidden: boolean; label: any }) => {
          if (hiddenLabels.has(ds.label)) {
            ds.hidden = true;
          } else if (!chartData.focusLabel) {
            ds.hidden = false;
          }
        });
      }

      chart.current.options = options;

      chart.current.update(updateMode);

      setLegendItems(chart.current.legend.legendItems);
    }
  }, [chartData, options, updateMode, hiddenLabels]);

  // Re-run layout only when annotations actually exist or were cleared
  useEffect(() => {
    const hasNow = Boolean(annotations?.length);
    if (!hasNow && !hadAnnotations.current) {
      return;
    }
    hadAnnotations.current = hasNow;
    chart.current?.update('none');
  }, [annotations]);

  return (
    <Column gap="6">
      <Box {...props}>
        <div style={{ position: 'relative', width: '100%', height: '100%', overflow: 'hidden' }}>
          <canvas
            ref={canvas}
            role="img"
            aria-label={ariaLabel || `${type || 'data'} chart`}
            style={{ position: 'absolute', top: 0, left: 0 }}
          />
          {markers.length > 0 && (
            <ChartAnnotationMarkers
              markers={markers}
              onClick={onAnnotationClick}
              onMoreClick={onAnnotationMoreClick}
            />
          )}
        </div>
      </Box>
      {legendActions ? (
        <div
          style={{
            display: 'grid',
            gridTemplateColumns: '1fr auto 1fr',
            alignItems: 'center',
            gap: 8,
          }}
        >
          <div style={{ gridColumn: 2 }}>
            <Legend items={legendItems} onClick={handleLegendClick} />
          </div>
          <div style={{ justifySelf: 'end' }}>{legendActions}</div>
        </div>
      ) : (
        <Legend items={legendItems} onClick={handleLegendClick} />
      )}
    </Column>
  );
}
