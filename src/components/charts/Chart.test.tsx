import { beforeEach, expect, test, vi } from 'vitest';
import { render, screen } from '@/test/render';
import { Chart } from './Chart';
import { PieChart } from './PieChart';

const chartSpies = vi.hoisted(() => ({
  construct: vi.fn(),
  update: vi.fn(),
  destroy: vi.fn(),
}));

vi.mock('chart.js', () => {
  class MockChart {
    static register = vi.fn();
    static defaults = { font: { family: '' } };
    canvas: HTMLCanvasElement;
    data: any;
    options: any;
    legend = { legendItems: [] };
    constructor(canvas: HTMLCanvasElement, config: any) {
      this.canvas = canvas;
      this.data = config.data;
      this.options = config.options;
      chartSpies.construct(config);
    }
    update = chartSpies.update;
    destroy = chartSpies.destroy;
  }

  return {
    Chart: MockChart,
    ArcElement: {},
    BarController: {},
    BarElement: {},
    BubbleController: {},
    CategoryScale: {},
    DoughnutController: {},
    Filler: {},
    Legend: {},
    LinearScale: {},
    LineController: {},
    LineElement: {},
    PieController: {},
    PointElement: {},
    TimeScale: {},
    TimeSeriesScale: {},
    Tooltip: {},
  };
});

beforeEach(() => {
  chartSpies.construct.mockClear();
  chartSpies.update.mockClear();
  chartSpies.destroy.mockClear();
});

test('avoids redundant chart.update calls on initial mount and updates when animationDuration changes', () => {
  const data = { labels: ['A'], datasets: [{ label: 'Views', data: [1] }] };
  const { rerender } = render(
    <Chart type="bar" chartData={data} animationDuration={300} ariaLabel="Pageviews chart" />,
  );

  expect(screen.getByRole('img', { name: 'Pageviews chart' })).toBeInTheDocument();
  expect(chartSpies.construct).toHaveBeenCalledTimes(1);
  expect(chartSpies.update).not.toHaveBeenCalled();

  rerender(<Chart type="bar" chartData={data} animationDuration={0} ariaLabel="Pageviews chart" />);

  expect(chartSpies.update).toHaveBeenCalledTimes(1);
});

test('PieChart isolates tooltip state from Chart.js updates on hover', () => {
  const data = {
    labels: ['iOS', 'macOS'],
    datasets: [{ data: [10, 5], backgroundColor: ['#111', '#222'] }],
  };

  render(<PieChart type="doughnut" chartData={data} />);

  expect(chartSpies.construct).toHaveBeenCalledTimes(1);
  const external = chartSpies.construct.mock.calls[0][0].options.plugins.tooltip.external;

  external({
    tooltip: {
      opacity: 1,
      title: ['iOS'],
      labelColors: [{ backgroundColor: '#111' }],
      dataPoints: [{ raw: 10 }],
    },
  });
  external({
    tooltip: {
      opacity: 1,
      title: ['iOS'],
      labelColors: [{ backgroundColor: '#111' }],
      dataPoints: [{ raw: 10 }],
    },
  });

  expect(chartSpies.update).not.toHaveBeenCalled();
});
