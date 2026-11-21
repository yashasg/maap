function getUrlOrigin(url: string) {
  try {
    return new URL(url).origin;
  } catch {
    return '';
  }
}

// Builds the Content-Security-Policy. Reads the environment when called, so it
// can run at build time (next.config.ts) or per request in the Docker proxy,
// where ALLOWED_FRAME_URLS and API_URL then apply without rebuilding the image.
export function getContentSecurityPolicy() {
  const live = process.env.NODE_ENV === 'development' ? ' http://localhost:8400' : '';
  const apiUrlOrigin = getUrlOrigin(process.env.API_URL || '');
  const connectSrc = ["'self'", 'https:', apiUrlOrigin].filter(Boolean).join(' ');
  const frameAncestors = ["'self'", process.env.ALLOWED_FRAME_URLS || ''].filter(Boolean).join(' ');

  const directives = [
    `default-src 'self'`,
    `img-src 'self' https: data: blob:`,
    `script-src 'self' 'unsafe-eval' 'unsafe-inline'${live}`,
    `style-src 'self' 'unsafe-inline'`,
    `connect-src ${connectSrc}${live}`,
    `frame-src 'self' http: https:`,
    `frame-ancestors ${frameAncestors}`,
  ];

  return `${directives.join('; ')};`;
}
