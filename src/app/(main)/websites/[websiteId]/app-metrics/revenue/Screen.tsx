'use client';

import { Button, Code, Column, Heading, Icon, Loading, Row, Text } from '@umami/react-zen';
import { useState } from 'react';
import { CopyButton } from '@/components/common/CopyButton';
import { EmptyPlaceholder } from '@/components/common/EmptyPlaceholder';
import { Panel } from '@/components/common/Panel';
import {
  useAppSettingsMutation,
  useAppSettingsQuery,
  useNavigation,
  useRevenueStatsQuery,
  useTimezone,
} from '@/components/hooks';
import { useDateRange } from '@/components/hooks/useDateRange';
import { AlertTriangle, BadgeDollarSign, CircleCheck } from '@/components/icons';
import { CurrencySelect } from '@/components/input/CurrencySelect';
import { CURRENCY_CONFIG, DEFAULT_CURRENCY } from '@/lib/constants';
import { getItem, setItem } from '@/lib/storage';
import { Revenue } from '../../(reports)/revenue/Revenue';

function hasRevenue(data?: { sum?: number; count?: number; unique_count?: number }) {
  return (
    !!data &&
    ((Number.isFinite(Number(data.sum)) && Number(data.sum) !== 0) ||
      Number(data.count) > 0 ||
      Number(data.unique_count) > 0)
  );
}

export function getRevenueViewState({
  enabled,
  data,
  error,
  isLoading,
}: {
  enabled: boolean;
  data?: { sum?: number; count?: number; unique_count?: number };
  error?: unknown;
  isLoading: boolean;
}) {
  if (isLoading) return 'loading';
  if (error) return 'error';
  if (!enabled) return 'setup';
  return hasRevenue(data) ? 'report' : 'zero';
}

export default function RevenueScreen({ websiteId }: { websiteId: string }) {
  const { timezone } = useTimezone();
  const {
    dateRange: { startDate, endDate, unit },
    compare,
  } = useDateRange({ timezone });
  const { router, renderUrl } = useNavigation();
  const [currency, setCurrency] = useState(
    () => getItem(CURRENCY_CONFIG) || process.env.defaultCurrency || DEFAULT_CURRENCY,
  );
  const settings = useAppSettingsQuery(websiteId);
  const settingsMutation = useAppSettingsMutation(websiteId);
  const enabled = settings.data?.revenueDisplayMode === 'revenuecat';
  const stats = useRevenueStatsQuery({ websiteId, currency, compare });
  const state = getRevenueViewState({
    enabled,
    data: stats.data,
    error: settings.error || (enabled ? stats.error : null),
    isLoading: settings.isLoading || (enabled && stats.isLoading),
  });

  const handleCurrencyChange = (value: string) => {
    setCurrency(value);
    setItem(CURRENCY_CONFIG, value);
  };

  const handleConfigured = () => {
    if (!settings.data?.canWrite) return;

    settingsMutation.mutate(
      {
        overviewLayout: settings.data.overviewLayout,
        colorScheme: settings.data.colorScheme,
        revenueDisplayMode: 'revenuecat',
      },
      {
        onSuccess: () => settingsMutation.touch(`app-settings:${websiteId}`),
      },
    );
  };

  return (
    <Column gap="5">
      <Column gap="2">
        <Heading size="3xl">Revenue</Heading>
        <Text color="muted">
          Review purchase revenue received by Maap. Analytics revenue is operational data, not a
          replacement for payment-processor accounting.
        </Text>
      </Column>

      {state === 'loading' && (
        <Panel minHeight="360px">
          <Column alignItems="center" justifyContent="center" minHeight="320px">
            <Loading icon="spinner" placement="inline" />
          </Column>
        </Panel>
      )}

      {state === 'error' && (
        <Panel minHeight="360px">
          <EmptyPlaceholder
            icon={<AlertTriangle />}
            title="Revenue could not be loaded"
            description="Maap could not verify revenue setup or load the selected range. No zero values are being shown."
          >
            <Button
              variant="primary"
              onPress={() => {
                settings.refetch();
                if (enabled) stats.refetch();
              }}
            >
              Try again
            </Button>
          </EmptyPlaceholder>
        </Panel>
      )}

      {state === 'setup' && (
        <RevenueSetup
          currency={currency}
          onCurrencyChange={handleCurrencyChange}
          onConfigured={handleConfigured}
          canWrite={settings.data?.canWrite === true}
          isSaving={settingsMutation.isPending}
          saveError={settingsMutation.error}
          onOpenSetup={() => router.push(renderUrl(`/websites/${websiteId}/app-reports/setup`))}
        />
      )}

      {(state === 'zero' || state === 'report') && (
        <Column gap="4">
          {state === 'zero' && (
            <Panel>
              <Row gap="3" alignItems="center">
                <Icon color="primary">
                  <CircleCheck />
                </Icon>
                <Column gap="1">
                  <Text weight="bold">Revenue reporting is enabled</Text>
                  <Text color="muted">
                    Maap received no {currency} revenue in the selected range. This is a real zero,
                    not a setup or loading state.
                  </Text>
                </Column>
              </Row>
            </Panel>
          )}
          <Revenue websiteId={websiteId} startDate={startDate} endDate={endDate} unit={unit} />
        </Column>
      )}
    </Column>
  );
}

function RevenueSetup({
  currency,
  onCurrencyChange,
  onConfigured,
  canWrite,
  isSaving,
  saveError,
  onOpenSetup,
}: {
  currency: string;
  onCurrencyChange: (value: string) => void;
  onConfigured: () => void;
  canWrite: boolean;
  isSaving: boolean;
  saveError?: unknown;
  onOpenSetup: () => void;
}) {
  const snippet = `TelemetryDeck.signal(
  "purchase",
  parameters: [
    "revenue": "19.99",
    "currency": "${currency}"
  ]
)`;

  return (
    <Panel minHeight="440px">
      <Column gap="5" maxWidth="680px" padding="6" style={{ margin: '0 auto' }}>
        <Row gap="3" alignItems="center">
          <Icon color="primary" size="lg">
            <BadgeDollarSign />
          </Icon>
          <Column gap="1">
            <Heading size="2xl">Set up revenue tracking</Heading>
            <Text color="muted">
              Send purchase events through the same TelemetryDeck SDK connection already used by
              this app. No paid integration or payment-provider access is required.
            </Text>
          </Column>
        </Row>

        <Column gap="2">
          <Text weight="bold">1. Choose the report currency</Text>
          <Text color="muted">
            Revenue is never converted between currencies. Select the ISO currency you send with
            each purchase.
          </Text>
          <Column maxWidth="320px">
            <CurrencySelect value={currency} onChange={onCurrencyChange} />
          </Column>
        </Column>

        <Column gap="2">
          <Text weight="bold">2. Send a purchase signal</Text>
          <Text color="muted">
            Maap stores positive revenue values from event parameters and reports the amount exactly
            as sent.
          </Text>
          <Row
            alignItems="start"
            justifyContent="space-between"
            gap="3"
            padding="4"
            style={{
              backgroundColor: 'var(--zen-background-muted)',
              borderRadius: 'var(--zen-radius)',
            }}
          >
            <Code style={{ whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' }}>{snippet}</Code>
            <CopyButton value={snippet} label="Copy Swift revenue example" />
          </Row>
          <Text size="sm" color="muted">
            Use one currency per event. Maap does not calculate taxes, fees, refunds, exchange
            rates, or processor-final net revenue.
          </Text>
        </Column>

        <Row gap="3" wrap="wrap">
          <Button variant="primary" onPress={onConfigured} isDisabled={!canWrite || isSaving}>
            {isSaving ? 'Enabling revenue report…' : 'Enable revenue report'}
          </Button>
          <Button onPress={onOpenSetup}>Review app SDK setup</Button>
        </Row>
        {!canWrite && (
          <Text size="sm" color="muted">
            You can review the integration, but you do not have permission to enable this app’s
            revenue report.
          </Text>
        )}
        {saveError && (
          <Text size="sm" color="muted">
            Revenue reporting was not enabled. Check your access and try again.
          </Text>
        )}
      </Column>
    </Panel>
  );
}
