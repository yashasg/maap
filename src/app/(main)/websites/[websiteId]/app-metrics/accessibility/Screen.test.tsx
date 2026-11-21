import { beforeEach, expect, test, vi } from 'vitest';
import { render, screen } from '@/test/render';
import AccessibilityScreen from './Screen';

const mocks = vi.hoisted(() => ({
  path: '',
  websiteId: '',
  refetch: vi.fn(),
}));

vi.mock('@/components/hooks/queries/useAnalyticsQuery', () => ({
  useAnalyticsQuery: (path: string, params: { websiteId: string }) => {
    mocks.path = path;
    mocks.websiteId = params.websiteId;
    return {
      data: {
        reports: [
          {
            id: 'preferredContentSize',
            totalSignals: 10,
            reportedSignals: 5,
            values: [
              { label: 'Large', signals: 5 },
              { label: 'Not reported', signals: 5 },
            ],
          },
          ...[
            'boldText',
            'reduceMotion',
            'reduceTransparency',
            'darkerSystemColors',
            'differentiateWithoutColor',
            'invertedColors',
          ].map(id => ({
            id,
            totalSignals: 10,
            reportedSignals: 5,
            values: [
              { label: 'On', signals: 2 },
              { label: 'Off', signals: 3 },
              { label: 'Not reported', signals: 5 },
            ],
          })),
        ],
      },
      error: null,
      isLoading: false,
      isFetching: false,
      refetch: mocks.refetch,
    };
  },
}));

vi.mock('@/components/charts/PieChart', () => ({
  PieChart: () => <div data-testid="aggregate-chart" />,
}));

beforeEach(() => {
  mocks.path = '';
  mocks.websiteId = '';
  mocks.refetch.mockClear();
});

test('renders all seven aggregate reports in reference order', () => {
  render(<AccessibilityScreen websiteId="website-1" />);

  expect(mocks.path).toBe('app-reports/accessibility');
  expect(mocks.websiteId).toBe('website-1');
  expect(screen.getAllByRole('heading').map(heading => heading.textContent)).toEqual([
    'Accessibility',
    'Preferred Content Size (Dynamic Type)',
    'Bold Text',
    'Reduce Motion',
    'Reduce Transparency',
    'Darker System Colors',
    'Differentiate Without Color',
    'Inverted Colors',
  ]);
});

test('keeps off and absent settings visibly distinct without user-level rows', () => {
  render(<AccessibilityScreen websiteId="website-1" />);

  const boldText = screen.getByRole('list', { name: 'Bold Text breakdown' });
  expect(boldText).toHaveTextContent('On');
  expect(boldText).toHaveTextContent('Off');
  expect(boldText).toHaveTextContent('Not reported');
  expect(screen.getAllByText('5 of 10 signals reported this setting')).toHaveLength(7);
  expect(
    screen.getByText(/These are aggregate signal counts, never individual user records/),
  ).toBeInTheDocument();
});

test('refreshes the app-scoped accessibility query', async () => {
  const { user } = render(<AccessibilityScreen websiteId="website-1" />);

  await user.click(screen.getByRole('button', { name: 'Refresh accessibility reports' }));

  expect(mocks.refetch).toHaveBeenCalledTimes(1);
});
