import { expect, test, vi } from 'vitest';
import { render } from '@/test/render';
import { WebsiteSparkline } from './WebsiteSparkline';

vi.mock('@/components/hooks', async importOriginal => {
  const actual = await importOriginal<typeof import('@/components/hooks')>();
  return {
    ...actual,
    useMessages: () => ({
      t: (v: string) => v,
      labels: { visitors: 'Visitors' },
    }),
  };
});

test('uses non-scaling-stroke and draws a visible horizontal line for a single value', () => {
  const { container } = render(<WebsiteSparkline values={[42]} total={42} />);
  const polyline = container.querySelector('polyline');

  expect(polyline).toHaveAttribute('vector-effect', 'non-scaling-stroke');
  const points = polyline?.getAttribute('points')?.trim().split(/\s+/) ?? [];
  expect(points).toHaveLength(2);
  expect(points[0]).toMatch(/^0,/);
  expect(points[1]).toMatch(/^96,/);
});
