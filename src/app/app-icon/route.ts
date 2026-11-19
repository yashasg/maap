const iconCache = new Map<string, { body: ArrayBuffer; contentType: string; expires: number }>();
const TTL_MS = 24 * 60 * 60 * 1000;

export async function resolveAppIconUrl(rawUrl: string, fetchImpl = fetch): Promise<string | null> {
  const appleMatch = rawUrl.match(
    /(?:apps|itunes)\.apple\.com\/(?:([a-z]{2})\/)?(?:mac-)?app\/(?:[^/\s?#]+\/)?id(\d+)/i,
  );
  if (appleMatch) {
    const country = (appleMatch[1] || 'us').toLowerCase();
    const appId = appleMatch[2];
    for (const storefront of new Set([country, 'us'])) {
      const res = await fetchImpl(
        `https://itunes.apple.com/lookup?id=${encodeURIComponent(appId)}&country=${encodeURIComponent(storefront)}`,
        { signal: AbortSignal.timeout(8000) },
      );
      if (!res.ok) continue;
      const data = (await res.json()) as {
        results?: Array<{ artworkUrl100?: string; artworkUrl60?: string }>;
      };
      const icon = data.results?.[0]?.artworkUrl100 || data.results?.[0]?.artworkUrl60;
      if (icon) return icon;
    }
    return null;
  }

  const playMatch = rawUrl.match(
    /play\.google\.com\/store\/apps\/details\?(?:[^#\s]*&)?id=([a-zA-Z0-9._]+)/i,
  );
  if (playMatch) {
    const appId = playMatch[1];
    const res = await fetchImpl(
      `https://play.google.com/store/apps/details?id=${encodeURIComponent(appId)}&hl=en_US`,
      { signal: AbortSignal.timeout(8000) },
    );
    if (!res.ok) return null;
    const html = await res.text();
    const ogMatch =
      html.match(/<meta\s+property=["']og:image["']\s+content=["']([^"']+)["']/i) ||
      html.match(/<meta\s+content=["']([^"']+)["']\s+property=["']og:image["']/i);
    return ogMatch?.[1] ?? null;
  }

  return null;
}

export async function GET(request: Request) {
  const { searchParams } = new URL(request.url);
  const rawUrl = searchParams.get('url')?.trim() || '';
  if (!rawUrl) {
    return new Response('Missing url parameter', { status: 400 });
  }

  const now = Date.now();
  const cached = iconCache.get(rawUrl);
  if (cached && cached.expires > now) {
    return new Response(cached.body, {
      headers: {
        'content-type': cached.contentType,
        'cache-control': 'public, max-age=86400, s-maxage=604800',
      },
    });
  }

  try {
    const iconUrl = await resolveAppIconUrl(rawUrl);
    if (iconUrl) {
      const imgRes = await fetch(iconUrl, { signal: AbortSignal.timeout(8000) });
      if (imgRes.ok) {
        const body = await imgRes.arrayBuffer();
        const contentType = imgRes.headers.get('content-type') || 'image/jpeg';
        if (iconCache.size > 200) iconCache.clear();
        iconCache.set(rawUrl, { body, contentType, expires: now + TTL_MS });
        return new Response(body, {
          headers: {
            'content-type': contentType,
            'cache-control': 'public, max-age=86400, s-maxage=604800',
          },
        });
      }
    }
  } catch {
    // Fall back to default platform icon below.
  }

  const fallback = /play\.google\.com/i.test(rawUrl)
    ? '/images/os/android-os.png'
    : /mt=12|mac-app/i.test(rawUrl)
      ? '/images/os/mac-os.png'
      : '/images/os/ios.png';
  return Response.redirect(new URL(fallback, request.url), 302);
}
