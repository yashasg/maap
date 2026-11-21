import { beforeEach, expect, test, vi } from 'vitest';
import prisma from '@/lib/prisma';
import { getQueryFilters, parseRequest } from '@/lib/request';
import { canViewWebsiteSection } from '@/permissions';
import { buildAccessibilityResponse, GET } from './route';

vi.mock('@/lib/prisma', () => ({
  default: { rawQuery: vi.fn() },
}));

vi.mock('@/lib/request', () => ({
  getQueryFilters: vi.fn(),
  parseRequest: vi.fn(),
}));

vi.mock('@/permissions', () => ({
  canViewWebsiteSection: vi.fn(),
}));

const parseRequestMock = vi.mocked(parseRequest);
const getQueryFiltersMock = vi.mocked(getQueryFilters);
const canViewWebsiteSectionMock = vi.mocked(canViewWebsiteSection);
const rawQueryMock = vi.mocked(prisma.rawQuery);

beforeEach(() => {
  parseRequestMock.mockReset();
  getQueryFiltersMock.mockReset();
  canViewWebsiteSectionMock.mockReset();
  rawQueryMock.mockReset();
});

test('keeps off separate from missing and never returns unexpected raw values', () => {
  const response = buildAccessibilityResponse([
    { metricIndex: 0, present: true, rawValue: 'L', signals: 4 },
    { metricIndex: 0, present: true, rawValue: 'private-value', signals: 2 },
    { metricIndex: 0, present: false, rawValue: null, signals: 1 },
    { metricIndex: 1, present: true, rawValue: 'true', signals: 2 },
    { metricIndex: 1, present: true, rawValue: 'false', signals: 3 },
    { metricIndex: 1, present: false, rawValue: null, signals: 5 },
  ]);

  expect(response.reports[1]).toMatchObject({
    totalSignals: 10,
    reportedSignals: 5,
    values: [
      { label: 'On', signals: 2 },
      { label: 'Off', signals: 3 },
      { label: 'Not reported', signals: 5 },
    ],
  });
  expect(response.reports[0].values).toEqual([
    { label: 'Large', signals: 4 },
    { label: 'Other reported size', signals: 2 },
    { label: 'Not reported', signals: 1 },
  ]);
  expect(JSON.stringify(response)).not.toContain('private-value');
});

test('authorizes and scopes the standalone Node query to app, range, and Test Mode', async () => {
  parseRequestMock.mockResolvedValue({
    auth: { user: { id: 'user-1' } },
    query: { startAt: 1000, endAt: 2000, testMode: 'true' },
    error: undefined,
  });
  canViewWebsiteSectionMock.mockResolvedValue(true);
  getQueryFiltersMock.mockResolvedValue({
    startDate: new Date(1000),
    endDate: new Date(2000),
    tag: 'eq.test',
  } as any);
  rawQueryMock.mockResolvedValue([]);

  const response = await GET(
    new Request(
      'http://localhost/api/websites/website-1/app-reports/accessibility?startAt=1000&endAt=2000&testMode=true',
    ),
    { params: Promise.resolve({ websiteId: 'website-1' }) },
  );

  expect(canViewWebsiteSectionMock).toHaveBeenCalledWith(
    { user: { id: 'user-1' } },
    'website-1',
    'events',
  );
  expect(rawQueryMock).toHaveBeenCalledWith(
    expect.stringContaining("e.tag = 'test'"),
    {
      websiteId: 'website-1',
      startDate: new Date(1000),
      endDate: new Date(2000),
    },
    'getAppAccessibilityBreakdowns',
  );
  expect(response.status).toBe(200);
  expect((await response.json()).reports).toHaveLength(7);
});
