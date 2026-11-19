import {
  Button,
  Form,
  FormField,
  FormSubmitButton,
  ListItem,
  Row,
  Select,
  TextField,
} from '@umami/react-zen';
import { useMessages, useUpdateQuery } from '@/components/hooks';
import { DOMAIN_REGEX } from '@/lib/constants';

export const PLATFORM_DOMAINS: Record<string, string> = {
  ios: 'app.ios',
  macos: 'app.macos',
  android: 'app.android',
};

export const PLATFORM_LABELS: Record<string, string> = {
  'app.ios': 'iOS',
  'app.macos': 'macOS',
  'app.android': 'Android',
};

export const PLATFORM_STORE_PLACEHOLDERS: Record<string, string> = {
  ios: 'https://apps.apple.com/app/id123456789',
  macos: 'https://apps.apple.com/app/id123456789',
  android: 'https://play.google.com/store/apps/details?id=com.example.app',
};

export function detectPlatform(domain?: string): 'ios' | 'macos' | 'android' | 'website' {
  if (!domain) return 'ios';
  if (domain === 'app.macos' || /(?:apps|itunes)\.apple\.com\/.*(?:\bmt=12\b|\/mac-app\/)/i.test(domain)) {
    return 'macos';
  }
  if (domain === 'app.ios' || /(?:apps|itunes)\.apple\.com\//i.test(domain)) {
    return 'ios';
  }
  if (domain === 'app.android' || /play\.google\.com\//i.test(domain)) {
    return 'android';
  }
  return 'website';
}

export function formatPlatformDomain(domain?: string): string | null {
  if (!domain) return null;
  if (PLATFORM_LABELS[domain]) return PLATFORM_LABELS[domain];
  if (/(?:apps|itunes)\.apple\.com\//i.test(domain)) {
    const id = domain.match(/\/id(\d+)/i)?.[1];
    return id ? `App Store (id${id})` : 'App Store';
  }
  if (/play\.google\.com\//i.test(domain)) {
    const pkg = domain.match(/[?&]id=([a-zA-Z0-9._]+)/i)?.[1];
    return pkg ? `Google Play (${pkg})` : 'Google Play';
  }
  return null;
}

export function WebsiteAddForm({
  teamId,
  onSave,
  onClose,
  closeOnSave = true,
}: {
  teamId?: string;
  onSave?: (website: any) => void | Promise<void>;
  onClose?: () => void;
  closeOnSave?: boolean;
}) {
  const { t, labels, messages } = useMessages();
  const { mutateAsync, error, isPending } = useUpdateQuery('/websites', { teamId });

  const handleSubmit = async ({ platform, name, domain }: any) => {
    const trimmed = domain?.trim();
    const website = await mutateAsync({
      name,
      domain: trimmed || PLATFORM_DOMAINS[platform] || domain,
    });

    await onSave?.(website);

    if (closeOnSave) {
      onClose?.();
    }
  };

  return (
    <Form
      onSubmit={handleSubmit}
      error={error?.message}
      defaultValues={{ name: '', platform: 'ios', domain: '' }}
    >
      {({ watch, setValue }) => {
        const platform = watch('platform') || 'ios';
        const isWebsite = platform === 'website';
        const storeLabel = platform === 'android' ? 'Google Play URL' : 'App Store URL';

        const handlePlatformChange = (value: string | number | null) => {
          const next = (value as string) || 'website';
          setValue('platform', next, { shouldDirty: true });
          setValue('domain', '', { shouldDirty: true });
        };

        return (
          <>
            <FormField
              label={t(labels.name)}
              data-test="input-name"
              name="name"
              rules={{ required: t(labels.required) }}
            >
              <TextField autoComplete="off" />
            </FormField>

            <FormField label="Platform" name="platform" rules={{ required: t(labels.required) }}>
              <Select
                value={platform}
                onChange={handlePlatformChange}
                data-test="select-platform"
              >
                <ListItem id="ios">iOS</ListItem>
                <ListItem id="macos">macOS</ListItem>
                <ListItem id="android">Android</ListItem>
                <ListItem id="website">{t(labels.website)}</ListItem>
              </Select>
            </FormField>

            <FormField
              label={isWebsite ? t(labels.domain) : storeLabel}
              data-test="input-domain"
              name="domain"
              rules={{
                ...(isWebsite ? { required: t(labels.required) } : {}),
                pattern: { value: DOMAIN_REGEX, message: t(messages.invalidDomain) },
              }}
            >
              <TextField
                autoComplete="off"
                placeholder={isWebsite ? 'example.com' : PLATFORM_STORE_PLACEHOLDERS[platform]}
              />
            </FormField>

            <Row justifyContent="flex-end" paddingTop="3" gap="3">
              {onClose && (
                <Button isDisabled={isPending} onPress={onClose}>
                  {t(labels.cancel)}
                </Button>
              )}
              <FormSubmitButton data-test="button-submit" isDisabled={false}>
                {t(labels.save)}
              </FormSubmitButton>
            </Row>
          </>
        );
      }}
    </Form>
  );
}
