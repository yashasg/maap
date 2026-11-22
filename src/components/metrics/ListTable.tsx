import { Column, Grid, Row, Skeleton, Text } from '@umami/react-zen';
import { useSpring, useTransform } from 'motion/react';
import { type ReactNode, useEffect } from 'react';
import { List, type RowComponentProps } from 'react-window';
import { AnimatedDiv } from '@/components/common/AnimatedDiv';
import { Empty } from '@/components/common/Empty';
import { useMessages, useMobile } from '@/components/hooks';
import { useReducedMotion } from '@/components/hooks/useReducedMotion';
import { formatLongNumber } from '@/lib/format';

const ITEM_SIZE = 30;

export function ListTableSkeleton({
  title,
  metric,
  itemCount = 10,
  showPercentage = true,
}: Pick<ListTableProps, 'title' | 'metric' | 'itemCount' | 'showPercentage'>) {
  const { t, labels } = useMessages();
  return (
    <Column gap role="status" aria-label={t(labels.loading)} data-report-skeleton="table">
      <Grid columns="1fr 100px" paddingLeft="2" alignItems="center" aria-hidden>
        {title ? <Text weight="bold">{title}</Text> : <Skeleton width="40%" />}
        {metric ? (
          <Text weight="bold" align="center">
            {metric}
          </Text>
        ) : (
          <Skeleton />
        )}
      </Grid>
      <Column gap="1" aria-hidden>
        {Array.from({ length: itemCount }, (_, index) => (
          <Grid
            key={index}
            columns={showPercentage ? '1fr 50px 50px' : '1fr 100px'}
            height={`${ITEM_SIZE}px`}
            paddingLeft="2"
            alignItems="center"
            gap
          >
            <Skeleton width="60%" />
            <Skeleton width="75%" />
            {showPercentage && <Skeleton width="75%" />}
          </Grid>
        ))}
      </Column>
    </Column>
  );
}

interface ListData {
  label: string;
  count: number;
  percent: number;
}

export interface ListTableProps {
  data?: ListData[];
  title?: string;
  metric?: string;
  className?: string;
  renderLabel?: (data: ListData, index: number) => ReactNode;
  renderChange?: (data: ListData, index: number) => ReactNode;
  animate?: boolean;
  virtualize?: boolean;
  showPercentage?: boolean;
  itemCount?: number;
  formatCount?: (n: number) => string;
}

export function ListTable({
  data = [],
  title,
  metric,
  renderLabel,
  renderChange,
  animate = true,
  virtualize = false,
  showPercentage = true,
  itemCount = 10,
  formatCount,
}: ListTableProps) {
  const { t, labels } = useMessages();
  const { isPhone } = useMobile();
  const reducedMotion = useReducedMotion();

  const getRow = (row: ListData, index: number) => {
    const { label, count, percent } = row;

    return (
      <AnimatedRow
        key={`${label}${index}`}
        label={renderLabel ? renderLabel(row, index) : (label ?? t(labels.unknown))}
        value={count}
        percent={percent}
        animate={animate && !virtualize && !reducedMotion}
        showPercentage={showPercentage}
        change={renderChange ? renderChange(row, index) : null}
        formatCount={formatCount}
        isPhone={isPhone}
      />
    );
  };

  const ListTableRow = ({ index, style }: RowComponentProps) => {
    return <div style={style}>{getRow(data[index], index)}</div>;
  };

  return (
    <Column gap>
      <Grid
        alignItems="center"
        justifyContent="space-between"
        paddingLeft="2"
        columns={'1fr 100px'}
      >
        <Text weight="bold">{title}</Text>
        <Text weight="bold" align="center">
          {metric}
        </Text>
      </Grid>
      <Column gap="1">
        {data?.length === 0 && <Empty />}
        {virtualize && data.length > 0 ? (
          <List
            style={{ width: '100%', height: itemCount * ITEM_SIZE }}
            defaultHeight={itemCount * ITEM_SIZE}
            rowCount={data.length}
            rowHeight={ITEM_SIZE}
            rowComponent={ListTableRow}
            rowProps={{}}
          />
        ) : (
          data.map(getRow)
        )}
      </Column>
    </Column>
  );
}

const AnimatedRow = ({
  label,
  value = 0,
  percent,
  change,
  animate,
  showPercentage = true,
  formatCount,
  isPhone,
}: {
  label: ReactNode;
  value?: number;
  percent: number;
  change: ReactNode;
  animate: boolean;
  showPercentage?: boolean;
  formatCount?: (n: number) => string;
  isPhone: boolean;
}) => {
  const y = !Number.isNaN(value) ? value : 0;
  const ySpring = useSpring(y, { stiffness: 170, damping: 26 });
  const widthSpring = useSpring(percent, { stiffness: 170, damping: 26 });
  const yText = useTransform(ySpring, n => (formatCount ? formatCount(n) : formatLongNumber(n)));
  const widthText = useTransform(widthSpring, n => `${n?.toFixed?.(0)}%`);

  useEffect(() => {
    if (animate) {
      ySpring.set(y);
      widthSpring.set(percent);
    } else {
      ySpring.jump(y);
      widthSpring.jump(percent);
    }
  }, [y, percent, animate, ySpring, widthSpring]);

  return (
    <Grid
      columns={showPercentage ? '1fr 50px 50px' : '1fr 100px'}
      paddingLeft="2"
      alignItems="center"
      borderRadius
      gap
      hover={{ backgroundColor: 'surface-sunken' }}
    >
      <Row alignItems="center">
        <Text truncate={true} style={{ maxWidth: isPhone ? '200px' : '400px' }}>
          {label}
        </Text>
      </Row>
      <Row
        alignItems="center"
        height="30px"
        justifyContent={showPercentage ? 'flex-end' : 'center'}
      >
        {change}
        <Text weight="bold">
          <AnimatedDiv title={String(value)}>
            {animate ? yText : formatCount ? formatCount(y) : formatLongNumber(y)}
          </AnimatedDiv>
        </Text>
      </Row>
      {showPercentage && (
        <Row
          alignItems="center"
          justifyContent="flex-start"
          position="relative"
          border="left"
          borderColor="strong"
          color="muted"
          paddingLeft="3"
        >
          <AnimatedDiv>{animate ? widthText : `${percent?.toFixed?.(0)}%`}</AnimatedDiv>
        </Row>
      )}
    </Grid>
  );
};
