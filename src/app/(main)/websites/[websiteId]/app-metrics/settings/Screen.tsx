'use client';

import {
  Alert,
  AlertTitle,
  Button,
  Column,
  Dialog,
  DialogTrigger,
  Form,
  FormButtons,
  FormField,
  FormSubmitButton,
  Heading,
  ListItem,
  Loading,
  Modal,
  Select,
  Text,
  TextField,
  useToast,
} from '@umami/react-zen';
import { type ReactNode, useState } from 'react';
import { WebsiteDeleteForm } from '@/app/(main)/websites/[websiteId]/settings/WebsiteDeleteForm';
import {
  detectPlatform,
  PLATFORM_DOMAINS,
  PLATFORM_STORE_PLACEHOLDERS,
} from '@/app/(main)/websites/WebsiteAddForm';
import {
  useAppSettingsMutation,
  useAppSettingsQuery,
  useMessages,
  useNavigation,
  useUpdateQuery,
  useWebsite,
} from '@/components/hooks';
import { APP_COLOR_PALETTES, type AppColorScheme, DEFAULT_APP_SETTINGS } from '@/lib/app-settings';
import { DOMAIN_REGEX } from '@/lib/constants';
import styles from './Screen.module.css';

type Platform = 'ios' | 'macos' | 'android' | 'website';

function SettingSection({
  title,
  description,
  children,
  danger,
}: {
  title: string;
  description: string;
  children: ReactNode;
  danger?: boolean;
}) {
  return (
    <section className={`${styles.section} ${danger ? styles.dangerSection : ''}`}>
      <div className={styles.sectionIntro}>
        <Heading size="base">{title}</Heading>
        <Text color="muted">{description}</Text>
      </div>
      <div className={styles.sectionContent}>{children}</div>
    </section>
  );
}

function ColorPreview({ scheme }: { scheme: AppColorScheme }) {
  return (
    <div className={styles.swatches} aria-label={`${scheme} color palette`}>
      {APP_COLOR_PALETTES[scheme].map(color => (
        <span key={color} style={{ backgroundColor: color }} />
      ))}
    </div>
  );
}

export default function Screen({ websiteId }: { websiteId: string }) {
  const website = useWebsite();
  const settingsQuery = useAppSettingsQuery(websiteId);
  const settingsMutation = useAppSettingsMutation(websiteId);
  const websiteMutation = useUpdateQuery(`/websites/${websiteId}`);
  const { t, labels, messages, getErrorMessage } = useMessages();
  const { router, renderUrl } = useNavigation();
  const { toast } = useToast();
  const [nameVersion, setNameVersion] = useState(0);
  const [preferenceVersion, setPreferenceVersion] = useState(0);
  const [infoVersion, setInfoVersion] = useState(0);

  const settings = settingsQuery.data ?? {
    websiteId,
    ...DEFAULT_APP_SETTINGS,
    canWrite: false,
  };
  const canManage = settings.canWrite;
  const platform = detectPlatform(website?.domain);
  const initialDomain = Object.values(PLATFORM_DOMAINS).includes(website?.domain)
    ? ''
    : (website?.domain ?? '');

  const saveWebsite = async (data: Record<string, string>, onSuccess: () => void) => {
    if (!canManage) return;

    await websiteMutation.mutateAsync(data, {
      onSuccess: () => {
        toast(t(messages.saved));
        websiteMutation.touch('websites');
        websiteMutation.touch(`website:${websiteId}`);
        onSuccess();
      },
    });
  };

  const handleNameSubmit = async ({ name }: { name: string }) => {
    await saveWebsite({ name }, () => setNameVersion(value => value + 1));
  };

  const handlePreferencesSubmit = async (data: typeof DEFAULT_APP_SETTINGS) => {
    if (!canManage) return;

    await settingsMutation.mutateAsync(data, {
      onSuccess: () => {
        toast(t(messages.saved));
        settingsMutation.touch(`app-settings:${websiteId}`);
        setPreferenceVersion(value => value + 1);
      },
    });
  };

  const handleAppInfoSubmit = async ({
    platform: selectedPlatform,
    domain,
  }: {
    platform: Platform;
    domain: string;
  }) => {
    const trimmedDomain = domain?.trim();
    const nextDomain = trimmedDomain || PLATFORM_DOMAINS[selectedPlatform] || website?.domain || '';

    await saveWebsite({ domain: nextDomain }, () => {
      setInfoVersion(value => value + 1);
      if (selectedPlatform === 'website') {
        router.push(renderUrl(`/websites/${websiteId}`));
      }
    });
  };

  const handleDelete = () => {
    websiteMutation.touch('websites');
    router.push(renderUrl('/websites'));
  };

  if (settingsQuery.isLoading) {
    return (
      <main className={styles.screen}>
        <div className={styles.header}>
          <Heading size="2xl">App Settings</Heading>
          <Text color="muted">Loading settings for this app.</Text>
        </div>
        <Loading />
      </main>
    );
  }

  if (settingsQuery.error) {
    return (
      <main className={styles.screen}>
        <div className={styles.header}>
          <Heading size="2xl">App Settings</Heading>
        </div>
        <Alert variant="danger">
          <AlertTitle>{getErrorMessage(settingsQuery.error)}</AlertTitle>
          <Button onPress={() => settingsQuery.refetch()}>Try again</Button>
        </Alert>
      </main>
    );
  }

  return (
    <main className={styles.screen}>
      <div className={styles.header}>
        <Heading size="2xl">App Settings</Heading>
        <Text color="muted">
          Configure the selected app. Changes apply only to this app and are checked against your
          access before they are saved.
        </Text>
      </div>

      {!canManage && (
        <div className={styles.readOnlyNotice} role="status">
          <Text weight="bold">Read-only access</Text>
          <Text color="muted">
            You can review these settings, but your role cannot change or delete this app.
          </Text>
        </div>
      )}

      <Form
        key={`name-${website?.id}-${nameVersion}`}
        onSubmit={handleNameSubmit}
        error={getErrorMessage(websiteMutation.error)}
        values={{ name: website?.name ?? '' }}
      >
        <SettingSection title="Name" description="Use this field to rename your app.">
          <Column gap="4">
            <FormField label="App name" name="name" rules={{ required: t(labels.required) }}>
              <TextField data-test="settings-name" autoComplete="off" isReadOnly={!canManage} />
            </FormField>
            <FormButtons>
              <Button
                isDisabled={!canManage || websiteMutation.isPending}
                onPress={() => setNameVersion(value => value + 1)}
              >
                {t(labels.cancel)}
              </Button>
              <FormSubmitButton
                data-test="settings-save-name"
                variant="primary"
                isDisabled={!canManage || websiteMutation.isPending}
                isLoading={websiteMutation.isPending}
              >
                Save name
              </FormSubmitButton>
            </FormButtons>
          </Column>
        </SettingSection>
      </Form>

      <Form
        key={`preferences-${websiteId}-${preferenceVersion}-${settingsQuery.dataUpdatedAt}`}
        onSubmit={handlePreferencesSubmit}
        error={getErrorMessage(settingsMutation.error)}
        values={{
          overviewLayout: settings.overviewLayout,
          colorScheme: settings.colorScheme,
          revenueDisplayMode: settings.revenueDisplayMode,
        }}
      >
        {({ watch }) => {
          const selectedColorScheme = watch('colorScheme') as AppColorScheme;

          return (
            <>
              <SettingSection
                title="Overview Layout"
                description="Choose whether Overview presents application or website-oriented data."
              >
                <FormField
                  label="Layout"
                  name="overviewLayout"
                  rules={{ required: t(labels.required) }}
                >
                  <Select isDisabled={!canManage} data-test="settings-overview-layout">
                    <ListItem id="application">Display application related data</ListItem>
                    <ListItem id="website">Show data for a blog or static website</ListItem>
                  </Select>
                </FormField>
              </SettingSection>

              <SettingSection
                title="Color Scheme"
                description="Choose the shared report palette for this app."
              >
                <Column gap="3">
                  <FormField
                    label="Color scheme"
                    name="colorScheme"
                    rules={{ required: t(labels.required) }}
                  >
                    <Select isDisabled={!canManage} data-test="settings-color-scheme">
                      <ListItem id="default">Default</ListItem>
                      <ListItem id="vibrant">Vibrant</ListItem>
                      <ListItem id="high-contrast">High Contrast</ListItem>
                      <ListItem id="emo">Emo</ListItem>
                    </Select>
                  </FormField>
                  <ColorPreview scheme={selectedColorScheme || settings.colorScheme} />
                </Column>
              </SettingSection>

              <SettingSection
                title="Revenue Display Mode"
                description="Choose whether this app exposes RevenueCat-backed revenue reporting."
              >
                <Column gap="4">
                  <FormField
                    label="Display mode"
                    name="revenueDisplayMode"
                    rules={{ required: t(labels.required) }}
                  >
                    <Select isDisabled={!canManage} data-test="settings-revenue-mode">
                      <ListItem id="none">No revenue data</ListItem>
                      <ListItem id="revenuecat">Use RevenueCat for tracking revenue data</ListItem>
                    </Select>
                  </FormField>
                  <FormButtons>
                    <Button
                      isDisabled={!canManage || settingsMutation.isPending}
                      onPress={() => setPreferenceVersion(value => value + 1)}
                    >
                      {t(labels.cancel)}
                    </Button>
                    <FormSubmitButton
                      data-test="settings-save-preferences"
                      variant="primary"
                      isDisabled={!canManage || settingsMutation.isPending}
                      isLoading={settingsMutation.isPending}
                    >
                      Save preferences
                    </FormSubmitButton>
                  </FormButtons>
                </Column>
              </SettingSection>
            </>
          );
        }}
      </Form>

      <Form
        key={`info-${website?.id}-${infoVersion}`}
        onSubmit={handleAppInfoSubmit}
        error={getErrorMessage(websiteMutation.error)}
        values={{ platform, domain: initialDomain }}
      >
        {({ watch, setValue }) => {
          const selectedPlatform = (watch('platform') || platform) as Platform;
          const isWebsite = selectedPlatform === 'website';
          const storeLabel = selectedPlatform === 'android' ? 'Google Play URL' : 'App Store URL';

          const handlePlatformChange = (value: string | number | null) => {
            const nextPlatform = (value || 'website') as Platform;
            setValue('platform', nextPlatform, { shouldDirty: true });
            setValue('domain', '', { shouldDirty: true });
          };

          return (
            <SettingSection
              title="App Information"
              description="Maintain the platform and public destination associated with this app."
            >
              <Column gap="4">
                <FormField
                  label="Platform"
                  name="platform"
                  rules={{ required: t(labels.required) }}
                >
                  <Select
                    value={selectedPlatform}
                    onChange={handlePlatformChange}
                    isDisabled={!canManage}
                    data-test="settings-platform"
                  >
                    <ListItem id="ios">iOS</ListItem>
                    <ListItem id="macos">macOS</ListItem>
                    <ListItem id="android">Android</ListItem>
                    <ListItem id="website">Website</ListItem>
                  </Select>
                </FormField>
                <FormField
                  label={isWebsite ? t(labels.domain) : storeLabel}
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
                    data-test="settings-domain"
                    autoComplete="off"
                    isReadOnly={!canManage}
                    placeholder={
                      isWebsite ? 'example.com' : PLATFORM_STORE_PLACEHOLDERS[selectedPlatform]
                    }
                  />
                </FormField>
                <FormButtons>
                  <Button
                    isDisabled={!canManage || websiteMutation.isPending}
                    onPress={() => setInfoVersion(value => value + 1)}
                  >
                    {t(labels.cancel)}
                  </Button>
                  <FormSubmitButton
                    data-test="settings-save-info"
                    variant="primary"
                    isDisabled={!canManage || websiteMutation.isPending}
                    isLoading={websiteMutation.isPending}
                  >
                    Save app information
                  </FormSubmitButton>
                </FormButtons>
              </Column>
            </SettingSection>
          );
        }}
      </Form>

      <SettingSection
        title="Danger Zone"
        description="Delete this app and all of its analytics data. This action cannot be undone."
        danger
      >
        <Column gap="2">
          <DialogTrigger>
            <Button data-test="settings-delete" variant="danger" isDisabled={!canManage}>
              Delete app
            </Button>
            <Modal>
              <Dialog title="Delete app" style={{ width: 400 }}>
                {({ close }) => (
                  <WebsiteDeleteForm websiteId={websiteId} onSave={handleDelete} onClose={close} />
                )}
              </Dialog>
            </Modal>
          </DialogTrigger>
          <Text color="muted">
            Confirmation requires typing DELETE. Closing the dialog leaves the app unchanged.
          </Text>
        </Column>
      </SettingSection>
    </main>
  );
}
