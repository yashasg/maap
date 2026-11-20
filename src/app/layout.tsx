import { FlagValues as FlagValuesScript } from 'flags/react';
import type { Metadata } from 'next';
import { Suspense } from 'react';
import { getConfig } from '@/lib/config';
import { getDefaultFlagValues } from '@/lib/flags';
import { Providers } from './Providers';
import '@umami/react-zen/styles.full.css';
import './global.css';

export default function ({ children }) {
  if (process.env.DISABLE_UI) {
    return (
      <html>
        <body></body>
      </html>
    );
  }

  const config = getConfig();
  const flags = getDefaultFlagValues();

  return (
    <html lang="en">
      <head>
        <link rel="stylesheet" href={`${process.env.basePath || ''}/fonts/satoshi.css`} />
        <link rel="icon" href="/favicon.ico" />
        <link rel="apple-touch-icon" sizes="180x180" href="/apple-touch-icon.png" />
        <link rel="icon" type="image/png" sizes="32x32" href="/favicon-32x32.png" />
        <link rel="icon" type="image/png" sizes="16x16" href="/favicon-16x16.png" />
        <link rel="manifest" href="/site.webmanifest" crossOrigin="use-credentials" />
        <link rel="mask-icon" href="/safari-pinned-tab.svg" color="#5bbad5" />
        <meta name="msapplication-TileColor" content="#da532c" />
        <meta name="theme-color" content="#fafafa" media="(prefers-color-scheme: light)" />
        <meta name="theme-color" content="#2f2f2f" media="(prefers-color-scheme: dark)" />
        <meta name="robots" content="noindex,nofollow" />
      </head>
      <body>
        <Suspense>
          <Providers config={config} flags={flags}>
            {children}
          </Providers>
        </Suspense>
        <FlagValuesScript values={flags} />
      </body>
    </html>
  );
}

export const metadata: Metadata = {
  title: {
    template: '%s | Maap',
    default: 'Maap',
  },
};
