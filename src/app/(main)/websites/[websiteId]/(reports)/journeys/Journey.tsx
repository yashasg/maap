import { Column, cn, Icon, Row, Text, Tooltip, TooltipTrigger } from '@umami/react-zen';
import { useMemo, useState } from 'react';
import { firstBy } from 'thenby';
import { LoadingPanel } from '@/components/common/LoadingPanel';
import { useEscapeKey, useJourneyQuery, useMessages } from '@/components/hooks';
import { File } from '@/components/icons';
import { Lightning } from '@/components/svg';
import { objectToArray } from '@/lib/data';
import { formatLongNumber } from '@/lib/format';
import styles from './Journey.module.css';

const NODE_HEIGHT = 60;
const NODE_GAP = 10;
const LINE_WIDTH = 3;

export interface JourneyProps {
  websiteId: string;
  startDate: Date;
  endDate: Date;
  steps: number;
  startStep?: string;
  endStep?: string;
  view: string;
}

const EVENT_TYPES = {
  views: 1,
  events: 2,
};

export function Journey({ websiteId, steps, startStep, endStep, view }: JourneyProps) {
  const [selectedNode, setSelectedNode] = useState(null);
  const [activeNode, setActiveNode] = useState(null);
  const { t, labels } = useMessages();
  const { data, error, isLoading } = useJourneyQuery({
    websiteId,
    steps,
    startStep,
    endStep,
    eventType: EVENT_TYPES[view],
  });

  useEscapeKey(() => setSelectedNode(null));

  const baseColumns = useMemo(() => {
    if (!data) {
      return [];
    }

    const cols = [];

    for (let columnIndex = 0; columnIndex < +steps; columnIndex++) {
      const nodes: Record<string, any> = {};

      data.forEach((entry: any, nodeIndex: number) => {
        const name = entry.items[columnIndex];
        if (!name) return;

        if (!nodes[name]) {
          nodes[name] = {
            name,
            count: entry.count,
            totalCount: entry.count,
            nodeIndex,
            columnIndex,
            paths: [entry],
          };
        } else {
          nodes[name].totalCount += entry.count;
          nodes[name].paths.push(entry);
        }
      });

      cols.push({
        nodes: objectToArray(nodes).sort(firstBy('totalCount', -1).thenBy('name')),
      });
    }

    return cols;
  }, [data, steps]);

  const columns = useMemo(() => {
    if (!baseColumns.length) {
      return [];
    }

    if (!selectedNode) {
      let previousTotal = 0;
      return baseColumns.map((column, columnIndex) => {
        const nodes = column.nodes.map((baseNode: any) => ({
          ...baseNode,
          selected: false,
          active: false,
          selectedCount: columnIndex ? 0 : baseNode.totalCount,
          activeCount: columnIndex ? 0 : baseNode.totalCount,
          lines: [],
        }));
        const visitorCount = nodes.reduce(
          (sum: number, { totalCount }: any) => sum + totalCount,
          0,
        );
        const dropOff =
          previousTotal > 0 ? ((visitorCount - previousTotal) / previousTotal) * 100 : 0;
        previousTotal = visitorCount;
        return { nodes, visitorCount, dropOff };
      });
    }

    const selectedPaths = selectedNode.paths ?? [];
    const activePaths = activeNode?.paths ?? [];
    const selectedByColumn = baseColumns.map(() => new Set<string>());
    const activeByColumn = baseColumns.map(() => new Set<string>());
    const edgeCountsByColumn = baseColumns.map(() => new Map<string, number>());

    for (const p of selectedPaths) {
      for (let col = 0; col < baseColumns.length; col++) {
        const curr = p.items[col];
        if (!curr) continue;
        selectedByColumn[col].add(curr);
        if (col > 0 && p.items[col - 1]) {
          const key = `${p.items[col - 1]}\0${curr}`;
          edgeCountsByColumn[col].set(key, (edgeCountsByColumn[col].get(key) ?? 0) + p.count);
        }
      }
    }
    for (const p of activePaths) {
      for (let col = 0; col < baseColumns.length; col++) {
        if (p.items[col]) activeByColumn[col].add(p.items[col]);
      }
    }

    const result: any[] = [];

    baseColumns.forEach((column, columnIndex) => {
      const previousNodes = result[columnIndex - 1]?.nodes;
      const edgeCounts = edgeCountsByColumn[columnIndex];

      const nodes = column.nodes.map((baseNode: any, currentNodeIndex: number) => {
        const selected = selectedByColumn[columnIndex].has(baseNode.name);
        const active = selected && activeByColumn[columnIndex].has(baseNode.name);
        let selectedCount = previousNodes ? 0 : baseNode.totalCount;
        let activeCount = selectedCount;
        const lines: number[][] = [];

        if (previousNodes && selected) {
          previousNodes.forEach((previousNode: any, previousNodeIndex: number) => {
            if (!previousNode.selected) return;
            const fromCount = edgeCounts.get(`${previousNode.name}\0${baseNode.name}`) ?? 0;
            if (fromCount) {
              lines.push([previousNodeIndex, currentNodeIndex]);
              selectedCount += fromCount;
              if (previousNode.active) {
                activeCount += fromCount;
              }
            }
          });
        }

        return { ...baseNode, selected, active, selectedCount, activeCount, lines };
      });

      const visitorCount = nodes.reduce(
        (sum: number, { selected, selectedCount, active, activeCount, totalCount }: any) => {
          if (!selectedNode) {
            sum += totalCount;
          } else if (!activeNode && selectedNode && selected) {
            sum += selectedCount;
          } else if (activeNode && active) {
            sum += activeCount;
          }
          return sum;
        },
        0,
      );

      const previousTotal = result[columnIndex - 1]?.visitorCount ?? 0;
      const dropOff =
        previousTotal > 0 ? ((visitorCount - previousTotal) / previousTotal) * 100 : 0;

      result.push({ nodes, visitorCount, dropOff });
    });

    return result;
  }, [baseColumns, selectedNode, activeNode]);

  const handleClick = (name: string, columnIndex: number, paths: any[]) => {
    if (name !== selectedNode?.name || columnIndex !== selectedNode?.columnIndex) {
      setSelectedNode({ name, columnIndex, paths });
    } else {
      setSelectedNode(null);
    }
    setActiveNode(null);
  };

  return (
    <LoadingPanel data={data} isLoading={isLoading} error={error} height="100%">
      <div className={styles.container}>
        <div className={styles.view}>
          {columns.map(({ visitorCount, nodes }, columnIndex) => {
            return (
              <div
                key={columnIndex}
                className={cn(styles.column, {
                  [styles.selected]: selectedNode,
                  [styles.active]: activeNode,
                })}
              >
                <div className={styles.header}>
                  <div className={styles.num}>{columnIndex + 1}</div>
                  <div className={styles.stats}>
                    <div className={styles.visitors} title={visitorCount}>
                      {formatLongNumber(visitorCount)} {t(labels.visitors)}
                    </div>
                  </div>
                </div>
                <div className={styles.nodes}>
                  {nodes.map(
                    ({
                      name,
                      totalCount,
                      selected,
                      active,
                      paths,
                      activeCount,
                      selectedCount,
                      lines,
                    }) => {
                      const nodeCount = selected
                        ? active
                          ? activeCount
                          : selectedCount
                        : totalCount;

                      const remaining =
                        columnIndex > 0
                          ? Math.round((nodeCount / columns[columnIndex - 1]?.visitorCount) * 100)
                          : 0;

                      const dropped = 100 - remaining;

                      return (
                        <div
                          key={name}
                          className={styles.wrapper}
                          onMouseEnter={() =>
                            selected && setActiveNode({ name, columnIndex, paths })
                          }
                          onMouseLeave={() => selected && setActiveNode(null)}
                        >
                          <div
                            className={cn(styles.node, {
                              [styles.selected]: selected,
                              [styles.active]: active,
                            })}
                            onClick={() => handleClick(name, columnIndex, paths)}
                          >
                            <Row alignItems="center" className={styles.name} title={name} gap>
                              <Icon>{name.startsWith('/') ? <File /> : <Lightning />}</Icon>
                              <Text truncate>{name}</Text>
                            </Row>
                            <div className={styles.count} title={nodeCount}>
                              {columnIndex === 0 || (selectedNode && !selected) ? (
                                <div>{formatLongNumber(nodeCount)}</div>
                              ) : (
                                <TooltipTrigger delay={0}>
                                  <div tabIndex={0}>{formatLongNumber(nodeCount)}</div>
                                  <Tooltip placement="top" sideOffset={20} showArrow>
                                    <Text transform="lowercase" color="red">
                                      {`${dropped}% ${t(labels.dropoff)}`}
                                    </Text>
                                    <Column>
                                      <Text transform="lowercase">
                                        {`${remaining}% ${t(labels.conversion)}`}
                                      </Text>
                                    </Column>
                                  </Tooltip>
                                </TooltipTrigger>
                              )}
                            </div>
                            {columnIndex < columns.length &&
                              lines.map(([fromIndex, nodeIndex], i) => {
                                const height =
                                  (Math.abs(nodeIndex - fromIndex) + 1) * (NODE_HEIGHT + NODE_GAP) -
                                  NODE_GAP;
                                const midHeight =
                                  (Math.abs(nodeIndex - fromIndex) - 1) * (NODE_HEIGHT + NODE_GAP) +
                                  NODE_GAP +
                                  LINE_WIDTH;
                                const nodeName = columns[columnIndex - 1]?.nodes[fromIndex].name;

                                return (
                                  <div
                                    key={`${fromIndex}${nodeIndex}${i}`}
                                    className={cn(styles.line, {
                                      [styles.active]:
                                        active &&
                                        activeNode?.paths.find(
                                          (path: { items: any[] }) =>
                                            path.items[columnIndex] === name &&
                                            path.items[columnIndex - 1] === nodeName,
                                        ),
                                      [styles.up]: fromIndex < nodeIndex,
                                      [styles.down]: fromIndex > nodeIndex,
                                      [styles.flat]: fromIndex === nodeIndex,
                                    })}
                                    style={{ height }}
                                  >
                                    <div className={cn(styles.segment, styles.start)} />
                                    <div
                                      className={cn(styles.segment, styles.mid)}
                                      style={{
                                        height: midHeight,
                                      }}
                                    />
                                    <div className={cn(styles.segment, styles.end)} />
                                  </div>
                                );
                              })}
                          </div>
                        </div>
                      );
                    },
                  )}
                </div>
              </div>
            );
          })}
        </div>
      </div>
    </LoadingPanel>
  );
}
