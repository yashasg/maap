import { expect, test, vi } from 'vitest';
import { render, screen } from '@/test/render';
import { WorkspacePoll } from './WorkspacePoll';

const mocks = vi.hoisted(() => ({
  get: vi.fn(),
  post: vi.fn(),
  mutateAsync: vi.fn(),
  refetch: vi.fn(),
  reset: vi.fn(),
  useMutation: vi.fn(),
  useQuery: vi.fn(),
}));

vi.mock('@/components/hooks/useApi', () => ({
  useApi: () => ({
    get: mocks.get,
    post: mocks.post,
    useMutation: mocks.useMutation,
    useQuery: mocks.useQuery,
  }),
}));

function mockMutation(overrides: Record<string, unknown> = {}) {
  mocks.useMutation.mockReturnValue({
    mutateAsync: mocks.mutateAsync,
    isPending: false,
    error: null,
    reset: mocks.reset,
    ...overrides,
  });
}

test('shows and updates the authenticated account choice', async () => {
  mocks.useQuery.mockReturnValue({
    data: { choice: 'community' },
    isLoading: false,
    isFetching: false,
    error: null,
    refetch: mocks.refetch,
  });
  mockMutation();
  mocks.mutateAsync.mockResolvedValue({ choice: 'content' });

  const { user } = render(<WorkspacePoll />);

  expect(screen.getByText('Saved answer: A developer community')).toBeInTheDocument();
  await user.click(screen.getByText('What brought you to Maap?'));
  await user.click(screen.getByRole('radio', { name: 'An article, video, or podcast' }));
  await user.click(screen.getByRole('button', { name: 'Update answer' }));

  expect(mocks.mutateAsync).toHaveBeenCalledWith('content');
  expect(await screen.findByRole('status')).toHaveTextContent('Saved on this Maap instance.');
  expect(screen.getByRole('button', { name: 'Update answer' })).toBeDisabled();
});

test('provides loading and retry feedback', async () => {
  mocks.useQuery.mockReturnValue({
    data: undefined,
    isLoading: false,
    isFetching: false,
    error: new Error('offline'),
    refetch: mocks.refetch,
  });
  mockMutation();

  const { user } = render(<WorkspacePoll />);

  expect(screen.getByText('Your saved answer could not be loaded.')).toBeInTheDocument();
  await user.click(screen.getByText('What brought you to Maap?'));
  expect(screen.getByRole('alert')).toHaveTextContent(
    'We couldn’t load your saved answer. Your existing choice was not changed.',
  );
  await user.click(screen.getByRole('button', { name: 'Try again' }));
  expect(mocks.refetch).toHaveBeenCalledOnce();
});

test('announces a real loading state', async () => {
  mocks.useQuery.mockReturnValue({
    data: undefined,
    isLoading: true,
    isFetching: true,
    error: null,
    refetch: mocks.refetch,
  });
  mockMutation();

  const { user } = render(<WorkspacePoll />);
  await user.click(screen.getByText('What brought you to Maap?'));

  expect(screen.getByRole('status')).toHaveTextContent('Loading your saved answer…');
});
