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
  rawQuery.mockResolvedValue([]);
});

test('clamps rolling output to the policy floor without shortening it to the selected start', async () => {
  vi.mocked(getQueryFilters).mockResolvedValue({
    minimumDate: new Date('2026-05-01T00:00:00.000Z'),
    startDate: new Date('2026-11-01T00:00:00.000Z'),
    endDate: new Date('2026-11-20T23:59:59.999Z'),
    timezone: 'UTC',
    tag: 'neq.test',
  } as any);

  const response = await GET(
    new Request(`http://localhost/api/websites/${websiteId}/app-reports/acquisition`),
    { params: Promise.resolve({ websiteId }) },
  );
  const [sql, params] = rawQuery.mock.calls[0];

  expect(response.status).toBe(200);
  expect(sql).toContain('e.created_at >= {{minimumDate}}');
  expect(sql).toContain("('monthly'::text, 'month'::text, {{monthlyStart}}::timestamptz)");
  expect(params).toMatchObject({
    minimumDate: new Date('2026-05-01T00:00:00.000Z'),
    startDate: new Date('2026-11-01T00:00:00.000Z'),
    monthlyStart: new Date('2026-05-01T00:00:00.000Z'),
    weeklyStart: new Date('2026-08-20T23:59:59.999Z'),
    fourWeeksStart: new Date('2026-10-23T23:59:59.999Z'),
  });
  expect(params.monthlyStart.getTime()).toBeLessThan(params.startDate.getTime());
});

test('returns an empty report without querying when the permitted range is wholly disallowed', async () => {
  vi.mocked(getQueryFilters).mockResolvedValue({
    minimumDate: new Date('2026-05-01T00:00:00.000Z'),
    startDate: new Date('2026-05-01T00:00:00.000Z'),
    endDate: new Date('2026-04-30T23:59:59.999Z'),
    timezone: 'UTC',
    tag: 'neq.test',
  } as any);

  const response = await GET(
    new Request(`http://localhost/api/websites/${websiteId}/app-reports/acquisition`),
    { params: Promise.resolve({ websiteId }) },
  );
  const body = await response.json();

  expect(response.status).toBe(200);
  expect(rawQuery).not.toHaveBeenCalled();
  expect(body.range.totalSignals).toBe(0);
  expect(body.series.monthly).toEqual([]);
  expect(body.patterns.hourOfDay4Weeks).toEqual([]);
});
