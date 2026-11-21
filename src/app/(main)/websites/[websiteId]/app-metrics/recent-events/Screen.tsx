'use client';

import {
  Button,
  Column,
  DataColumn,
  DataTable,
  type DataTableProps,
  Dialog,
  DialogTrigger,
  Grid,
  Heading,
  Icon,
  Label,
  Popover,
  Row,
  Text,
} from '@umami/react-zen';
import { useState } from 'react';
import { DataGrid } from '@/components/common/DataGrid';
import { DateDistance } from '@/components/common/DateDistance';
import { Empty } from '@/components/common/Empty';
import { IconLabel } from '@/components/common/IconLabel';
import { PageHeader } from '@/components/common/PageHeader';
import { TypeIcon } from '@/components/common/TypeIcon';
import { useFormat, useMessages, useNavigation, useWebsiteEventsQuery } from '@/components/hooks';
import { AlertTriangle, Eye, FileText, RefreshCw } from '@/components/icons';
import { FilterButtons } from '@/components/input/FilterButtons';
import { EventData } from '@/components/metrics/EventData';
import { Lightning } from '@/components/svg';
import type { WebsiteEvent } from '@/lib/types';

type EventView = 'all' | 'views' | 'events';

export default function RecentEventsScreen({ websiteId }: { websiteId: string }) {
  const { t, labels } = useMessages();
  const { router, updateParams } = useNavigation();
  const [view, setView] = useState<EventView>('all');
  const query = useWebsiteEventsQuery(websiteId, { view });

  const filters = [
    { id: 'all', label: t(labels.all) },
    { id: 'views', label: t(labels.views) },
    { id: 'events', label: t(labels.events) },
  ];

  const handleViewChange = (nextView: string) => {
    setView(nextView as EventView);
    router.push(updateParams({ page: 1 }));
  };

  return (
    <Column gap="4" minWidth="0">
      <PageHeader
        title="Recent Events"
        description="The latest events sent by your app in the selected range."
        showBorder={false}
      />

      {query.error ? (
        <Column
          alignItems="center"
          justifyContent="center"
          gap="3"
          minHeight="300px"
          border
          borderRadius="md"
          padding="6"
        >
          <Icon size="lg">
            <AlertTriangle />
          </Icon>
          <Heading size="base">Events could not be loaded</Heading>
          <Text color="muted">Check your connection and try loading this range again.</Text>
          <Button onPress={() => query.refetch()}>Try again</Button>
        </Column>
      ) : (
        <DataGrid
          query={query}
          allowSearch
          autoFocus={false}
          allowPaging
          renderActions={() => (
            <Row alignItems="center" gap wrap="wrap">
              <FilterButtons items={filters} value={view} onChange={handleViewChange} />
              <Button variant="quiet" isDisabled={query.isFetching} onPress={() => query.refetch()}>
                <Icon>
                  <RefreshCw />
                </Icon>
                <Text>{query.isFetching ? 'Refreshing' : 'Refresh'}</Text>
              </Button>
            </Row>
          )}
          renderEmpty={() => (
            <Empty message="No events match this range and filter. Try a wider range or clear the search." />
          )}
        >
          {({ data }) => <RecentEventsTable data={data} />}
        </DataGrid>
      )}
    </Column>
  );
}

function RecentEventsTable(props: DataTableProps) {
  const { t, labels } = useMessages();
  const { formatValue } = useFormat();

  return (
    <DataTable {...props}>
      <DataColumn id="event" label={t(labels.event)} width="2fr">
        {(row: WebsiteEvent) => {
          const name = row.eventName || row.urlPath || t(labels.unknown);

          return (
            <Row alignItems="center" gap="2" wrap="nowrap" style={{ minWidth: 0 }}>
              <IconLabel
                icon={row.eventName ? <Lightning /> : <Eye />}
                label={row.eventName ? t(labels.triggeredEvent) : t(labels.viewedPage)}
                labelProps={{ wrap: 'nowrap' }}
              />
              <Text weight="bold" title={name} truncate style={{ maxWidth: 360 }}>
                {name}
              </Text>
            </Row>
          );
        }}
      </DataColumn>
      <DataColumn id="location" label={t(labels.location)}>
        {(row: WebsiteEvent) => (
          <TypeIcon type="country" value={row.country}>
            {row.city ? `${row.city}, ` : ''}
            {formatValue(row.country, 'country')}
          </TypeIcon>
        )}
      </DataColumn>
      <DataColumn id="environment" label="Environment" width="220px">
        {(row: WebsiteEvent) => (
          <Column gap="1">
            <TypeIcon type="os" value={row.os}>
              {formatValue(row.os, 'os')}
            </TypeIcon>
            <Text color="muted" size="sm">
              {formatValue(row.device, 'device')} · {formatValue(row.browser, 'browser')}
            </Text>
          </Column>
        )}
      </DataColumn>
      <DataColumn id="created" width="150px" label={t(labels.created)}>
        {(row: WebsiteEvent) => <DateDistance date={new Date(row.createdAt)} />}
      </DataColumn>
      <DataColumn id="inspect" label="" width="110px">
        {(row: WebsiteEvent) => <InspectEventButton event={row} />}
      </DataColumn>
    </DataTable>
  );
}

function InspectEventButton({ event }: { event: WebsiteEvent }) {
  const name = event.eventName || event.urlPath || 'event';

  return (
    <DialogTrigger>
      <Button variant="quiet">
        <Icon>
          <FileText />
        </Icon>
        <Text>Inspect</Text>
      </Button>
      <Popover side="left" sideOffset={8}>
        <Dialog aria-label={`Event details: ${name}`}>
          <EventDetails event={event} title={name} />
        </Dialog>
      </Popover>
    </DialogTrigger>
  );
}

function EventDetails({ event, title }: { event: WebsiteEvent; title: string }) {
  const { formatValue } = useFormat();

  const path = event.hostname
    ? `${event.hostname}${event.urlPath || ''}${event.urlQuery ? `?${event.urlQuery}` : ''}`
    : event.urlPath;

  return (
    <Column gap="5" minWidth="300px" maxWidth="520px">
      <Column gap="1">
        <Text color="muted" size="sm">
          {event.eventName ? 'App event' : 'Page view'}
        </Text>
        <Heading size="lg">{title}</Heading>
        <DateDistance date={new Date(event.createdAt)} />
      </Column>

      <Grid columns={{ base: '1fr', sm: '1fr 1fr' }} gap="4">
        <Detail
          label="Location"
          value={[event.city, formatValue(event.country, 'country')].filter(Boolean).join(', ')}
        />
        <Detail label="Operating system" value={formatValue(event.os, 'os')} />
        <Detail label="Device" value={formatValue(event.device, 'device')} />
        <Detail label="Browser" value={formatValue(event.browser, 'browser')} />
        {path && <Detail label="Path" value={path} />}
        {event.referrerDomain && <Detail label="Referrer" value={event.referrerDomain} />}
      </Grid>

      {event.hasData && (
        <Column gap="2" border="top" paddingTop="4">
          <Heading size="base">Properties</Heading>
          <EventData websiteId={event.websiteId} eventId={event.id} />
        </Column>
      )}
    </Column>
  );
}

function Detail({ label, value }: { label: string; value?: string }) {
  return (
    <Column gap="1">
      <Label>{label}</Label>
      <Text>{value || 'Unknown'}</Text>
    </Column>
  );
}
