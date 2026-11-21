import { afterEach, expect, it, vi } from 'vitest';
import { getContentSecurityPolicy } from './csp';

afterEach(() => vi.unstubAllEnvs());

it('allows the Impeccable picker only for development scripts and connections', () => {
  const origin = 'http://localhost:8400';
  vi.stubEnv('API_URL', '');
  vi.stubEnv('NODE_ENV', 'development');
  const directives = getContentSecurityPolicy().split('; ');
  expect(
    directives.filter(rule => rule.split(' ').includes(origin)).map(rule => rule.split(' ')[0]),
  ).toEqual(['script-src', 'connect-src']);

  vi.stubEnv('NODE_ENV', 'production');
  expect(getContentSecurityPolicy()).not.toContain(origin);
});
