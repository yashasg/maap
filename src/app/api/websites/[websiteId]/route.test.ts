import { beforeEach, expect, test, vi } from 'vitest';
import { parseRequest } from '@/lib/request';
import { canViewSharedWebsite } from '@/permissions';
import { getWebsite } from '@/queries/prisma';
import { GET } from './route';

vi.mock('@/lib/request', () => ({
  parseRequest: vi.fn(),
}));

vi.mock('@/permissions', () => ({
  canDeleteWebsite: vi.fn(),
  canUpdateWebsite: vi.fn(),
  canViewSharedWebsite: vi.fn(),
}));

vi.mock('@/queries/prisma', () => ({
  createShare: vi.fn(),
  deleteSharesByEntityId: vi.fn(),
  deleteWebsite: vi.fn(),
  getShareByEntityId: vi.fn(),
  getWebsite: vi.fn(),
  updateWebsite: vi.fn(),
}));

const parseRequestMock = vi.mocked(parseRequest);
const canViewSharedWebsiteMock = vi.mocked(canViewSharedWebsite);
const getWebsiteMock = vi.mocked(getWebsite);

beforeEach(() => {
  vi.clearAllMocks();
});

test('GET returns null to an admin after a website is deleted', async () => {
  parseRequestMock.mockResolvedValue({
    auth: {
      user: {
        id: 'admin-1',
        username: 'admin',
        role: 'admin',
        isAdmin: true,
      },
    },
    error: undefined,
  } as any);
  canViewSharedWebsiteMock.mockResolvedValue(false);
  getWebsiteMock.mockResolvedValue({ id: 'website-1', deletedAt: new Date() } as any);

  const response = await GET(new Request('http://localhost'), {
    params: Promise.resolve({ websiteId: 'website-1' }),
  });

  expect(response.status).toBe(200);
  await expect(response.json()).resolves.toBeNull();
});

test('GET keeps missing websites unauthorized for non-admin users', async () => {
  parseRequestMock.mockResolvedValue({
    auth: {
      user: {
        id: 'user-1',
        username: 'user',
        role: 'user',
        isAdmin: false,
      },
    },
    error: undefined,
  } as any);
  canViewSharedWebsiteMock.mockResolvedValue(false);

  const response = await GET(new Request('http://localhost'), {
    params: Promise.resolve({ websiteId: 'website-1' }),
  });

  expect(response.status).toBe(401);
  expect(getWebsiteMock).not.toHaveBeenCalled();
});
