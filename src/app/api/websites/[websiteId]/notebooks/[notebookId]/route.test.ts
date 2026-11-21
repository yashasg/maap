import { beforeEach, expect, test, vi } from 'vitest';
import prisma from '@/lib/prisma';
import { parseRequest } from '@/lib/request';
import { canUpdateWebsite, canViewWebsite } from '@/permissions';
import { GET, POST } from './route';

vi.mock('@/lib/prisma', () => {
  const client = {
    $queryRawUnsafe: vi.fn(),
  };
  return {
    default: { client },
    getRawQueryClient: () => client,
  };
});

vi.mock('@/lib/request', () => ({
  parseRequest: vi.fn(),
}));

vi.mock('@/permissions', () => ({
  canUpdateWebsite: vi.fn(),
  canViewWebsite: vi.fn(),
}));

const queryMock = vi.mocked(prisma.client.$queryRawUnsafe);

beforeEach(() => {
  vi.mocked(parseRequest).mockReset();
  vi.mocked(canUpdateWebsite).mockReset();
  vi.mocked(canViewWebsite).mockReset();
  queryMock.mockReset();
});

test('GET requires a live app in addition to app-scoped notebook ids', async () => {
  vi.mocked(parseRequest).mockResolvedValue({
    auth: { user: { id: 'user-1' } },
    error: undefined,
  } as any);
  vi.mocked(canViewWebsite).mockResolvedValue(true);
  queryMock.mockResolvedValue([{ id: 'notebook-1', websiteId: 'app-1', title: 'Notes' }] as any);
  vi.mocked(canUpdateWebsite).mockResolvedValue(false);

  const response = await GET(new Request('http://localhost'), {
    params: Promise.resolve({ websiteId: 'app-1', notebookId: 'notebook-1' }),
  });

  expect(queryMock).toHaveBeenCalledWith(
    expect.stringMatching(/website_id = \$1[\s\S]*notebook_id = \$2[\s\S]*deleted_at IS NULL/),
    'app-1',
    'notebook-1',
  );
  await expect(response.json()).resolves.toEqual({
    data: { id: 'notebook-1', websiteId: 'app-1', title: 'Notes' },
    canWrite: false,
  });
});

test('POST cannot update a notebook through another app URL', async () => {
  vi.mocked(parseRequest).mockResolvedValue({
    auth: { user: { id: 'user-1' } },
    body: {
      title: 'Scoped notes',
      content: {
        version: 1,
        blocks: [{ id: 'one', type: 'paragraph', text: 'Private to this app' }],
      },
    },
    error: undefined,
  } as any);
  vi.mocked(canUpdateWebsite).mockResolvedValue(true);
  queryMock.mockResolvedValue([] as any);

  const response = await POST(new Request('http://localhost', { method: 'POST' }), {
    params: Promise.resolve({ websiteId: 'other-app', notebookId: 'notebook-1' }),
  });

  expect(queryMock).toHaveBeenCalledWith(
    expect.stringMatching(
      /deleted_at IS NULL[\s\S]*FOR UPDATE[\s\S]*website_id = \$1[\s\S]*notebook_id = \$2/,
    ),
    'other-app',
    'notebook-1',
    'Scoped notes',
    expect.any(String),
  );
  expect(response.status).toBe(404);
});
