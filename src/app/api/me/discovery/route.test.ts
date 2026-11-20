import { beforeEach, expect, test, vi } from 'vitest';
import { parseRequest } from '@/lib/request';
import { GET, POST } from './route';

const mocks = vi.hoisted(() => ({
  findUnique: vi.fn(),
  upsert: vi.fn(),
}));

vi.mock('@/lib/request', () => ({
  parseRequest: vi.fn(),
}));

vi.mock('@/lib/prisma', () => ({
  default: {
    client: {
      appSetting: {
        findUnique: mocks.findUnique,
        upsert: mocks.upsert,
      },
    },
  },
}));

const parseRequestMock = vi.mocked(parseRequest);

beforeEach(() => {
  parseRequestMock.mockReset();
  mocks.findUnique.mockReset();
  mocks.upsert.mockReset();
  parseRequestMock.mockResolvedValue({
    auth: { user: { id: 'user-1' } },
    body: { choice: 'community' },
    error: undefined,
  });
});

test('GET returns the saved choice for the authenticated account', async () => {
  mocks.findUnique.mockResolvedValue({
    key: 'workspaceDiscovery:user-1',
    value: 'recommendation',
  });

  const response = await GET(new Request('http://localhost/api/me/discovery'));

  expect(mocks.findUnique).toHaveBeenCalledWith({
    where: { key: 'workspaceDiscovery:user-1' },
  });
  expect(response.headers.get('Cache-Control')).toBe('private, no-store, max-age=0');
  await expect(response.json()).resolves.toEqual({ choice: 'recommendation' });
});

test('GET returns a null choice when the account has not answered', async () => {
  mocks.findUnique.mockResolvedValue(null);

  const response = await GET(new Request('http://localhost/api/me/discovery'));

  await expect(response.json()).resolves.toEqual({ choice: null });
});

test('POST accepts only finite choices and uses a server-generated setting key', async () => {
  mocks.upsert.mockResolvedValue({
    key: 'workspaceDiscovery:user-1',
    value: 'community',
  });

  const response = await POST(new Request('http://localhost/api/me/discovery', { method: 'POST' }));
  const schema = parseRequestMock.mock.calls[0][1];

  expect(schema.safeParse({ choice: 'search' }).success).toBe(true);
  expect(schema.safeParse({ choice: 'custom answer' }).success).toBe(false);
  expect(schema.safeParse({ choice: 'search', key: 'twoFactorRequiredGlobal' }).success).toBe(
    false,
  );
  expect(mocks.upsert).toHaveBeenCalledWith({
    where: { key: 'workspaceDiscovery:user-1' },
    update: { value: 'community' },
    create: {
      key: 'workspaceDiscovery:user-1',
      value: 'community',
    },
  });
  await expect(response.json()).resolves.toEqual({ choice: 'community' });
});

test('returns the authentication error without reading settings', async () => {
  parseRequestMock.mockResolvedValue({
    auth: null,
    error: () => new Response(null, { status: 401 }),
  });

  const response = await GET(new Request('http://localhost/api/me/discovery'));

  expect(response.status).toBe(401);
  expect(mocks.findUnique).not.toHaveBeenCalled();
});
