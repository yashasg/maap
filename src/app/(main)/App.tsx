'use client';
import { Column, Grid, Loading, Row, Text } from '@umami/react-zen';
import Script from 'next/script';
import { useEffect } from 'react';
import { MobileNav } from '@/app/(main)/MobileNav';
import { SideNav } from '@/app/(main)/SideNav';
import { TopNav } from '@/app/(main)/TopNav';
import {
  isAppReportPath,
  isNativeApp,
} from '@/app/(main)/websites/[websiteId]/app-metrics/report-layout';
import {
  useConfig,
  useLoginQuery,
  useNavigation,
  useSubscription,
  useTeamQuery,
  useTwoFactorStatusQuery,
} from '@/components/hooks';
import { useWebsiteQuery } from '@/components/hooks/queries/useWebsiteQuery';
import { PasswordChangeRequiredModal } from '@/components/modals/PasswordChangeRequiredModal';
import { TwoFactorSetupModal } from '@/components/modals/TwoFactorSetupModal';
import { LAST_TEAM_CONFIG } from '@/lib/constants';
import { removeItem, setItem } from '@/lib/storage';
import { UpdateNotice } from './UpdateNotice';

export function App({ children }) {
  const { user, isLoading, error } = useLoginQuery();
  const config = useConfig();
  const { pathname, router, teamId, websiteId } = useNavigation();
  const { billingStatus, isOwner, isLoading: isSubscriptionLoading } = useSubscription(teamId);
  const isBillingPastDue = config.cloudMode && billingStatus === 'past_due';
  const isBlockedTeam = !!teamId && isBillingPastDue;
  const { isLoading: isTeamLoading, error: teamError } = useTeamQuery(teamId, {
    enabled: !!teamId && (!config.cloudMode || (!isSubscriptionLoading && !isBlockedTeam)),
  });
  const { data: twoFactorStatus } = useTwoFactorStatusQuery(!!user && !config.cloudMode);
  const needsTwoFactorSetup = !!(twoFactorStatus?.isRequired && !twoFactorStatus?.isEnabled);
  const passwordChangeRequired = !!user?.passwordChangeRequired && !config.cloudMode;
  const accessBlocked = needsTwoFactorSetup || passwordChangeRequired;
  const reportPath = isAppReportPath(pathname, websiteId);
  const { data: property, isLoading: isPropertyLoading } = useWebsiteQuery(websiteId, {
    enabled: !!user && reportPath && !isBlockedTeam && !accessBlocked,
    placeholderData: undefined,
  });
  const appReports = reportPath && property?.id === websiteId && isNativeApp(property?.domain);
  const workspaceHome = /^\/(?:teams\/[a-f0-9-]+\/)?websites\/?$/i.test(pathname);
  const standalone = appReports || workspaceHome;

  useEffect(() => {
    if (teamId) {
      setItem(LAST_TEAM_CONFIG, teamId);
    } else {
      removeItem(LAST_TEAM_CONFIG);
    }
  }, [teamId]);

  useEffect(() => {
    if (teamId && teamError) {
      removeItem(LAST_TEAM_CONFIG);
      router.replace('/');
    }
  }, [teamId, teamError, router]);

  useEffect(() => {
    if (isBillingPastDue && (!teamId || isOwner)) {
      window.location.href = `${process.env.cloudUrl}/settings/billing`;
    }
  }, [isBillingPastDue, isOwner, teamId]);

  if (isLoading || (config.cloudMode && isSubscriptionLoading) || (teamId && isTeamLoading)) {
    return <Loading placement="absolute" />;
  }

  if (error) {
    window.location.href = config.cloudMode
      ? `${process.env.cloudUrl}/login`
      : `${process.env.basePath || ''}/login`;
    return null;
  }

  if (!user) {
    return null;
  }

  if (teamId && teamError) {
    return null;
  }

  if (isBlockedTeam) {
    return (
      <Grid
        columns={{ base: '1fr', lg: 'auto 1fr' }}
        rows={{ base: 'auto 1fr', lg: '1fr' }}
        height="screen"
      >
        <Row display={{ base: 'flex', lg: 'none' }} alignItems="center" gap padding="3">
          <MobileNav />
        </Row>
        <Column display={{ base: 'none', lg: 'flex' }} minHeight="0" style={{ overflow: 'hidden' }}>
          <SideNav />
        </Column>
        <Column alignItems="center" justifyContent="center" padding="6" textAlign="center" gap="2">
          <Text size="lg" weight="bold">
            This team is temporarily unavailable.
          </Text>
          <Text color="muted">
            This team&apos;s subscription payment is overdue. Please contact the team owner to
            restore access.
          </Text>
        </Column>
      </Grid>
    );
  }

  if (reportPath && !accessBlocked && isPropertyLoading) {
    return <Loading placement="absolute" />;
  }

  return (
    <Grid
      columns={standalone ? 'minmax(0, 1fr)' : { base: '1fr', lg: 'auto 1fr' }}
      rows={standalone ? '1fr' : { base: 'auto 1fr', lg: '1fr' }}
      height="screen"
    >
      {!standalone && (
        <>
          <Row display={{ base: 'flex', lg: 'none' }} alignItems="center" gap padding="3">
            <MobileNav />
          </Row>
          <Column
            display={{ base: 'none', lg: 'flex' }}
            minHeight="0"
            style={{ overflow: 'hidden' }}
          >
            <SideNav />
          </Column>
        </>
      )}
      <Column overflowX="hidden" minHeight="0" position="relative">
        <TopNav appReports={appReports} workspaceHome={workspaceHome} />
        <Column
          alignItems="center"
          aria-hidden={accessBlocked || undefined}
          style={accessBlocked ? { pointerEvents: 'none' } : undefined}
        >
          {children}
        </Column>
      </Column>
      {needsTwoFactorSetup && <TwoFactorSetupModal required={true} />}
      {passwordChangeRequired && !needsTwoFactorSetup && <PasswordChangeRequiredModal />}
      <UpdateNotice user={user} config={config} />
      {process.env.NODE_ENV === 'production' && !pathname.includes('/share/') && (
        <Script src={`${process.env.basePath || ''}/telemetry.js`} />
      )}
      {process.env.selfTrack && (
        <Script
          async
          data-website-id={process.env.selfTrack}
          src={`${process.env.basePath || ''}/script.js`}
          data-cache="true"
          data-performance="true"
          data-errors="true"
        />
      )}
      {process.env.selfRecord && (
        <Script
          async
          data-website-id={process.env.selfRecord}
          data-sample-rate="1"
          src={`${process.env.basePath || ''}/recorder.js`}
        />
      )}
    </Grid>
  );
}
