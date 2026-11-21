import { beforeEach, describe, expect, test, vi } from 'vitest';
import { POST as send } from '@/app/api/send/route';
import * as namespace from './namespace/[namespace]/route';
import * as root from './route';

vi.mock('@/app/api/send/route', () => ({ POST: vi.fn() }));

beforeEach(() => {
  vi.clearAllMocks();
});

describe.each([
  ['root', root],
  ['namespace', namespace],
] as const)('TelemetryDeck %s CORS', (_name, route) => {
  test('accepts JSON and cache-token preflights without ingesting', async () => {
    const response = route.OPTIONS();
    expect(response.status).toBe(204);
    expect(await response.text()).toBe('');
    expect(response.headers.get('Access-Control-Allow-Origin')).toBe('*');
    expect(response.headers.get('Access-Control-Allow-Headers')).toContain('Content-Type');
    expect(response.headers.get('Access-Control-Allow-Headers')).toContain('x-umami-cache');
    expect(response.headers.get('Access-Control-Allow-Methods')).toContain('POST');
    expect(send).not.toHaveBeenCalled();
  });

  test.each([200, 503])(
    'keeps ingestion counts browser-readable when send returns %i',
    async status => {
      vi.mocked(send).mockResolvedValue(Response.json({}, { status }));
      const response = await route.POST(
        new Request('http://collector.example/v2', {
          method: 'POST',
          headers: { Origin: 'https://browser.example', 'Content-Type': 'application/json' },
          body: JSON.stringify([
            {
              appID: '11111111-1111-4111-8111-111111111111',
              clientUser: 'a'.repeat(64),
              type: 'browser-event',
              payload: {},
            },
          ]),
        }),
      );
      expect(send).toHaveBeenCalledOnce();
      expect(response.status).toBe(200);
      expect(response.headers.get('Access-Control-Allow-Origin')).toBe('*');
      expect(await response.json()).toEqual({
        accepted: status === 200 ? 1 : 0,
        rejected: status === 200 ? 0 : 1,
        dropped: 0,
      });
    },
  );

  test.each(['[', '{}'])('keeps invalid batch responses browser-readable: %s', async body => {
    const response = await route.POST(
      new Request('http://collector.example/v2', { method: 'POST', body }),
    );
    expect(response.status).toBe(400);
    expect(response.headers.get('Access-Control-Allow-Origin')).toBe('*');
    expect(await response.json()).toEqual({ error: 'Expected a JSON array of signals.' });
    expect(send).not.toHaveBeenCalled();
  });
});
