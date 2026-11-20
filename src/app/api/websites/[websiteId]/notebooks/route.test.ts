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

const parseRequestMock = vi.mocked(parseRequest);
const canViewWebsiteMock = vi.mocked(canViewWebsite);
const canUpdateWebsiteMock = vi.mocked(canUpdateWebsite);
const queryMock = vi.mocked(prisma.client.$queryRawUnsafe);

beforeEach(() => {
  parseRequestMock.mockReset();
  canViewWebsiteMock.mockReset();
  canUpdateWebsiteMock.mockReset();
  queryMock.mockReset();
});

test('GET scopes notebook rows to the requested app and reports write permission', async () => {
  parseRequestMock.mockResolvedValue({
    auth: { user: { id: 'user-1' } },
    error: undefined,
  } as any);
  canViewWebsiteMock.mockResolvedValue(true);
  canUpdateWebsiteMock.mockResolvedValue(false);
  queryMock.mockResolvedValue([{ id: 'notebook-1', websiteId: 'app-1' }] as any);

  const response = await GET(new Request('http://localhost'), {
    params: Promise.resolve({ websiteId: 'app-1' }),
  });

  expect(queryMock).toHaveBeenCalledWith(
    expect.stringMatching(/WHERE website_id = \$1[\s\S]*deleted_at IS NULL/),
    'app-1',
  );
  await expect(response.json()).resolves.toEqual({
    data: [{ id: 'notebook-1', websiteId: 'app-1' }],
    canWrite: false,
  });
});

test('POST denies a read-only user before writing', async () => {
  parseRequestMock.mockResolvedValue({
    auth: { user: { id: 'user-1' } },
    body: {
      title: 'Notes',
      content: {
        version: 1,
        blocks: [{ id: 'one', type: 'paragraph', text: '' }],
      },
    },
    error: undefined,
  } as any);
  canUpdateWebsiteMock.mockResolvedValue(false);

  const response = await POST(new Request('http://localhost', { method: 'POST' }), {
    params: Promise.resolve({ websiteId: 'app-1' }),
  });

  expect(response.status).toBe(401);
  expect(queryMock).not.toHaveBeenCalled();
});

test('POST locks and rechecks the live app before inserting notebook content', async () => {
  parseRequestMock.mockResolvedValue({
    auth: { user: { id: 'user-1' } },
    body: {
      title: 'Notes',
      content: {
        version: 1,
        blocks: [{ id: 'one', type: 'paragraph', text: 'Persisted' }],
      },
    },
    error: undefined,
  } as any);
  canUpdateWebsiteMock.mockResolvedValue(true);
  queryMock.mockResolvedValue([{ id: 'notebook-1', websiteId: 'app-1' }] as any);

  const response = await POST(new Request('http://localhost', { method: 'POST' }), {
    params: Promise.resolve({ websiteId: 'app-1' }),
  });

  expect(queryMock).toHaveBeenCalledWith(
    expect.stringMatching(/deleted_at IS NULL[\s\S]*FOR UPDATE[\s\S]*INSERT INTO notebook/),
    expect.any(String),
    'app-1',
    'user-1',
    'Notes',
    expect.any(String),
  );
  expect(response.status).toBe(200);
});

test('POST refuses an insert when the app is deleted after the permission check', async () => {
  parseRequestMock.mockResolvedValue({
    auth: { user: { id: 'user-1' } },
    body: {
      title: 'Notes',
      content: {
        version: 1,
        blocks: [{ id: 'one', type: 'paragraph', text: '' }],
      },
    },
    error: undefined,
  } as any);
  canUpdateWebsiteMock.mockResolvedValue(true);
  queryMock.mockResolvedValue([] as any);

  const response = await POST(new Request('http://localhost', { method: 'POST' }), {
    params: Promise.resolve({ websiteId: 'app-1' }),
  });

  expect(response.status).toBe(401);
});
