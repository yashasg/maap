'use client';

import {
  Alert,
  AlertDescription,
  AlertTitle,
  Column,
  Heading,
  Icon,
  Label,
  Loading,
  Row,
  Text,
} from '@umami/react-zen';
import { useEffect, useState } from 'react';
import { CopyButton } from '@/components/common/CopyButton';
import { LinkButton } from '@/components/common/LinkButton';
import { PageHeader } from '@/components/common/PageHeader';
import { useNavigation } from '@/components/hooks';
import { useWebsiteQuery } from '@/components/hooks/queries/useWebsiteQuery';
import { AlertTriangle, CheckCircle2 } from '@/components/icons';
import styles from './Screen.module.css';

const SWIFT_PACKAGE_URL = 'https://github.com/TelemetryDeck/SwiftSDK.git';
const SWIFT_VERSION = '2.14.2';

function getSelfHostedBase() {
  if (typeof window === 'undefined') {
    return '';
  }

  return `${window.location.origin}${process.env.basePath || ''}/`;
}

function swiftSetupCode(websiteId: string, ingestBase: string) {
  return `import Foundation
import TelemetryDeck

func startMaap() {
    let config = TelemetryDeck.Config(
        appID: "${websiteId}",
        baseURL: URL(string: "${ingestBase}")!
    )

    #if DEBUG
    config.testMode = true
    #endif

    TelemetryDeck.initialize(config: config)
}`;
}

export default function SetupScreen({ websiteId }: { websiteId: string }) {
  const [ingestBase, setIngestBase] = useState('');
  const websiteQuery = useWebsiteQuery(websiteId);
  const { renderUrl } = useNavigation();

  useEffect(() => {
    setIngestBase(getSelfHostedBase());
  }, []);

  if (websiteQuery.isLoading || !ingestBase) {
    return <Loading placement="absolute" />;
  }

  if (websiteQuery.error) {
    return (
      <Alert variant="danger">
        <AlertTriangle aria-hidden />
        <AlertTitle>Setup details could not be loaded</AlertTitle>
        <AlertDescription>Reload this page before configuring an SDK.</AlertDescription>
      </Alert>
    );
  }

  if (!websiteQuery.data) {
    return (
      <Alert>
        <AlertTitle>App unavailable</AlertTitle>
        <AlertDescription>
          This app may have been removed or you may no longer have access.
        </AlertDescription>
      </Alert>
    );
  }

  const code = swiftSetupCode(websiteId, ingestBase);
  const recentEventsUrl = renderUrl(`/websites/${websiteId}/app-reports/recent-events`, {
    testMode: 'true',
  });

  return (
    <Column gap="6" minWidth="0">
      <PageHeader
        title="Setup"
        description="Connect a verified SDK to this app, then confirm its first test signal in Maap."
        showBorder={false}
      />

      <Column gap="3">
        <Heading size="lg">Connection details</Heading>
        <Text color="muted">
          These values route events to this app. They are identifiers, not credentials.
        </Text>
        <div className={styles.identityGrid}>
          <div className={styles.valueCard}>
            <Label>App ID</Label>
            <div className={styles.valueRow}>
              <code className={styles.codeValue}>{websiteId}</code>
              <CopyButton value={websiteId} label="Copy app ID" />
            </div>
          </div>
          <div className={styles.valueCard}>
            <Label>Self-hosted ingest base</Label>
            <div className={styles.valueRow}>
              <code className={styles.codeValue}>{ingestBase}</code>
              <CopyButton value={ingestBase} label="Copy ingest base" />
            </div>
          </div>
        </div>
        <Alert>
          <AlertTitle>Leave the SDK namespace unset</AlertTitle>
          <AlertDescription>
            Maap uses the App ID to select the app. A TelemetryDeck namespace changes the endpoint;
            it is not an organization or team identifier.
          </AlertDescription>
        </Alert>
      </Column>

      <Column gap="3">
        <Heading size="lg">Set up the Swift SDK</Heading>
        <details>
          <summary>Other SDKs</summary>
          <Text color="muted">
            Only Swift {SWIFT_VERSION} is verified with Maap. Before using another SDK, check its
            ingest URL, request body, and Test Mode behavior.
          </Text>
        </details>
      </Column>

      <Column gap="5">
        <ol className={styles.steps}>
          <li className={styles.step}>
            <Column gap="2" minWidth="0">
              <Heading size="sm">Add the exact Swift package version</Heading>
              <Text color="muted">
                In Xcode, add the package URL and choose Exact Version {SWIFT_VERSION}.
              </Text>
              <div className={styles.codeShell}>
                <pre className={styles.codeBlock}>{SWIFT_PACKAGE_URL}</pre>
                <div className={styles.copyButton}>
                  <CopyButton value={SWIFT_PACKAGE_URL} label="Copy Swift package URL" />
                </div>
              </div>
            </Column>
          </li>

          <li className={styles.step}>
            <Column gap="2" minWidth="0">
              <Heading size="sm">Initialize once from the app entry point</Heading>
              <Text color="muted">
                Keep the namespace unset. The base URL must end before <code>/v2/</code>; the SDK
                appends that path.
              </Text>
              <div className={styles.codeShell}>
                <pre className={styles.codeBlock}>{code}</pre>
                <div className={styles.copyButton}>
                  <CopyButton value={code} label="Copy Swift setup code" />
                </div>
              </div>
            </Column>
          </li>

          <li className={styles.step}>
            <Column gap="2">
              <Heading size="sm">Send a test signal from a Debug build</Heading>
              <Text color="muted">
                Track a screen or a non-identifying named action. Do not submit a synthetic event
                from this production dashboard.
              </Text>
            </Column>
          </li>

          <li className={styles.step}>
            <Column gap="2">
              <Heading size="sm">Verify it in Test Mode</Heading>
              <Text color="muted">
                Open Recent Events with Test Mode enabled and look for the fresh signal from your
                device.
              </Text>
              <Row>
                <LinkButton href={recentEventsUrl} variant="primary">
                  <Icon>
                    <CheckCircle2 />
                  </Icon>
                  Open Recent Events in Test Mode
                </LinkButton>
              </Row>
            </Column>
          </li>
        </ol>

        <Alert>
          <AlertTitle>Confirm the ingest route before shipping</AlertTitle>
          <AlertDescription>
            <p>
              The URL above is this Maap server&apos;s origin. Your operator must allow the app to
              reach <code>/v2/</code> over HTTPS without an interactive dashboard login. Never embed
              access tokens, API keys or passwords in the app.
            </p>
          </AlertDescription>
        </Alert>
      </Column>
    </Column>
  );
}
