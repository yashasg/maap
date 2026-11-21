import { expect, test } from 'vitest';
import { isAllowedCollectionOrigin } from './collection-origin';

test('requires the registered browser hostname, allowing www but not arbitrary subdomains', () => {
  const allowed = (origin: string, payload = {}, domain: string | null = 'kashiworks.com') =>
    isAllowedCollectionOrigin(new Headers({ origin }), domain, payload);

  expect(allowed('https://kashiworks.com')).toBe(true);
  expect(allowed('https://WWW.KASHIWORKS.COM:8443', { url: '/products' })).toBe(true);
  for (const origin of [
    'https://maap.kashiworks.com',
    'https://kashiworks.com.evil.example',
    'https://evil.example',
    'null',
    '',
    'file:///tmp/page',
    'https://user@kashiworks.com',
    'https://kashiworks.com/path',
  ]) {
    expect(allowed(origin), origin).toBe(false);
  }
  expect(allowed('https://kashiworks.com', {}, null)).toBe(false);
  expect(allowed('https://kashiworks.com', { hostname: 'maap.kashiworks.com' })).toBe(false);
  expect(allowed('https://kashiworks.com', { url: '//maap.kashiworks.com/' })).toBe(false);
  expect(allowed('https://kashiworks.com', { url: 'https://maap.kashiworks.com/' })).toBe(false);
  expect(isAllowedCollectionOrigin(new Headers(), 'app.ios', { hostname: 'app.ios' })).toBe(true);
});
