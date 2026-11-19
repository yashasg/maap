'use client';
import { usePathname } from 'next/navigation';
import NotFound from '@/app/not-found';
import { SharePage } from './[slug]/[[...path]]/SharePage';
import { ShareProvider } from './ShareProvider';

// Static shell for standalone Rust; the slug selects the same provider as the Next route.
export default function StandaloneSharePage() {
  const slug = usePathname().split('/')[2];
  if (!slug) return <NotFound />;
  return (
    <ShareProvider slug={slug}>
      <SharePage />
    </ShareProvider>
  );
}
