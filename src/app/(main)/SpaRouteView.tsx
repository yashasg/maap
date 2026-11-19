'use client';
import type { ReactNode } from 'react';
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

export function SpaRouteView({ pathname, children }: { pathname: string; children: ReactNode }) {
  const normalizedPath = pathname.replace(/^\/teams\/[0-9a-f-]{36}/i, '') || '/';
  const websiteMatch = normalizedPath.match(/^\/websites\/([0-9a-f-]{36})(?:\/(.*))?$/i);

  if (websiteMatch) {
    const websiteId = websiteMatch[1];
    const subpath = (websiteMatch[2] || '').replace(/\/+$/, '');
    let page: ReactNode;

    switch (subpath) {
      case '':
        page = <WebsitePage websiteId={websiteId} />;
        break;
      case 'events':
        page = <EventsPage websiteId={websiteId} />;
        break;
      case 'sessions':
        page = <SessionsPage websiteId={websiteId} />;
        break;
      case 'realtime':
        page = <RealtimePage websiteId={websiteId} />;
        break;
      case 'settings':
        page = <SettingsPage websiteId={websiteId} />;
        break;
      case 'compare':
        page = <ComparePage websiteId={websiteId} />;
        break;
      case 'segments':
        page = <SegmentsPage websiteId={websiteId} />;
        break;
      case 'cohorts':
        page = <CohortsPage websiteId={websiteId} />;
        break;
      case 'attribution':
        page = <AttributionPage websiteId={websiteId} />;
        break;
      case 'breakdown':
        page = <BreakdownPage websiteId={websiteId} />;
        break;
      case 'funnels':
        page = <FunnelsPage websiteId={websiteId} />;
        break;
      case 'goals':
        page = <GoalsPage websiteId={websiteId} />;
        break;
      case 'journeys':
        page = <JourneysPage websiteId={websiteId} />;
        break;
      case 'retention':
        page = <RetentionPage websiteId={websiteId} />;
        break;
      case 'revenue':
        page = <RevenuePage websiteId={websiteId} />;
        break;
      case 'utm':
        page = <UTMPage websiteId={websiteId} />;
        break;
      default: {
        const sessionMatch = subpath.match(/^sessions\/([^/]+)$/);
        page = sessionMatch ? (
          <SessionProfile websiteId={websiteId} sessionId={sessionMatch[1]} />
        ) : (
          <WebsitePage websiteId={websiteId} />
        );
      }
    }

    return <WebsiteLayout websiteId={websiteId}>{page}</WebsiteLayout>;
  }

  const linkMatch = normalizedPath.match(/^\/links\/([0-9a-f-]{36})(?:\/(edit))?$/i);

  if (linkMatch) {
    const linkId = linkMatch[1];
    return linkMatch[2] ? <LinkEditPage linkId={linkId} /> : <LinkPage linkId={linkId} />;
  }

  const pixelMatch = normalizedPath.match(/^\/pixels\/([0-9a-f-]{36})(?:\/(edit))?$/i);

  if (pixelMatch) {
    const pixelId = pixelMatch[1];
    return pixelMatch[2] ? <PixelEditPage pixelId={pixelId} /> : <PixelPage pixelId={pixelId} />;
  }

  return <>{children}</>;
}
