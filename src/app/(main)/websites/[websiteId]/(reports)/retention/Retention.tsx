import { Column, Grid, Icon, Row, Text } from '@umami/react-zen';
import { differenceInCalendarDays } from 'date-fns';
import { type ReactNode, useMemo } from 'react';
import { LoadingPanel } from '@/components/common/LoadingPanel';
import { Panel } from '@/components/common/Panel';
import { useLocale, useMessages, useRetentionQuery } from '@/components/hooks';
import { Users } from '@/components/icons';
import { formatDate, parseBackendDate } from '@/lib/date';
import { formatLongNumber } from '@/lib/format';

const DAYS = [1, 2, 3, 4, 5, 6, 7, 14, 21, 28];

export interface RetentionProps {
  websiteId: string;
  startDate: Date;
  endDate: Date;
  days?: number[];
}

export function Retention({ websiteId, days = DAYS, startDate, endDate }: RetentionProps) {
  const { t, labels } = useMessages();
  const { locale } = useLocale();
  const { data, error, isLoading } = useRetentionQuery({
    websiteId,
    startDate,
    endDate,
  });

  const { rows, maxPercentage } = useMemo(() => {
    if (!data?.length) {
      return { rows: [], maxPercentage: 0 };
    }

    const byDate = new Map<string, Map<number, any>>();
    const cohorts: Array<{ date: string; visitors: number }> = [];

    for (const row of data as Array<{
      date: string;
      visitors: number;
      day: number;
      percentage?: number;
    }>) {
      if (row.day === 0) {
        cohorts.push({ date: row.date, visitors: row.visitors });
      }
      let bucket = byDate.get(row.date);
      if (!bucket) {
        bucket = new Map();
        byDate.set(row.date, bucket);
      }
      bucket.set(row.day, row);
    }

    let max = 0;
    const computedRows = cohorts.map(({ date, visitors }) => {
      const parsedDate = parseBackendDate(date);
      const maxDay = differenceInCalendarDays(endDate, parsedDate);
      const bucket = byDate.get(date);
      const percentages = new Map<number, number>();

      for (const day of days) {
        const percentage = bucket?.get(day)?.percentage;
        if (percentage != null) {
          percentages.set(day, Number(percentage));
          if (day <= maxDay && Number(percentage) > max) {
            max = Number(percentage);
          }
        }
      }

      return { date, parsedDate, maxDay, visitors, percentages };
    });

    return { rows: computedRows, maxPercentage: max };
  }, [data, days, endDate]);

  return (
    <LoadingPanel data={data} isLoading={isLoading} error={error}>
      {data && (
        <Panel allowFullscreen minHeight="320px" maxHeight="900px">
          <Column paddingY="6" paddingX={{ base: '3', md: '6' }} gap="1" overflow="auto">
            <Column gap="1" role="table" aria-label={t(labels.cohort)}>
              <Grid
                role="row"
                columns={`120px repeat(${days.length}, 100px)`}
                alignItems="center"
                gap="1"
                height="50px"
                width="max-content"
                minWidth="100%"
                autoFlow="column"
              >
                <Column role="columnheader">
                  <Text weight="bold" align="center">
                    {t(labels.cohort)}
                  </Text>
                </Column>
                {days.map(n => (
                  <Column key={n} role="columnheader">
                    <Text weight="bold" align="center" wrap="nowrap">
                      {t(labels.day)} {n}
                    </Text>
                  </Column>
                ))}
              </Grid>
              {rows.map(({ date, parsedDate, maxDay, visitors, percentages }) => (
                <Grid
                  key={date}
                  role="row"
                  columns={`120px repeat(${days.length}, 100px)`}
                  gap="1"
                  autoFlow="column"
                  width="max-content"
                  minWidth="100%"
                >
                  <Column role="rowheader" justifyContent="center" gap="1">
                    <Text weight="bold">{formatDate(parsedDate, 'PP', locale)}</Text>
                    <Row alignItems="center" gap>
                      <Icon>
                        <Users />
                      </Icon>
                      <Text>{formatLongNumber(visitors)}</Text>
                    </Row>
                  </Column>
                  {days.map(day => {
                    if (day > maxDay) return null;
                    const percentage = percentages.get(day);
                    return (
                      <Cell
                        key={day}
                        // Square root keeps later days visible next to Day 1.
                        intensity={
                          percentage && maxPercentage ? Math.sqrt(percentage / maxPercentage) : 0
                        }
                      >
                        {percentage ? `${percentage.toFixed(2)}%` : ''}
                      </Cell>
                    );
                  })}
                </Grid>
              ))}
            </Column>
          </Column>
        </Panel>
      )}
    </LoadingPanel>
  );
}

const Cell = ({ intensity = 0, children }: { intensity?: number; children: ReactNode }) => {
  // Capped at 60% so the normal text color stays readable.
  const style = intensity
    ? {
        backgroundColor: `color-mix(in srgb, var(--zen-primary) ${Math.round(intensity * 60)}%, var(--zen-surface-raised))`,
      }
    : undefined;

  return (
    <Column
      role="cell"
      justifyContent="center"
      alignItems="center"
      width="100px"
      height="100px"
      backgroundColor="surface-raised"
      borderRadius
      style={style}
    >
      {children}
    </Column>
  );
};
