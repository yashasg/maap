import { useMutation, useQuery } from '@tanstack/react-query';
import { fireEvent } from '@testing-library/react';
import { beforeEach, expect, test, vi } from 'vitest';
import { render, screen, waitFor } from '@/test/render';
import NotebooksScreen from './Screen';

const mocks = vi.hoisted(() => ({
  get: vi.fn(),
  post: vi.fn(),
  replace: vi.fn(),
  query: {} as Record<string, string>,
}));

vi.mock('@/components/hooks/useApi', () => ({
  useApi: () => ({
    get: mocks.get,
    post: mocks.post,
    useMutation,
    useQuery,
  }),
}));

vi.mock('@/components/hooks/useNavigation', () => ({
  useNavigation: () => ({
    query: mocks.query,
    router: { replace: mocks.replace },
    updateParams: (values: Record<string, string | undefined>) => {
      const params = new URLSearchParams({ range: '30day', timezone: 'UTC', ...mocks.query });
      for (const [key, value] of Object.entries(values)) {
        if (value) params.set(key, value);
        else params.delete(key);
      }
      return `/websites/app-1/app-reports/notebooks?${params}`;
    },
  }),
}));

const notebook = {
  id: 'notebook-1',
  websiteId: 'app-1',
  createdBy: 'user-1',
  title: 'Release review',
  content: {
    version: 1 as const,
    blocks: [{ id: 'block-1', type: 'paragraph' as const, text: 'Check upgrade adoption.' }],
  },
  createdAt: '2026-11-20T10:00:00.000Z',
  updatedAt: '2026-11-20T10:00:00.000Z',
};

beforeEach(() => {
  mocks.get.mockReset();
  mocks.post.mockReset();
  mocks.replace.mockReset();
  mocks.query = {};
});

test('creates a persisted app-scoped notebook and preserves report controls', async () => {
  mocks.get.mockResolvedValue({ data: [], canWrite: true });
  mocks.post.mockResolvedValue(notebook);
  const { user } = render(<NotebooksScreen websiteId="app-1" />);

  await user.click(await screen.findByRole('button', { name: 'Create New' }));

  expect(mocks.post).toHaveBeenCalledWith(
    '/websites/app-1/notebooks',
    expect.objectContaining({
      title: 'Untitled notebook',
      content: expect.objectContaining({ version: 1 }),
    }),
  );
  await waitFor(() =>
    expect(mocks.replace).toHaveBeenCalledWith(
      expect.stringContaining('range=30day&timezone=UTC&notebook=notebook-1'),
      { scroll: false },
    ),
  );
});

test('reopens, edits, and saves the selected notebook in the same app scope', async () => {
  mocks.query = { notebook: 'notebook-1' };
  mocks.get.mockImplementation((url: string) => {
    if (url.endsWith('/notebook-1')) return Promise.resolve({ data: notebook, canWrite: true });
    return Promise.resolve({ data: [notebook], canWrite: true });
  });
  mocks.post.mockImplementation((_url: string, body: any) =>
    Promise.resolve({ ...notebook, ...body, updatedAt: '2026-11-20T11:00:00.000Z' }),
  );
  const { user } = render(<NotebooksScreen websiteId="app-1" />);

  const title = await screen.findByLabelText('Notebook title');
  expect(title).toHaveValue('Release review');
  expect(screen.getByLabelText('Text block 1')).toHaveValue('Check upgrade adoption.');

  await user.clear(title);
  await user.type(title, 'Launch notes');
  fireEvent.change(screen.getByLabelText('Text block 1'), {
    target: { value: 'Confirm version 2 rollout.' },
  });
  await user.click(screen.getByRole('button', { name: 'Save notebook' }));

  await waitFor(() =>
    expect(mocks.post).toHaveBeenCalledWith('/websites/app-1/notebooks/notebook-1', {
      title: 'Launch notes',
      content: {
        version: 1,
        blocks: [
          {
            id: 'block-1',
            type: 'paragraph',
            text: 'Confirm version 2 rollout.',
          },
        ],
      },
    }),
  );
  expect(await screen.findByText('Saved')).toBeInTheDocument();
});

test('keeps read-only users from creating or changing notebooks', async () => {
  mocks.get.mockResolvedValue({ data: [], canWrite: false });
  render(<NotebooksScreen websiteId="app-1" />);

  expect(await screen.findByRole('button', { name: 'Create New' })).toBeDisabled();
  expect(await screen.findByRole('button', { name: 'Create your first notebook' })).toBeDisabled();
  expect(mocks.post).not.toHaveBeenCalled();
});
