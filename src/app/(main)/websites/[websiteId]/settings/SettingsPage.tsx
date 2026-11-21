'use client';
import { Column, Heading } from '@umami/react-zen';
import { useMessages } from '@/components/hooks';
import { WebsiteSettings } from './WebsiteSettings';

export function SettingsPage({ websiteId }: { websiteId: string }) {
  const { t, labels } = useMessages();
  return (
    <Column gap width="100%" maxWidth="800px" style={{ marginInline: 'auto' }}>
      <Heading>{t(labels.settings)}</Heading>
      <WebsiteSettings websiteId={websiteId} />
    </Column>
  );
}
