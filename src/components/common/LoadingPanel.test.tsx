import { expect, test } from 'vitest';
import { render, screen } from '@/test/render';
import { LoadingPanel } from './LoadingPanel';

test('keeps loaded content mounted and focused during a refresh', async () => {
  const { rerender, user } = render(
    <LoadingPanel data={[1]}>
      <input aria-label="Table filter" defaultValue="checkout" />
    </LoadingPanel>,
  );
  const input = screen.getByRole('textbox');
  await user.click(input);
  rerender(
    <LoadingPanel data={[1]} isFetching>
      <input aria-label="Table filter" defaultValue="checkout" />
    </LoadingPanel>,
  );
  expect(screen.getByRole('textbox')).toBe(input);
  expect(input).toHaveFocus();
  expect(input).toHaveValue('checkout');
  expect(screen.getByRole('status', { name: 'Loading' })).toBeVisible();

  rerender(
    <LoadingPanel data={[2]}>
      <input aria-label="Table filter" defaultValue="new data" />
    </LoadingPanel>,
  );
  expect(screen.getByRole('textbox')).toBe(input);
  expect(screen.queryByRole('status', { name: 'Loading' })).not.toBeInTheDocument();
});

test('distinguishes first load, refreshing empty results, and errors', () => {
  const empty = () => <p>No matching events</p>;
  const { rerender } = render(
    <LoadingPanel data={[]} isLoading isFetching renderEmpty={empty}>
      Table
    </LoadingPanel>,
  );
  expect(screen.queryByText('No matching events')).not.toBeInTheDocument();
  expect(screen.getByRole('status', { name: 'Loading' })).toBeVisible();

  rerender(
    <LoadingPanel data={[]} isFetching renderEmpty={empty}>
      Table
    </LoadingPanel>,
  );
  expect(screen.getByText('No matching events')).toBeVisible();

  rerender(
    <LoadingPanel data={[1]} error={new Error('Refresh failed')}>
      Stale table
    </LoadingPanel>,
  );
  expect(screen.queryByText('Stale table')).not.toBeInTheDocument();
  expect(screen.getByText('Something went wrong.')).toBeVisible();
});

test('shows a content skeleton only on first load, not over retained data or error states', () => {
  const props = {
    loadingContent: (
      <div role="status" aria-label="Loading rows">
        Row placeholders
      </div>
    ),
  };
  const { rerender } = render(
    <LoadingPanel {...props} isLoading>
      Results
    </LoadingPanel>,
  );
  expect(screen.getByRole('status', { name: 'Loading rows' })).toBeVisible();
  expect(screen.queryByText('Results')).not.toBeInTheDocument();
  expect(screen.getAllByRole('status')).toHaveLength(1);

  rerender(
    <LoadingPanel {...props} data={[1]} isFetching>
      Results
    </LoadingPanel>,
  );
  expect(screen.getByText('Results')).toBeVisible();
  expect(screen.queryByText('Row placeholders')).not.toBeInTheDocument();

  rerender(
    <LoadingPanel {...props} error={new Error('Failed')}>
      Results
    </LoadingPanel>,
  );
  expect(screen.getByText('Something went wrong.')).toBeVisible();
  expect(screen.queryByText('Row placeholders')).not.toBeInTheDocument();
});
