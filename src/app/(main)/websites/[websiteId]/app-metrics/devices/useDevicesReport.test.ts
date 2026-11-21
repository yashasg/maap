import { expect, test } from 'vitest';
import { getDevicesReportFilters } from './useDevicesReport';

test('excludes Test Mode by default and preserves an explicit Test Mode filter', () => {
  expect(getDevicesReportFilters({})).toEqual({ tag: 'neq.test' });
  expect(getDevicesReportFilters({ tag: 'test', segment: 'segment-1' })).toEqual({
    tag: 'test',
    segment: 'segment-1',
  });
});
