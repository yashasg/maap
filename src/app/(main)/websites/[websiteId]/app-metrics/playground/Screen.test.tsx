import { fireEvent } from '@testing-library/react';
import { beforeEach, expect, test, vi } from 'vitest';
import { render, screen, waitFor, within } from '@/test/render';
import PlaygroundScreen from './Screen';

const mocks = vi.hoisted(() => ({
  get: vi.fn(),
}));

vi.mock('@/components/hooks/useApi', () => ({
  useApi: () => ({ get: mocks.get }),
}));

vi.mock('@/components/hooks/useDateParameters', () => ({
  useDateParameters: () => ({
    startAt: 1_795_564_800_000,
    endAt: 1_798_156_799_999,
    timezone: 'America/Los_Angeles',
    unit: 'day',
  }),
}));

vi.mock('@/components/hooks/useFilterParameters', () => ({
  useFilterParameters: () => ({ tag: 'test' }),
}));

beforeEach(() => {
  mocks.get.mockReset();
});

test('runs an authorized traffic query and renders the real API response', async () => {
  mocks.get.mockResolvedValue({
    pageviews: [{ x: '2026-11-01T00:00:00.000Z', y: 12 }],
    sessions: [{ x: '2026-11-01T00:00:00.000Z', y: 3 }],
  });
  const { user } = render(<PlaygroundScreen websiteId="website-1" />);

  await user.click(screen.getByRole('button', { name: 'Run query' }));

  await waitFor(() =>
    expect(mocks.get).toHaveBeenCalledWith('/websites/website-1/pageviews', {
      startAt: 1_795_564_800_000,
      endAt: 1_798_156_799_999,
      timezone: 'America/Los_Angeles',
      unit: 'day',
      tag: 'test',
    }),
  );
  const table = await screen.findByRole('table');
  expect(within(table).getByText('12')).toBeInTheDocument();
  expect(within(table).getByText('3')).toBeInTheDocument();
  expect(within(table).getByText('4')).toBeInTheDocument();
  expect(screen.getByText('1 result row')).toBeInTheDocument();
});

test('rejects unsupported JSON input without sending a request', async () => {
  const { user } = render(<PlaygroundScreen websiteId="website-1" />);

  await user.click(screen.getByRole('tab', { name: 'JSON editor' }));
  fireEvent.change(screen.getByLabelText('Structured query'), {
    target: {
      value:
        '{"queryType":"traffic-timeseries","granularity":"day","sql":"select * from website_event"}',
    },
  });
  await user.click(screen.getByRole('button', { name: 'Run query' }));

  expect(screen.getByRole('alert')).toHaveTextContent('Unsupported option: sql');
  expect(mocks.get).not.toHaveBeenCalled();
});
