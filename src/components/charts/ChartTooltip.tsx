import { Column, FloatingTooltip, Row, StatusLight, Text } from '@umami/react-zen';
import type { ReactNode } from 'react';

export function ChartTooltip({
  title,
  color,
  value,
}: {
  title?: string;
  color?: string;
  value?: ReactNode;
}) {
  return (
    <FloatingTooltip style={{ backgroundColor: 'transparent', padding: 0 }}>
      <Column
        gap="3"
        padding="4"
        borderRadius="md"
        style={{
          backgroundColor: 'var(--zen-surface-inverted, rgba(0, 0, 0, 0.85))',
          borderRadius: 'var(--zen-radius-md, 6px)',
          color: 'var(--zen-fg-inverted, #ffffff)',
          gap: '0.625rem',
          padding: '0.75rem 0.875rem',
        }}
      >
        {title && (
          <Text size="sm" style={{ fontSize: '0.75rem', lineHeight: 1.4 }}>
            {title}
          </Text>
        )}
        <Row alignItems="center">
          <StatusLight color={color}>
            <Text size="sm" style={{ fontSize: '0.75rem', lineHeight: 1.4 }}>
              {value}
            </Text>
          </StatusLight>
        </Row>
      </Column>
    </FloatingTooltip>
  );
}
