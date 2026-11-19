'use client';
import { usePathname } from 'next/navigation';
import type { ComponentType, ReactNode } from 'react';
import { LinkEditPage } from '@/app/(main)/links/[linkId]/edit/LinkEditPage';
import { LinkPage } from '@/app/(main)/links/[linkId]/LinkPage';
import { PixelEditPage } from '@/app/(main)/pixels/[pixelId]/edit/PixelEditPage';
import { PixelPage } from '@/app/(main)/pixels/[pixelId]/PixelPage';
import { AttributionPage } from '@/app/(main)/websites/[websiteId]/(reports)/attribution/AttributionPage';
import { BreakdownPage } from '@/app/(main)/websites/[websiteId]/(reports)/breakdown/BreakdownPage';
import { FunnelsPage } from '@/app/(main)/websites/[websiteId]/(reports)/funnels/FunnelsPage';
import { GoalsPage } from '@/app/(main)/websites/[websiteId]/(reports)/goals/GoalsPage';
import { JourneysPage } from '@/app/(main)/websites/[websiteId]/(reports)/journeys/JourneysPage';
import { RetentionPage } from '@/app/(main)/websites/[websiteId]/(reports)/retention/RetentionPage';
import { RevenuePage } from '@/app/(main)/websites/[websiteId]/(reports)/revenue/RevenuePage';
import { UTMPage } from '@/app/(main)/websites/[websiteId]/(reports)/utm/UTMPage';
import { CohortsPage } from '@/app/(main)/websites/[websiteId]/cohorts/CohortsPage';
import { ComparePage } from '@/app/(main)/websites/[websiteId]/compare/ComparePage';
import { EventsPage } from '@/app/(main)/websites/[websiteId]/events/EventsPage';
import { RealtimePage } from '@/app/(main)/websites/[websiteId]/realtime/RealtimePage';
import { SegmentsPage } from '@/app/(main)/websites/[websiteId]/segments/SegmentsPage';
import { SessionProfile } from '@/app/(main)/websites/[websiteId]/sessions/SessionProfile';
import { SessionsPage } from '@/app/(main)/websites/[websiteId]/sessions/SessionsPage';
import { SettingsPage } from '@/app/(main)/websites/[websiteId]/settings/SettingsPage';
import { WebsiteLayout } from '@/app/(main)/websites/[websiteId]/WebsiteLayout';
import { WebsitePage } from '@/app/(main)/websites/[websiteId]/WebsitePage';
import NotFound from '@/app/not-found';
import { AdminLayout } from './admin/AdminLayout';
import { AdminTeamPage } from './admin/teams/[teamId]/AdminTeamPage';
import { UserPage } from './admin/users/[userId]/UserPage';
import { BoardDesignPage } from './boards/[boardId]/BoardEditPage';
import { BoardViewPage } from './boards/[boardId]/BoardViewPage';
import { BoardEditPage } from './boards/[boardId]/edit/BoardEditPage';
import { TestConsolePage } from './console/[websiteId]/TestConsolePage';
import { SettingsLayout } from './settings/SettingsLayout';
import { TeamSettingsPage } from './settings/teams/[teamId]/TeamSettingsPage';
import { WebsiteSettingsPage } from './settings/websites/[websiteId]/WebsiteSettingsPage';
import { HeatmapsPage } from './websites/[websiteId]/(reports)/heatmaps/HeatmapsPage';
import { PerformancePage } from './websites/[websiteId]/(reports)/performance/PerformancePage';
import { ErrorIssuePage } from './websites/[websiteId]/errors/[issueId]/ErrorIssuePage';
import { ErrorsPage } from './websites/[websiteId]/errors/ErrorsPage';
import { GoogleAdsPage } from './websites/[websiteId]/google-ads/GoogleAdsPage';
import { ReplayPlayback } from './websites/[websiteId]/replays/[replayId]/ReplayPlayback';
import { ReplaysPage } from './websites/[websiteId]/replays/ReplaysPage';
import { SearchConsolePage } from './websites/[websiteId]/search-console/SearchConsolePage';

const WEBSITE_PAGES: Record<string, ComponentType<{ websiteId: string }>> = {
  '': WebsitePage,
  events: EventsPage,
  sessions: SessionsPage,
  realtime: RealtimePage,
  settings: SettingsPage,
  compare: ComparePage,
  segments: SegmentsPage,
  cohorts: CohortsPage,
  attribution: AttributionPage,
  breakdown: BreakdownPage,
  funnels: FunnelsPage,
  goals: GoalsPage,
  journeys: JourneysPage,
  retention: RetentionPage,
  revenue: RevenuePage,
  utm: UTMPage,
  errors: ErrorsPage,
  replays: ReplaysPage,
  performance: PerformancePage,
  heatmaps: HeatmapsPage,
  'google-ads': GoogleAdsPage,
  'search-console': SearchConsolePage,
};

// Only the prerendered websites shell dispatches by URL. Next-rendered pages keep their layouts and modal slots.
export function SpaRouteView({ children }: { children: ReactNode }) {
  const normalizedPath = usePathname()
    .replace(/^\/teams\/[0-9a-f-]{36}(?=\/|$)/i, '')
    .replace(/\/+$/, '');
  if (!normalizedPath || normalizedPath === '/websites') return children;
  const websiteMatch = normalizedPath.match(/^\/websites\/([0-9a-f-]{36})(?:\/(.*))?$/i);

  if (websiteMatch) {
    const websiteId = websiteMatch[1];
    const subpath = (websiteMatch[2] || '').replace(/\/+$/, '');
    const [section, id, extra] = subpath.split('/');
    let page: ReactNode;
    if (extra === undefined && id && section === 'sessions')
      page = <SessionProfile websiteId={websiteId} sessionId={id} />;
    else if (extra === undefined && id && section === 'replays')
      page = <ReplayPlayback websiteId={websiteId} replayId={id} />;
    else if (extra === undefined && id && section === 'errors')
      page = <ErrorIssuePage websiteId={websiteId} issueId={id} />;
    else if (Object.hasOwn(WEBSITE_PAGES, subpath)) {
      const Page = WEBSITE_PAGES[subpath];
      page = <Page websiteId={websiteId} />;
    } else return <NotFound />;
    return <WebsiteLayout websiteId={websiteId}>{page}</WebsiteLayout>;
  }

  const linkMatch = normalizedPath.match(/^\/links\/([0-9a-f-]{36})(?:\/(edit))?$/i);
  if (linkMatch) {
    return linkMatch[2] ? (
      <LinkEditPage linkId={linkMatch[1]} />
    ) : (
      <LinkPage linkId={linkMatch[1]} />
    );
  }

  const pixelMatch = normalizedPath.match(/^\/pixels\/([0-9a-f-]{36})(?:\/(edit))?$/i);
  if (pixelMatch) {
    return pixelMatch[2] ? (
      <PixelEditPage pixelId={pixelMatch[1]} />
    ) : (
      <PixelPage pixelId={pixelMatch[1]} />
    );
  }

  const board = normalizedPath.match(/^\/boards\/([0-9a-f-]{36})(?:\/(edit|design))?$/i);
  if (board) {
    const Page =
      board[2] === 'edit' ? BoardEditPage : board[2] === 'design' ? BoardDesignPage : BoardViewPage;
    return <Page boardId={board[1]} />;
  }
  const details = normalizedPath.match(
    /^\/(admin\/(?:users|teams|websites)|settings\/(?:teams|websites)|console)\/([0-9a-f-]{36})$/i,
  );
  if (details) {
    const [, route, id] = details;
    switch (route) {
      case 'console':
        return <TestConsolePage websiteId={id} />;
      case 'admin/users':
        return (
          <AdminLayout>
            <UserPage userId={id} />
          </AdminLayout>
        );
      case 'admin/teams':
        return (
          <AdminLayout>
            <AdminTeamPage teamId={id} />
          </AdminLayout>
        );
      case 'admin/websites':
        return (
          <AdminLayout>
            <WebsiteSettingsPage websiteId={id} />
          </AdminLayout>
        );
      case 'settings/teams':
        return (
          <SettingsLayout>
            <TeamSettingsPage teamId={id} />
          </SettingsLayout>
        );
      case 'settings/websites':
        return (
          <SettingsLayout>
            <WebsiteSettingsPage websiteId={id} />
          </SettingsLayout>
        );
    }
  }
  return <NotFound />;
}
