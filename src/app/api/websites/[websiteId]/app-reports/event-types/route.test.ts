import { beforeEach, expect, test, vi } from 'vitest';
import prisma from '@/lib/prisma';
import { getQueryFilters, parseRequest } from '@/lib/request';
import { canViewWebsiteSection } from '@/permissions';
import { GET } from './route';

vi.mock('@/lib/prisma', () => ({
  default: { rawQuery: vi.fn() },
}));

vi.mock('@/lib/request', () => ({
  parseRequest: vi.fn(),
  getQueryFilters: vi.fn(),
}));

vi.mock('@/permissions', () => ({
  canViewWebsiteSection: vi.fn(),
}));

const websiteId = '00000000-0000-0000-0000-000000000001';
const rawQuery = vi.mocked(prisma.rawQuery);

beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(parseRequest).mockResolvedValue({
    auth: { user: { id: 'user-1' } },
    query: {},
    error: undefined,
  } as any);
  vi.mocked(canViewWebsiteSection).mockResolvedValue(true);
  vi.mocked(getQueryFilters).mockResolvedValue({
    startDate: new Date('2026-11-01T00:00:00.000Z'),
    endDate: new Date('2026-11-30T23:59:59.999Z'),
    tag: 'neq.test',
  } as any);
  rawQuery.mockResolvedValue([
    { name: 'cart.added', users: 2n, events: 4n },
    { name: 'checkout.started', users: 1n, events: 1n },
  ]);
});

test('returns event frequency and distinct SDK-user reach for the scoped app and range', async () => {
  const response = await GET(
    new Request(`http://localhost/api/websites/${websiteId}/app-reports/event-types`),
    { params: Promise.resolve({ websiteId }) },
  );
  const body = await response.json();
  const [sql, values, queryName] = rawQuery.mock.calls[0];

  expect(response.status).toBe(200);
  expect(canViewWebsiteSection).toHaveBeenCalledWith(expect.anything(), websiteId, 'events');
  expect(values).toMatchObject({
    websiteId,
    startDate: new Date('2026-11-01T00:00:00.000Z'),
    endDate: new Date('2026-11-30T23:59:59.999Z'),
  });
  expect(queryName).toBe('getAppEventTypes');
  expect(sql).toContain("count(distinct nullif(s.distinct_id, ''))");
  expect(sql).not.toContain('count(distinct e.session_id)');
  expect(sql).toContain("coalesce(e.tag, '') != 'test'");
  expect(body).toEqual({
    data: [
      { name: 'cart.added', users: 2, events: 4 },
      { name: 'checkout.started', users: 1, events: 1 },
    ],
  });
});

test('selects only test signals when Test Mode is enabled', async () => {
  vi.mocked(getQueryFilters).mockResolvedValue({
    startDate: new Date('2026-11-01T00:00:00.000Z'),
    endDate: new Date('2026-11-30T23:59:59.999Z'),
    tag: 'test',
  } as any);

  await GET(new Request(`http://localhost/api/websites/${websiteId}/app-reports/event-types`), {
    params: Promise.resolve({ websiteId }),
  });

  expect(rawQuery.mock.calls[0][0]).toContain("e.tag = 'test'");
});
