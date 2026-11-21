import { beforeEach, expect, test, vi } from 'vitest';
import prisma from '@/lib/prisma';
import { getQueryFilters, parseRequest } from '@/lib/request';
import { canViewWebsiteSection } from '@/permissions';
import { buildDevicesResponse, GET } from './route';

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

test('keeps a landscape tablet in exact device buckets and preserves missing orientation', () => {
  const response = buildDevicesResponse([
    { metricIndex: 0, value: 'iPad14,3', signals: 3 },
    { metricIndex: 1, value: 'iOS 18.4.1', signals: 3 },
    { metricIndex: 2, value: 'iPad14,3', signals: 3 },
    { metricIndex: 3, value: 'iPadOS', signals: 3 },
    { metricIndex: 4, value: '834', signals: 3 },
    { metricIndex: 5, value: '1194', signals: 3 },
    { metricIndex: 6, value: 'Dark', signals: 3 },
    { metricIndex: 7, value: 'landscape', signals: 2 },
    { metricIndex: 7, value: null, signals: 1 },
  ]);

  expect(response.reports[2].values).toEqual([{ value: 'iPad14,3', signals: 3 }]);
  expect(response.reports[3].values).toEqual([{ value: 'iPadOS', signals: 3 }]);
  expect(response.reports[4].values).toEqual([{ value: '834', signals: 3 }]);
  expect(response.reports[5].values).toEqual([{ value: '1194', signals: 3 }]);
  expect(response.reports[7]).toMatchObject({
    totalSignals: 3,
    reportedSignals: 2,
    values: [
      { value: 'landscape', signals: 2 },
      { value: null, signals: 1 },
    ],
  });
});

test('authorizes and applies app, range, and Test Mode to the aggregate query', async () => {
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
      'http://localhost/api/websites/website-1/app-reports/devices?startAt=1000&endAt=2000&testMode=true',
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
    'getAppDeviceBreakdowns',
  );
  expect(response.status).toBe(200);
  expect((await response.json()).reports).toHaveLength(8);
});
