import { beforeEach, expect, test, vi } from 'vitest';
import { render } from '@/test/render';
import { PageviewsChart } from './PageviewsChart';

const state = vi.hoisted(() => ({
  theme: 'light' as 'light' | 'dark',
  lastChartData: null as any,
}));

vi.mock('@umami/react-zen', async importOriginal => {
  const actual = await importOriginal<typeof import('@umami/react-zen')>();
  return {
    ...actual,
    useTheme: () => ({ theme: state.theme }),
  };
});

vi.mock('@/components/charts/BarChart', () => ({
  BarChart: ({ chartData }: any) => {
    state.lastChartData = chartData;
    return null;
  },
}));

vi.mock('@/lib/colors', () => ({
  getThemeColors: (theme: string) => ({
    colors: {
      chart: {
        visitors: { backgroundColor: theme === 'dark' ? '#111111' : '#eeeeee' },
        views: { backgroundColor: theme === 'dark' ? '#222222' : '#dddddd' },
      },
    },
  }),
}));

vi.mock('@/components/hooks', async importOriginal => {
  const actual = await importOriginal<typeof import('@/components/hooks')>();
  return {
    ...actual,
    useLocale: () => ({ locale: 'en-US', dateLocale: undefined }),
    useMessages: () => ({
      t: (v: string) => v,
      labels: { visitors: 'Visitors', views: 'Views', previous: 'Previous' },
    }),
  };
});

beforeEach(() => {
  state.theme = 'light';
  state.lastChartData = null;
});

test('recomputes dataset colors when theme changes without refetching data', () => {
  const data = {
    pageviews: [{ x: '2026-11-20', y: 10 }],
    sessions: [{ x: '2026-11-20', y: 5 }],
  };
  const minDate = new Date('2026-11-20T00:00:00Z');
  const maxDate = new Date('2026-11-21T00:00:00Z');

  const { rerender } = render(
    <PageviewsChart data={data} unit="day" minDate={minDate} maxDate={maxDate} />,
  );
  const lightColor = state.lastChartData.datasets[0].backgroundColor;

  state.theme = 'dark';
  rerender(<PageviewsChart data={data} unit="day" minDate={minDate} maxDate={maxDate} />);
  const darkColor = state.lastChartData.datasets[0].backgroundColor;

  expect(lightColor).toBeTruthy();
  expect(darkColor).toBeTruthy();
  expect(darkColor).not.toBe(lightColor);
});
