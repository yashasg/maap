'use client';

import { Button, Icon, ListItem, Select } from '@umami/react-zen';
import { useTranslations } from 'next-intl';
import { type FormEvent, useMemo, useState } from 'react';
import Link from '@/components/common/Link';
import { LinkButton } from '@/components/common/LinkButton';
import {
  useDateParameters,
  useLoginQuery,
  useNavigation,
  useTeamMembersQuery,
  useTeamQuery,
  useTimezone,
  useUserWebsitesQuery,
} from '@/components/hooks';
import {
  Apple,
  ArrowRight,
  ChevronDown,
  CircleAlert,
  FolderPlus,
  Globe,
  Pencil,
  Smartphone,
  Users,
} from '@/components/icons';
import { DialogButton } from '@/components/input/DialogButton';
import { WebsiteDateFilter } from '@/components/input/WebsiteDateFilter';
import type { Website } from '@/generated/prisma/client';
import { ROLES } from '@/lib/constants';
import { decodePunycodeDomain } from '@/lib/format';
import {
  useWorkspaceCardMetrics,
  useWorkspaceFolders,
  useWorkspaceHeartbeat,
} from './useWorkspaceHome';
import { WebsiteAddButton } from './WebsiteAddButton';
import { detectPlatform, formatPlatformDomain } from './WebsiteAddForm';
import styles from './WorkspaceHome.module.css';
import { WorkspacePoll } from './WorkspacePoll';

const timezones = ['UTC', ...Intl.supportedValuesOf('timeZone').filter(value => value !== 'UTC')];

type WebsiteListResponse = {
  data: Website[];
  count: number;
};

function getPlatform(website: Website) {
  const platform = detectPlatform(website.domain);

  if (platform === 'ios' || platform === 'macos') {
    return { label: platform === 'ios' ? 'iOS' : 'macOS', icon: <Apple /> };
  }

  if (platform === 'android') {
    return { label: 'Android', icon: <Smartphone /> };
  }

  return {
    label: formatPlatformDomain(website.domain) || decodePunycodeDomain(website.domain),
    icon: <Globe />,
  };
}

export function WebsitesPage() {
  const t = useTranslations('workspaceHome');
  const { user } = useLoginQuery();
  const { teamId, query, router, updateParams, renderUrl } = useNavigation();
  const { data: members } = useTeamMembersQuery(teamId);
  const { data: team } = useTeamQuery(teamId);
  const { startAt, endAt } = useDateParameters();
  const { timezone, saveTimezone } = useTimezone();
  const [timezoneSearch, setTimezoneSearch] = useState('');
  const testTag = query.tag === 'test' || query.tag === 'eq.test';
  const testMode = query.testMode === 'true' || testTag;
  const search = String(query.search || '')
    .trim()
    .toLocaleLowerCase();
  const canManage =
    user.role !== ROLES.viewOnly &&
    (teamId
      ? members?.data?.some(
          member => member.userId === user.id && member.role !== ROLES.teamViewOnly,
        )
      : true);
  const websitesQuery = useUserWebsitesQuery(
    { userId: user.id, teamId },
    { pageSize: 1000, orderBy: 'name' },
  );
  const response = websitesQuery.data as WebsiteListResponse | undefined;
  const websites = response?.data ?? [];
  const visibleWebsites = useMemo(
    () =>
      search
        ? websites.filter(website =>
            `${website.name} ${website.domain}`.toLocaleLowerCase().includes(search),
          )
        : websites,
    [search, websites],
  );
  const websiteIds = useMemo(() => websites.map(website => website.id), [websites]);
  const metricsQuery = useWorkspaceCardMetrics({
    websiteIds,
    startAt,
    endAt,
    testMode,
    teamId,
  });
  const foldersQuery = useWorkspaceFolders(teamId);
  const metrics = useMemo(
    () =>
      new Map(
        (metricsQuery.data?.data ?? []).map(metric => [
          metric.websiteId,
          { users: metric.users, errors: metric.errors },
        ]),
      ),
    [metricsQuery.data],
  );
  const assignments = useMemo(
    () => new Map(foldersQuery.assignments.map(item => [item.websiteId, item.folderId])),
    [foldersQuery.assignments],
  );
  const visibleIds = useMemo(
    () => new Set(visibleWebsites.map(website => website.id)),
    [visibleWebsites],
  );
  const unfiled = visibleWebsites.filter(website => !assignments.has(website.id));
  const workspaceName = teamId ? team?.name || t('workspace') : user.username;
  const filteredTimezones = timezoneSearch
    ? timezones.filter(item =>
        item.toLocaleLowerCase().includes(timezoneSearch.toLocaleLowerCase()),
      )
    : timezones;

  const handleTestMode = () => {
    router.push(
      updateParams({
        testMode: testMode ? undefined : 'true',
        tag: testTag ? undefined : query.tag,
        page: undefined,
      }),
    );
  };

  const handleMove = async (websiteId: string, folderId: string) => {
    try {
      await foldersQuery.moveWebsite.mutateAsync({
        websiteId,
        folderId: folderId || null,
      });
    } catch {
      // The mutation error is rendered beside the workspace controls.
    }
  };

  return (
    <main className={styles.page}>
      {testMode && (
        <div className={styles.testBanner} role="status">
          {t('test-data')}
          <span>{t('test-banner')}</span>
        </div>
      )}
      <div className={styles.container}>
        <header className={styles.header}>
          <div>
            <h1 className={styles.title}>{workspaceName}</h1>
            <div className={styles.meta}>
              <span className={styles.metaItem}>
                <Icon size="sm">
                  <Globe />
                </Icon>
                {t('app-count', { count: response?.count ?? websites.length })}
              </span>
              {metricsQuery.isFetching && <span>{t('updating-metrics')}</span>}
            </div>
          </div>
          {canManage && (
            <div className={styles.actions}>
              <WebsiteAddButton teamId={teamId} label={t('create-app')} />
              <FolderCreateButton
                onCreate={name => foldersQuery.createFolder.mutateAsync(name)}
                isPending={foldersQuery.createFolder.isPending}
                error={foldersQuery.createFolder.error}
              />
            </div>
          )}
        </header>

        <div className={styles.controls} aria-label={t('report-controls')}>
          <WebsiteDateFilter showButtons={false} showAllTime={false} />
          <Select
            className={styles.timezone}
            aria-label={t('timezone')}
            value={timezone}
            allowSearch
            maxHeight={360}
            onSearch={setTimezoneSearch}
            onOpenChange={open => open && setTimezoneSearch('')}
            onChange={(value: string | number) => saveTimezone(String(value))}
          >
            {filteredTimezones.map(item => (
              <ListItem key={item} id={item}>
                {item}
              </ListItem>
            ))}
          </Select>
          <button
            type="button"
            className={styles.testModeButton}
            aria-pressed={testMode}
            onClick={handleTestMode}
          >
            {testMode ? t('test-mode-on') : `+ ${t('test-mode')}`}
          </button>
        </div>
        {(foldersQuery.isError || foldersQuery.moveWebsite.error || metricsQuery.isError) && (
          <div className={styles.errorNotice} role="alert">
            <CircleAlert size={16} aria-hidden />
            <span>
              {foldersQuery.moveWebsite.error
                ? t('move-error')
                : foldersQuery.isError
                  ? t('folders-error')
                  : t('metrics-error')}
            </span>
            {(foldersQuery.isError || metricsQuery.isError) && (
              <button
                type="button"
                onClick={() => {
                  if (foldersQuery.isError) {
                    void foldersQuery.refetch();
                  }
                  if (metricsQuery.isError) {
                    void metricsQuery.refetch();
                  }
                }}
              >
                {t('try-again')}
              </button>
            )}
          </div>
        )}

        <div className={styles.layout}>
          <section className={styles.apps} aria-label={t('apps')}>
            {websitesQuery.isLoading ? (
              <div className={styles.empty} role="status">
                {t('loading-apps')}
              </div>
            ) : websitesQuery.isError ? (
              <div className={styles.empty} role="alert">
                <p>{t('apps-error')}</p>
                <button type="button" onClick={() => void websitesQuery.refetch()}>
                  {t('try-again')}
                </button>
              </div>
            ) : visibleWebsites.length === 0 ? (
              <div className={styles.empty}>
                {search ? t('no-search-results', { search: query.search }) : t('no-apps')}
              </div>
            ) : (
              <>
                {unfiled.length > 0 && (
                  <FolderSection
                    label={foldersQuery.folders.length > 0 ? t('unfiled') : t('apps')}
                    websites={unfiled}
                    folders={foldersQuery.folders}
                    assignments={assignments}
                    metrics={metrics}
                    canManage={canManage}
                    movePending={foldersQuery.moveWebsite.isPending}
                    onMove={handleMove}
                    renderUrl={renderUrl}
                  />
                )}
                {foldersQuery.folders.map(folder => {
                  const folderWebsites = websites.filter(
                    website =>
                      assignments.get(website.id) === folder.id && visibleIds.has(website.id),
                  );

                  if (search && folderWebsites.length === 0) {
                    return null;
                  }

                  return (
                    <FolderSection
                      key={folder.id}
                      label={folder.name}
                      websites={folderWebsites}
                      folders={foldersQuery.folders}
                      assignments={assignments}
                      metrics={metrics}
                      canManage={canManage}
                      movePending={foldersQuery.moveWebsite.isPending}
                      onMove={handleMove}
                      renderUrl={renderUrl}
                      collapsible
                    />
                  );
                })}
              </>
            )}
          </section>
          <WorkspacePoll />
        </div>

        <WorkspaceFooter />
      </div>
    </main>
  );
}

function FolderSection({
  label,
  websites,
  folders,
  assignments,
  metrics,
  canManage,
  movePending,
  onMove,
  renderUrl,
  collapsible = false,
}: {
  label: string;
  websites: Website[];
  folders: { id: string; name: string }[];
  assignments: Map<string, string>;
  metrics: Map<string, { users: number | null; errors: number | null }>;
  canManage: boolean;
  movePending: boolean;
  onMove: (websiteId: string, folderId: string) => Promise<void>;
  renderUrl: (path: string, params?: Record<string, string | number | undefined> | false) => string;
  collapsible?: boolean;
}) {
  const content =
    websites.length > 0 ? (
      <div className={styles.cardGrid}>
        {websites.map(website => (
          <WebsiteCard
            key={website.id}
            website={website}
            folders={folders}
            folderId={assignments.get(website.id)}
            metric={metrics.get(website.id)}
            canManage={canManage}
            movePending={movePending}
            onMove={onMove}
            renderUrl={renderUrl}
          />
        ))}
      </div>
    ) : (
      <FolderEmptyState />
    );

  if (!collapsible) {
    return (
      <section className={styles.folder}>
        <FolderHeading label={label} count={websites.length} />
        {content}
      </section>
    );
  }

  return (
    <details className={styles.folder} open>
      <summary>
        <ChevronDown size={16} aria-hidden />
        {label}
        <span className={styles.folderCount}>{websites.length}</span>
      </summary>
      {content}
    </details>
  );
}

function FolderHeading({ label, count }: { label: string; count: number }) {
  return (
    <div>
      <h2 style={{ display: 'inline', margin: 0, fontSize: '1rem' }}>{label}</h2>{' '}
      <span className={styles.folderCount}>{count}</span>
    </div>
  );
}

function FolderEmptyState() {
  const t = useTranslations('workspaceHome');
  return <div className={styles.empty}>{t('empty-folder')}</div>;
}

function WebsiteCard({
  website,
  folders,
  folderId,
  metric,
  canManage,
  movePending,
  onMove,
  renderUrl,
}: {
  website: Website;
  folders: { id: string; name: string }[];
  folderId?: string;
  metric?: { users: number | null; errors: number | null };
  canManage: boolean;
  movePending: boolean;
  onMove: (websiteId: string, folderId: string) => Promise<void>;
  renderUrl: (path: string, params?: Record<string, string | number | undefined> | false) => string;
}) {
  const t = useTranslations('workspaceHome');
  const platform = getPlatform(website);
  const formatNumber = (value: number | null | undefined) =>
    value == null ? '—' : new Intl.NumberFormat().format(value);
  const appUrl = renderUrl(`/websites/${website.id}`, {
    search: undefined,
    page: undefined,
  });

  return (
    <article
      className={styles.card}
      data-test="website-card"
      aria-label={t('app-card-label', { name: website.name })}
    >
      <div className={styles.cardHeader}>
        <div className={styles.appIdentity}>
          <Icon size="sm">{platform.icon}</Icon>
          <Link href={appUrl}>{website.name}</Link>
        </div>
        <div className={styles.cardHeaderActions}>
          <span className={styles.platform} title={website.domain}>
            {platform.label}
          </span>
          {canManage && (
            <select
              className={styles.nativeSelect}
              aria-label={t('move-app', { name: website.name })}
              value={folderId || ''}
              disabled={movePending}
              onChange={event => void onMove(website.id, event.target.value)}
            >
              <option value="">{t('unfiled')}</option>
              {folders.map(folder => (
                <option key={folder.id} value={folder.id}>
                  {folder.name}
                </option>
              ))}
            </select>
          )}
        </div>
      </div>
      <div className={styles.metricRow}>
        <div className={styles.metric}>
          <span className={styles.metricLabel}>
            <Users size={13} aria-hidden />
            {t('users')}
          </span>
          <span className={styles.metricValue}>{formatNumber(metric?.users)}</span>
        </div>
        <div className={styles.metric}>
          <span className={styles.metricLabel}>
            <CircleAlert size={13} aria-hidden />
            {t('errors')}
          </span>
          <span className={styles.metricValue} data-error={!!metric?.errors}>
            {formatNumber(metric?.errors)}
          </span>
        </div>
      </div>
      <div className={styles.cardActions}>
        <span />
        <div className={styles.cardLinks}>
          {canManage && (
            <LinkButton
              href={renderUrl(`/websites/${website.id}/settings`)}
              variant="quiet"
              aria-label={t('edit-app', { name: website.name })}
              data-test="link-button-edit"
            >
              <Icon size="sm">
                <Pencil />
              </Icon>
            </LinkButton>
          )}
          <LinkButton
            href={appUrl}
            variant="quiet"
            aria-label={t('open-app', { name: website.name })}
          >
            <Icon size="sm">
              <ArrowRight />
            </Icon>
          </LinkButton>
        </div>
      </div>
    </article>
  );
}

function FolderCreateButton({
  onCreate,
  isPending,
  error,
}: {
  onCreate: (name: string) => Promise<unknown>;
  isPending: boolean;
  error: Error | null;
}) {
  const t = useTranslations('workspaceHome');
  const [isOpen, setIsOpen] = useState(false);
  const [name, setName] = useState('');

  const handleSubmit = async (event: FormEvent) => {
    event.preventDefault();
    const trimmed = name.trim();

    if (!trimmed) {
      return;
    }

    try {
      await onCreate(trimmed);
      setName('');
      setIsOpen(false);
    } catch {
      // The mutation error is rendered in the dialog.
    }
  };

  return (
    <>
      <Button onPress={() => setIsOpen(true)}>
        <Icon size="sm">
          <FolderPlus />
        </Icon>
        {t('new-folder')}
      </Button>
      <DialogButton title={t('new-folder')} width="400px" isOpen={isOpen} onOpenChange={setIsOpen}>
        {({ close }) => (
          <form className={styles.folderForm} onSubmit={handleSubmit}>
            <label>
              {t('folder-name')}
              <input
                className={styles.folderInput}
                value={name}
                maxLength={100}
                autoFocus
                autoComplete="off"
                onChange={event => setName(event.target.value)}
              />
            </label>
            {error && (
              <p className={styles.formError} role="alert">
                {t('folder-error')}
              </p>
            )}
            <div className={styles.formActions}>
              <Button onPress={close} isDisabled={isPending}>
                {t('cancel')}
              </Button>
              <Button variant="primary" type="submit" isDisabled={isPending || !name.trim()}>
                {isPending ? t('creating') : t('create-folder')}
              </Button>
            </div>
          </form>
        )}
      </DialogButton>
    </>
  );
}

function WorkspaceFooter() {
  const t = useTranslations('workspaceHome');
  const heartbeat = useWorkspaceHeartbeat();
  const status = heartbeat.isError ? 'offline' : heartbeat.data?.ok ? 'online' : 'checking';

  return (
    <footer className={styles.footer}>
      <span>© {new Date().getFullYear()} Maap</span>
      <a href="https://github.com/yashasg/maap" target="_blank" rel="noreferrer">
        {t('source')}
      </a>
      <a href="https://github.com/yashasg/maap/issues" target="_blank" rel="noreferrer">
        {t('issues')}
      </a>
      <span className={styles.status}>
        <span className={styles.statusDot} data-state={status} aria-hidden />
        {t(`service-${status}`)}
      </span>
    </footer>
  );
}
