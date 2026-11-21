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

const rawQuery = vi.mocked(prisma.rawQuery);

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(parseRequest).mockResolvedValue({
    auth: { user: { id: 'user-1' } },
    query: { testMode: 'true' },
    error: undefined,
  } as any);
  vi.mocked(canViewWebsiteSection).mockResolvedValue(true);
  vi.mocked(getQueryFilters).mockResolvedValue({
    minimumDate: new Date('2026-05-01T00:00:00.000Z'),
    startDate: new Date('2026-10-01T00:00:00.000Z'),
    endDate: new Date('2026-10-31T23:59:59.999Z'),
    timezone: 'America/Los_Angeles',
    tag: 'test',
  } as any);
  rawQuery.mockResolvedValue([
    { section: 'series', metric: 'daily', bucket: '2026-10-02T00:00:00Z', count: 2n },
    { section: 'scope', metric: 'totalSignals', bucket: '', count: 10n },
    { section: 'scope', metric: 'identifiedSignals', bucket: '', count: 8n },
    { section: 'scope', metric: 'returningUsers', bucket: '', count: 2n },
  ]);
});

test('returns scoped distinct clientUser counts without cohort percentages', async () => {
  const response = await GET(
    new Request(
      'http://localhost/api/websites/00000000-0000-0000-0000-000000000001/app-reports/retention?testMode=true',
    ),
    { params: Promise.resolve({ websiteId: '00000000-0000-0000-0000-000000000001' }) },
  );
  const body = await response.json();
  const [sql, params, name] = rawQuery.mock.calls[0];

  expect(response.status).toBe(200);
  expect(name).toBe('getAppRetentionReport');
  expect(sql).toContain('count(distinct ie.user_id)');
  expect(sql).toContain('first_seen.first_seen <');
  expect(sql).toContain("date_trunc('week'");
  expect(sql).toContain("e.tag = 'test'");
  expect(sql).toContain('e.created_at >= {{minimumDate}}');
  expect(sql).not.toContain('percentage');
  expect(sql).not.toContain('sum(');
  expect(params).toMatchObject({
    timezone: 'America/Los_Angeles',
    minimumDate: new Date('2026-05-01T00:00:00.000Z'),
    startDate: new Date('2026-10-01T00:00:00.000Z'),
    hourStart: new Date('2026-10-30T23:59:59.999Z'),
  });
  expect(body.criteria.verified).toBe(false);
  expect(body.identity.source).toBe('clientUser');
  expect(body.range.testMode).toBe(true);
  expect(body.scope.returningUsers).toBe(2);
  expect(body.series.daily).toEqual([{ bucket: '2026-10-02T00:00:00Z', returningUsers: 2 }]);
});
