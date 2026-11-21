import {
  getRedirectUrl,
  unstable_getResponseFromNextConfig,
} from 'next/experimental/testing/server';
import { afterEach, expect, test, vi } from 'vitest';

afterEach(() => {
  vi.unstubAllEnvs();
});

test.each(['', '/analytics'])(
  'keeps SDK preflights direct and dashboard redirects unchanged with base path %j',
  async basePath => {
    vi.stubEnv('BASE_PATH', basePath);
    const responseForPath = async (path: string) => {
      // Next's route loader mutates patterns when applying the base path.
      vi.resetModules();
      const { default: nextConfig } = await import('../../next.config');
      return unstable_getResponseFromNextConfig({
        url: `https://collector.example${basePath}${path}`,
        nextConfig,
      });
    };

    for (const path of ['/v2', '/v2/', '/v2/namespace/browser', '/v2/namespace/browser/']) {
      const response = await responseForPath(path);
      expect(response.status, path).toBe(200);
      expect(getRedirectUrl(response), path).toBeNull();
      expect(response.headers.get('Access-Control-Allow-Origin')).toBe('*');
    }

    for (const path of ['/login/', '/websites/', '/v20/', ...(basePath ? ['/'] : [])]) {
      const response = await responseForPath(path);
      expect(response.status, path).toBe(308);
      expect(getRedirectUrl(response)).toBe(
        `https://collector.example${basePath}${path.slice(0, -1)}`,
      );
    }
  },
);
