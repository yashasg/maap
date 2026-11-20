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

test('GET returns app defaults and read-only capability when no settings row exists', async () => {
  parseRequestMock.mockResolvedValue({
    auth: { user: { id: 'member-1' } },
    error: undefined,
  } as any);
  canViewWebsiteMock.mockResolvedValue(true);
  canUpdateWebsiteMock.mockResolvedValue(false);
  queryMock.mockResolvedValue([] as any);

  const response = await GET(new Request('http://localhost'), {
    params: Promise.resolve({ websiteId: 'app-1' }),
  });

  expect(queryMock).toHaveBeenCalledWith(
    expect.stringMatching(/WHERE website_id = \$1[\s\S]*deleted_at IS NULL/),
    'app-1',
  );
  await expect(response.json()).resolves.toEqual({
    websiteId: 'app-1',
    overviewLayout: 'application',
    colorScheme: 'default',
    revenueDisplayMode: 'none',
    canWrite: false,
  });
});

test('POST denies a read-only member before the upsert', async () => {
  parseRequestMock.mockResolvedValue({
    auth: { user: { id: 'member-1' } },
    body: {
      overviewLayout: 'application',
      colorScheme: 'vibrant',
      revenueDisplayMode: 'none',
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

test('POST upserts one complete validated settings record for the requested app', async () => {
  const body = {
    overviewLayout: 'website',
    colorScheme: 'high-contrast',
    revenueDisplayMode: 'revenuecat',
  } as const;
  parseRequestMock.mockResolvedValue({
    auth: { user: { id: 'owner-1' } },
    body,
    error: undefined,
  } as any);
  canUpdateWebsiteMock.mockResolvedValue(true);
  queryMock.mockResolvedValue([{ websiteId: 'app-1', ...body }] as any);

  const response = await POST(new Request('http://localhost', { method: 'POST' }), {
    params: Promise.resolve({ websiteId: 'app-1' }),
  });

  expect(queryMock).toHaveBeenCalledWith(
    expect.stringMatching(/deleted_at IS NULL[\s\S]*FOR UPDATE[\s\S]*ON CONFLICT/),
    'app-1',
    'website',
    'high-contrast',
    'revenuecat',
  );
  await expect(response.json()).resolves.toEqual({
    websiteId: 'app-1',
    ...body,
    canWrite: true,
  });
});

test('POST refuses a write when the app is deleted after the permission check', async () => {
  parseRequestMock.mockResolvedValue({
    auth: { user: { id: 'owner-1' } },
    body: {
      overviewLayout: 'application',
      colorScheme: 'default',
      revenueDisplayMode: 'none',
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
