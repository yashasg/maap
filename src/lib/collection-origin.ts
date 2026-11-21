// Origin is browser context, not authentication. Native/server SDKs omit it.
export function isAllowedCollectionOrigin(
  headers: Headers,
  domain?: string | null,
  payload: { hostname?: string; url?: string } = {},
) {
  const origin = headers.get('origin');
  if (origin === null) return true;

  try {
    const source = new URL(origin);
    if (
      !['http:', 'https:'].includes(source.protocol) ||
      source.username ||
      source.password ||
      source.pathname !== '/' ||
      source.search ||
      source.hash ||
      !domain
    ) {
      return false;
    }
    const hostname = (url: URL) => url.hostname.replace(/^www\./, '').replace(/\.$/, '');
    const registered = new URL(domain.includes('://') ? domain : `https://${domain}`);
    const expected = hostname(registered);
    return (
      hostname(source) === expected &&
      (!payload.hostname || hostname(new URL(`https://${payload.hostname}`)) === expected) &&
      (!payload.url || hostname(new URL(payload.url, source)) === expected)
    );
  } catch {
    return false;
  }
}
