import { expect, test } from 'vitest';
import { render, screen, waitFor } from '@/test/render';
import { ListTable } from './ListTable';
import { MetricCard } from './MetricCard';

test('renders real numbers on mount and remount, then animates subsequent updates', async () => {
  const data = [{ label: 'Checkout', count: 80, percent: 40 }];
  const { rerender } = render(<ListTable data={data} />);
  expect(screen.getByTitle('80')).toHaveTextContent('80');
  expect(screen.getByText('40%')).toBeVisible();

  rerender(<ListTable data={[{ ...data[0], count: 120, percent: 60 }]} />);
  // Existing rows update from their previous value, never from zero.
  expect(Number(screen.getByTitle('120').textContent)).toBeGreaterThanOrEqual(80);
  await waitFor(() => expect(screen.getByTitle('120')).toHaveTextContent('120'));
  await waitFor(() => expect(screen.getByText('60%')).toBeVisible());

  rerender(<ListTable key="remount" data={data} />);
  expect(screen.getByTitle('80')).toHaveTextContent('80');
  expect(screen.getByText('40%')).toBeVisible();
});

test('overview metric cards also start at their actual values and change percentages', () => {
  const { rerender } = render(<MetricCard value={80} change={40} showChange />);
  expect(screen.getByTitle('80')).toHaveTextContent('80');
  expect(screen.getByText('100%')).toBeVisible();
  rerender(<MetricCard key="remount" value={80} change={40} showChange />);
  expect(screen.getByTitle('80')).toHaveTextContent('80');
  expect(screen.getByText('100%')).toBeVisible();
});
