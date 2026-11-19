import {
  Form,
  FormButtons,
  FormField,
  FormSubmitButton,
  ListItem,
  Select,
  TextField,
} from '@umami/react-zen';
import {
  detectPlatform,
  PLATFORM_DOMAINS,
  PLATFORM_STORE_PLACEHOLDERS,
} from '@/app/(main)/websites/WebsiteAddForm';
import { useMessages, useUpdateQuery, useWebsite } from '@/components/hooks';
import { DOMAIN_REGEX } from '@/lib/constants';

export function WebsiteEditForm({ websiteId, onSave }: { websiteId: string; onSave?: () => void }) {
  const website = useWebsite();
  const { t, labels, messages, getErrorMessage } = useMessages();
  const { mutateAsync, error, touch, toast } = useUpdateQuery(`/websites/${websiteId}`);
  const initialPlatform = detectPlatform(website?.domain);
  const initialDomain = Object.values(PLATFORM_DOMAINS).includes(website?.domain)
    ? ''
    : (website?.domain ?? '');

  const handleSubmit = async (data: any) => {
    const { shareId, platform, ...updateData } = data;
    const trimmed = updateData.domain?.trim();
    updateData.domain = trimmed || PLATFORM_DOMAINS[platform] || updateData.domain;
    await mutateAsync(updateData, {
      onSuccess: async () => {
        toast(t(messages.saved));
        touch('websites');
        touch(`website:${website.id}`);
        onSave?.();
      },
    });
  };

  return (
    <Form
      onSubmit={handleSubmit}
      error={getErrorMessage(error)}
      values={{ ...website, platform: initialPlatform, domain: initialDomain }}
    >
      {({ watch, setValue }) => {
        const platform = watch('platform') || initialPlatform;
        const isWebsite = platform === 'website';
        const storeLabel = platform === 'android' ? 'Google Play URL' : 'App Store URL';

        const handlePlatformChange = (value: string | number | null) => {
          const next = (value as string) || 'website';
          setValue('platform', next, { shouldDirty: true });
          if (next === 'website' && Object.values(PLATFORM_DOMAINS).includes(watch('domain'))) {
            setValue('domain', '', { shouldDirty: true });
          }
        };

        return (
          <>
            <FormField name="id" label={t(labels.websiteId)}>
              <TextField data-test="text-field-websiteId" value={website?.id} isReadOnly allowCopy />
            </FormField>
            <FormField
              label={t(labels.name)}
              data-test="input-name"
              name="name"
              rules={{ required: t(labels.required) }}
            >
              <TextField />
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
                pattern: {
                  value: DOMAIN_REGEX,
                  message: t(messages.invalidDomain),
                },
              }}
            >
              <TextField
                placeholder={isWebsite ? 'example.com' : PLATFORM_STORE_PLACEHOLDERS[platform]}
              />
            </FormField>
            <FormButtons>
              <FormSubmitButton data-test="button-submit" variant="primary">
                {t(labels.save)}
              </FormSubmitButton>
            </FormButtons>
          </>
        );
      }}
    </Form>
  );
}
