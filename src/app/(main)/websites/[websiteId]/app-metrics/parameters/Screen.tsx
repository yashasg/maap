'use client';

import { Column, DataColumn, DataTable, Text } from '@umami/react-zen';
import { Empty } from '@/components/common/Empty';
import { LoadingPanel } from '@/components/common/LoadingPanel';
import { PageHeader } from '@/components/common/PageHeader';
import { Panel } from '@/components/common/Panel';
import { usePropertyFieldsQuery } from '@/components/hooks';
import { DATA_TYPES } from '@/lib/constants';

interface ParameterField {
  propertyName: string;
  dataType: number;
}

interface ParameterRow extends ParameterField {
  id: string;
  type: string;
}

function getTypeLabel(dataType: number) {
  const type = DATA_TYPES[dataType as keyof typeof DATA_TYPES];

  return type ? `${type[0].toUpperCase()}${type.slice(1)}` : `Unknown (${dataType})`;
}

export function getParameterRows(data: unknown): ParameterRow[] | null {
  if (!Array.isArray(data)) {
    return null;
  }

  const rows: ParameterRow[] = [];

  for (const item of data) {
    if (
      !item ||
      typeof item !== 'object' ||
      typeof item.propertyName !== 'string' ||
      !item.propertyName.trim() ||
      !Number.isInteger(item.dataType)
    ) {
      return null;
    }

    const propertyName = item.propertyName;
    const dataType = item.dataType;

    rows.push({
      id: `${propertyName}:${dataType}`,
      propertyName,
      dataType,
      type: getTypeLabel(dataType),
    });
  }

  return rows.sort(
    (left, right) =>
      left.propertyName.localeCompare(right.propertyName) || left.dataType - right.dataType,
  );
}

export default function ParametersScreen({ websiteId }: { websiteId: string }) {
  const { data, isLoading, isFetching, error } = usePropertyFieldsQuery(
    'event',
    websiteId,
    undefined,
  );
  const rows = data === undefined ? undefined : getParameterRows(data);
  const responseError =
    !isLoading && !isFetching && !error && (data === undefined || rows === null)
      ? new Error('Invalid parameter catalog response')
      : undefined;

  return (
    <Column gap="6">
      <PageHeader
        title="Parameters"
        description="Parameters observed for this app in the selected date range."
        showBorder={false}
      />
      <Panel title="Parameters">
        <LoadingPanel
          data={rows}
          error={error || responseError}
          isLoading={isLoading}
          isFetching={isFetching}
          isEmpty={rows?.length === 0}
          minHeight="240px"
          renderEmpty={() => (
            <Empty message="No parameters were observed for this app in the selected date range." />
          )}
        >
          {rows && (
            <DataTable data={rows}>
              <DataColumn id="propertyName" label="Name" width="minmax(180px, 1fr)">
                {(row: ParameterRow) => (
                  <Text truncate title={row.propertyName}>
                    {row.propertyName}
                  </Text>
                )}
              </DataColumn>
              <DataColumn id="type" label="Type" width="140px">
                {(row: ParameterRow) => <Text color="muted">{row.type}</Text>}
              </DataColumn>
            </DataTable>
          )}
        </LoadingPanel>
      </Panel>
    </Column>
  );
}
