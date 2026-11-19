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
    const website = await mutateAsync({
      name,
      domain: PLATFORM_DOMAINS[platform] ?? domain,
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
      defaultValues={{ name: '', platform: 'ios', domain: 'app.ios' }}
    >
      {({ watch, setValue }) => {
        const platform = watch('platform') || 'ios';
        const isWebsite = platform === 'website';

        const handlePlatformChange = (value: string | number | null) => {
          const next = (value as string) || 'website';
          setValue('platform', next, { shouldDirty: true });
          setValue('domain', PLATFORM_DOMAINS[next] ?? '', { shouldDirty: true });
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

            {isWebsite && (
              <FormField
                label={t(labels.domain)}
                data-test="input-domain"
                name="domain"
                rules={{
                  required: t(labels.required),
                  pattern: { value: DOMAIN_REGEX, message: t(messages.invalidDomain) },
                }}
              >
                <TextField autoComplete="off" />
              </FormField>
            )}

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
