'use client';
import { Icon, Row } from '@umami/react-zen';
import { useEffect, useRef } from 'react';
import Link from '@/components/common/Link';
import { useNavigation } from '@/components/hooks';
import { Minus, Search } from '@/components/icons';
import { BoardSelect } from '@/components/input/BoardSelect';
import { LinkSelect } from '@/components/input/LinkSelect';
import { PixelSelect } from '@/components/input/PixelSelect';
import { TeamsButton } from '@/components/input/TeamsButton';
import { UserButton } from '@/components/input/UserButton';
import { WebsiteSelect } from '@/components/input/WebsiteSelect';
import { Logo } from '@/components/svg';
import workspaceStyles from './TopNav.module.css';
import styles from './websites/[websiteId]/app-metrics/AppReports.module.css';

export function TopNav({
  appReports = false,
  workspaceHome = false,
}: {
  appReports?: boolean;
  workspaceHome?: boolean;
}) {
  const { websiteId, linkId, pixelId, boardId, teamId, query, router, updateParams, renderUrl } =
    useNavigation();
  const searchRef = useRef<HTMLInputElement>(null);
  const standalone = appReports || workspaceHome;

  const navigateToEntity = (basePath: string, value: string | number | null) => {
    if (value === null || value === undefined || value === '') {
      return;
    }

    router.push(
      renderUrl(
        `${basePath}/${value}`,
        workspaceHome ? { search: undefined, page: undefined } : false,
      ),
    );
  };

  const handleWebsiteChange = (value: string | number | null) => {
    navigateToEntity('/websites', value);
  };

  const handleLinkChange = (value: string | number | null) => {
    navigateToEntity('/links', value);
  };

  const handlePixelChange = (value: string | number | null) => {
    navigateToEntity('/pixels', value);
  };

  const handleBoardChange = (value: string | number | null) => {
    navigateToEntity('/boards', value);
  };

  useEffect(() => {
    if (!workspaceHome) {
      return;
    }

    const handleShortcut = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLocaleLowerCase() === 'k') {
        event.preventDefault();
        searchRef.current?.focus();
      }
    };

    window.addEventListener('keydown', handleShortcut);
    return () => window.removeEventListener('keydown', handleShortcut);
  }, [workspaceHome]);

  return (
    <Row
      position="sticky"
      top="0"
      alignItems="center"
      justifyContent="flex-start"
      paddingY="2"
      paddingX="3"
      paddingRight="5"
      width="100%"
      zIndex={100}
      backgroundColor="surface-raised"
      className={
        appReports ? styles.topBar : workspaceHome ? workspaceStyles.workspaceTopBar : undefined
      }
    >
      {standalone && (
        <Link
          href={renderUrl('/websites', false)}
          className={appReports ? styles.brand : workspaceStyles.brand}
          aria-label="Maap"
        >
          <Logo aria-hidden />
        </Link>
      )}
      <Row
        alignItems="center"
        className={
          appReports ? styles.pickers : workspaceHome ? workspaceStyles.pickers : undefined
        }
      >
        <TeamsButton compact={standalone} />
        {(workspaceHome || websiteId || linkId || pixelId || boardId) && (
          <>
            <Icon size="sm" color="muted" rotate={90} style={{ opacity: 0.7, margin: '0 6px' }}>
              <Minus />
            </Icon>
            {(workspaceHome || websiteId) && (
              <div className={workspaceHome ? workspaceStyles.workspaceAppPicker : undefined}>
                <WebsiteSelect
                  websiteId={websiteId}
                  teamId={teamId}
                  includeTeams={!teamId}
                  onChange={handleWebsiteChange}
                  placeholder={workspaceHome ? 'Select an app' : undefined}
                  buttonProps={{
                    style: standalone
                      ? { minWidth: 0, maxWidth: 'min(28vw, 200px)' }
                      : { minWidth: 200, maxWidth: 200 },
                  }}
                />
              </div>
            )}
            {linkId && (
              <LinkSelect
                linkId={linkId}
                teamId={teamId}
                onChange={handleLinkChange}
                buttonProps={{
                  className:
                    'border-transparent bg-transparent shadow-none hover:border-transparent hover:bg-interactive active:bg-interactive-hover',
                  style: { minHeight: 40, minWidth: 200, maxWidth: 200 },
                }}
              />
            )}
            {pixelId && (
              <PixelSelect
                pixelId={pixelId}
                teamId={teamId}
                onChange={handlePixelChange}
                buttonProps={{
                  className:
                    'border-transparent bg-transparent shadow-none hover:border-transparent hover:bg-interactive active:bg-interactive-hover',
                  style: { minHeight: 40, minWidth: 200, maxWidth: 200 },
                }}
              />
            )}
            {boardId && (
              <BoardSelect
                boardId={boardId}
                teamId={teamId}
                onChange={handleBoardChange}
                buttonProps={{
                  className:
                    'border-transparent bg-transparent shadow-none hover:border-transparent hover:bg-interactive active:bg-interactive-hover',
                  style: { minHeight: 40, minWidth: 200, maxWidth: 200 },
                }}
              />
            )}
          </>
        )}
      </Row>
      {workspaceHome && (
        <>
          <span className={workspaceStyles.divider} aria-hidden />
          <div className={workspaceStyles.mobileAppPicker}>
            <WebsiteSelect
              websiteId={websiteId}
              teamId={teamId}
              includeTeams={!teamId}
              onChange={handleWebsiteChange}
              placeholder="Select an app"
              buttonProps={{
                style: { width: '100%', minWidth: 0, maxWidth: '100%' },
              }}
            />
          </div>
          <label className={workspaceStyles.search}>
            <Search size={15} aria-hidden />
            <input
              ref={searchRef}
              type="search"
              aria-label="Search apps"
              placeholder="Search apps"
              value={String(query.search || '')}
              onChange={event =>
                router.replace(
                  updateParams({
                    search: event.target.value || undefined,
                    page: undefined,
                  }),
                )
              }
            />
            <kbd className={workspaceStyles.shortcut}>⌘ K</kbd>
          </label>
        </>
      )}
      {standalone ? (
        <div className={appReports ? styles.account : workspaceStyles.account}>
          <UserButton showText={false} />
        </div>
      ) : (
        <div
          style={{
            position: 'absolute',
            bottom: -16,
            left: 0,
            right: 0,
            height: 16,
            background: 'linear-gradient(to bottom, var(--zen-surface-raised), transparent)',
            pointerEvents: 'none',
          }}
        />
      )}
    </Row>
  );
}
