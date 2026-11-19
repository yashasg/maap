import {
  Form,
  FormButtons,
  FormField,
  FormSubmitButton,
  ListItem,
  Select,
  TextField,
} from '@umami/react-zen';
import { PLATFORM_DOMAINS } from '@/app/(main)/websites/WebsiteAddForm';
import { useMessages, useUpdateQuery, useWebsite } from '@/components/hooks';
import { DOMAIN_REGEX } from '@/lib/constants';

export function WebsiteEditForm({ websiteId, onSave }: { websiteId: string; onSave?: () => void }) {
  const website = useWebsite();
  const { t, labels, messages, getErrorMessage } = useMessages();
  const { mutateAsync, error, touch, toast } = useUpdateQuery(`/websites/${websiteId}`);
  const initialPlatform =
    Object.entries(PLATFORM_DOMAINS).find(([, domain]) => domain === website?.domain)?.[0] ??
    'website';

  const handleSubmit = async (data: any) => {
    const { shareId, platform, ...updateData } = data;
    updateData.domain = PLATFORM_DOMAINS[platform] ?? updateData.domain;
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
      values={{ ...website, platform: initialPlatform }}
    >
      {({ watch, setValue }) => {
        const platform = watch('platform') || initialPlatform;
        const isWebsite = platform === 'website';

        const handlePlatformChange = (value: string | number | null) => {
          const next = (value as string) || 'website';
          setValue('platform', next, { shouldDirty: true });
          setValue('domain', PLATFORM_DOMAINS[next] ?? '', { shouldDirty: true });
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
            {isWebsite && (
              <FormField
                label={t(labels.domain)}
                data-test="input-domain"
                name="domain"
                rules={{
                  required: t(labels.required),
                  pattern: {
                    value: DOMAIN_REGEX,
                    message: t(messages.invalidDomain),
                  },
                }}
              >
                <TextField />
              </FormField>
            )}
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
