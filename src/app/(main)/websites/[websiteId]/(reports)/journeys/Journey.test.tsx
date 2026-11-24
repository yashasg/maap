import { expect, test, vi } from 'vitest';
import { render } from '@/test/render';
import { Journey } from './Journey';

const mockData = [
  { items: ['/contact', '/thank-you'], count: 4 },
  { items: ['/', '/pricing'], count: 100 },
  { items: ['/about', '/team'], count: 25 },
  { items: ['/blog', '/pricing'], count: 50 },
  { items: ['/beta', '/next'], count: 10 },
  { items: ['/alpha', '/next'], count: 10 },
];

vi.mock('@/components/hooks', async importOriginal => {
  const actual = await importOriginal<typeof import('@/components/hooks')>();

  return {
    ...actual,
    useJourneyQuery: () => ({
      data: mockData,
      isLoading: false,
      error: null,
    }),
    useMessages: () => ({
      t: (value: string) => value,
      labels: {
        step: 'Step',
        dropoff: 'Dropoff',
        conversion: 'Conversion',
      },
    }),
    useEscapeKey: vi.fn(),
  };
});

test('sorts nodes in each step column by total visitor count descending with alphabetical tie-breaking', () => {
  const { container } = render(
    <Journey
      websiteId="website-1"
      steps={2}
      startDate={new Date('2026-07-01T00:00:00.000Z')}
      endDate={new Date('2026-07-31T23:59:59.999Z')}
      view="views"
    />,
  );

  const columns = container.querySelectorAll('[class*="column"]');
  expect(columns.length).toBeGreaterThanOrEqual(2);

  // Column 1 (Step 1)
  const step1NodesContainer = columns[0].querySelector('[class*="nodes"]');
  const step1NodeNames = Array.from(
    step1NodesContainer?.querySelectorAll('[class*="name"]') || [],
  ).map(el => el.getAttribute('title'));

  expect(step1NodeNames).toEqual([
    '/', // 100
    '/blog', // 50
    '/about', // 25
    '/alpha', // 10 (tied with /beta, sorted alphabetically)
    '/beta', // 10 (tied with /alpha, sorted alphabetically)
    '/contact', // 4
  ]);

  // Column 2 (Step 2)
  const step2NodesContainer = columns[1].querySelector('[class*="nodes"]');
  const step2NodeNames = Array.from(
    step2NodesContainer?.querySelectorAll('[class*="name"]') || [],
  ).map(el => el.getAttribute('title'));

  expect(step2NodeNames).toEqual([
    '/pricing', // 100 + 50 = 150
    '/team', // 25
    '/next', // 10 + 10 = 20
    '/thank-you', // 4
  ]);
});

test('selecting and hovering a node highlights flow counts without reordering columns', async () => {
  const { container, user } = render(
    <Journey
      websiteId="website-1"
      steps={2}
      startDate={new Date('2026-07-01T00:00:00.000Z')}
      endDate={new Date('2026-07-31T23:59:59.999Z')}
      view="views"
    />,
  );

  const rootNode = container.querySelector('[title="/"]') as HTMLElement;
  await user.click(rootNode);
  await user.hover(rootNode);

  const step2Nodes = Array.from(
    container.querySelectorAll('[class*="column"]')[1]?.querySelectorAll('[class*="name"]') || [],
  ).map(el => el.getAttribute('title'));

  expect(step2Nodes).toEqual(['/pricing', '/team', '/next', '/thank-you']);
  expect(container.querySelector('[title="100"]')).toBeInTheDocument();
});
