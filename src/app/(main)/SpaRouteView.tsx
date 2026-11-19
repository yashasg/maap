'use client';
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
};

export function SpaRouteView({ pathname, children }: { pathname: string; children: ReactNode }) {
  const normalizedPath = pathname.replace(/^\/teams\/[0-9a-f-]{36}/i, '') || '/';
  const websiteMatch = normalizedPath.match(/^\/websites\/([0-9a-f-]{36})(?:\/(.*))?$/i);

  if (websiteMatch) {
    const websiteId = websiteMatch[1];
    const subpath = (websiteMatch[2] || '').replace(/\/+$/, '');
    const sessionMatch = subpath.match(/^sessions\/([^/]+)$/);
    const Page = WEBSITE_PAGES[subpath] ?? WebsitePage;

    return (
      <WebsiteLayout websiteId={websiteId}>
        {sessionMatch ? (
          <SessionProfile websiteId={websiteId} sessionId={sessionMatch[1]} />
        ) : (
          <Page websiteId={websiteId} />
        )}
      </WebsiteLayout>
    );
  }

  const linkMatch = normalizedPath.match(/^\/links\/([0-9a-f-]{36})(?:\/(edit))?$/i);
  if (linkMatch) {
    return linkMatch[2] ? <LinkEditPage linkId={linkMatch[1]} /> : <LinkPage linkId={linkMatch[1]} />;
  }

  const pixelMatch = normalizedPath.match(/^\/pixels\/([0-9a-f-]{36})(?:\/(edit))?$/i);
  if (pixelMatch) {
    return pixelMatch[2] ? <PixelEditPage pixelId={pixelMatch[1]} /> : <PixelPage pixelId={pixelMatch[1]} />;
  }

  return <>{children}</>;
}
