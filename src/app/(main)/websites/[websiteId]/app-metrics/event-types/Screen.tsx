'use client';

import { Column, Text } from '@umami/react-zen';
import { Empty } from '@/components/common/Empty';
import Link from '@/components/common/Link';
import { LoadingPanel } from '@/components/common/LoadingPanel';
import { PageHeader } from '@/components/common/PageHeader';
import { Panel } from '@/components/common/Panel';
import { useApi } from '@/components/hooks/useApi';
import { useDateParameters } from '@/components/hooks/useDateParameters';
import { useNavigation } from '@/components/hooks/useNavigation';
import styles from './Screen.module.css';

export interface EventTypeRow {
  name: string;
  users: number;
  events: number;
}

type EventTypesResponse = {
  data: EventTypeRow[];
};

export default function EventTypesScreen({ websiteId }: { websiteId: string }) {
  const { get, useQuery } = useApi();
  const { startAt, endAt } = useDateParameters();
  const { query, renderUrl } = useNavigation();
  const testMode = query.testMode === 'true';
  const eventTypes = useQuery<EventTypesResponse, Error, EventTypeRow[]>({
    queryKey: ['app-reports:event-types', { websiteId, startAt, endAt, testMode }],
    queryFn: () =>
      get(`/websites/${websiteId}/app-reports/event-types`, {
        startAt,
        endAt,
        testMode: testMode ? 'true' : undefined,
      }),
    select: response => response.data,
    enabled: Boolean(websiteId),
  });

  return (
    <Column gap="6">
      <PageHeader
        title="Event Types"
        description="Events received from this app, with reach counted by SDK user identity."
        showBorder={false}
      />

      <Panel title="Event Types" paddingX="0" paddingBottom="0">
        <LoadingPanel
          data={eventTypes.data}
          isLoading={eventTypes.isLoading}
          isFetching={eventTypes.isFetching}
          error={eventTypes.error}
          minHeight="220px"
          renderEmpty={() => <Empty message="No event types were received in this range." />}
        >
          <div className={styles.tableScroll}>
            <table className={styles.table}>
              <thead>
                <tr>
                  <th scope="col">Name</th>
                  <th scope="col">Users</th>
                  <th scope="col">Events</th>
                </tr>
              </thead>
              <tbody>
                {eventTypes.data?.map(row => (
                  <tr key={row.name}>
                    <td>
                      <Link
                        className={styles.eventLink}
                        href={renderUrl(`/websites/${websiteId}/app-reports/recent-events`, {
                          search: row.name,
                          page: undefined,
                        })}
                      >
                        {row.name}
                      </Link>
                    </td>
                    <td className={styles.number}>{row.users.toLocaleString()}</td>
                    <td className={styles.number}>{row.events.toLocaleString()}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <Text className={styles.note} color="muted" size="sm">
            Signals without an SDK user identity are included in Events, but not Users.
          </Text>
        </LoadingPanel>
      </Panel>
    </Column>
  );
}
