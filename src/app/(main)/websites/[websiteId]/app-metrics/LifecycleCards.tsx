import { Button, Column, Heading, Icon, Text } from '@umami/react-zen';
import type { ReactNode } from 'react';
import { AlertTriangle, RefreshCw } from '@/components/icons';
import styles from './Lifecycle.module.css';

export type CountPoint = { key: string; label: string; count: number };
export type SeriesProps = { title: string; subtitle?: string; points: CountPoint[] };
const number = new Intl.NumberFormat();

export function ReportSection({ title, children }: { title: string; children: ReactNode }) {
  const id = title.replaceAll(' ', '-').toLowerCase();
  return (
    <section aria-labelledby={id}>
      <Heading id={id} size="sm" className={styles.sectionTitle}>
        {title}
      </Heading>
      {children}
    </section>
  );
}

export function CardGrid({ children }: { children: ReactNode }) {
  return <div className={styles.grid}>{children}</div>;
}

export function CountSeriesCard({
  title,
  subtitle,
  points,
  missingMessage,
  peakLabel = '',
}: SeriesProps & { missingMessage: string; peakLabel?: string }) {
  const peak = Math.max(0, ...points.map(point => point.count));
  return (
    <article className={styles.card}>
      <header className={styles.cardHeader}>
        <div>
          <Heading size="sm">{title}</Heading>
          {subtitle && (
            <Text size="sm" color="muted">
              {subtitle}
            </Text>
          )}
        </div>
        <div className={styles.cardValue}>
          {peakLabel}
          {number.format(peak)}
        </div>
      </header>
      {points.length ? (
        <div className={styles.bars} role="img" aria-label={`${title} chart`}>
          {points.map(point => (
            <div className={styles.barColumn} key={point.key}>
              <div
                className={styles.bar}
                style={{ height: `${peak ? Math.max(4, (point.count / peak) * 100) : 4}%` }}
                title={`${point.label}: ${number.format(point.count)}`}
              />
              <span title={point.label}>{point.label}</span>
            </div>
          ))}
        </div>
      ) : (
        <div className={styles.cardEmpty}>{missingMessage}</div>
      )}
    </article>
  );
}

export function StateCard({ title, body }: { title: string; body: string }) {
  return (
    <Column
      alignItems="center"
      justifyContent="center"
      gap="2"
      minHeight="280px"
      border
      borderRadius="lg"
      padding="6"
    >
      <Heading size="base">{title}</Heading>
      <Text color="muted" align="center">
        {body}
      </Text>
    </Column>
  );
}

export function ErrorState({ report, onRetry }: { report: string; onRetry: () => void }) {
  return (
    <Column
      alignItems="center"
      justifyContent="center"
      gap="3"
      minHeight="300px"
      border
      borderRadius="lg"
      padding="6"
    >
      <Icon size="lg" color="red">
        <AlertTriangle />
      </Icon>
      <Heading size="base">{report} could not be loaded</Heading>
      <Text color="muted">Check the connection and try this range again.</Text>
      <Button onPress={onRetry}>
        <Icon>
          <RefreshCw />
        </Icon>
        <Text>Try again</Text>
      </Button>
    </Column>
  );
}

export function LoadingState({ label }: { label: string }) {
  return (
    <div className={styles.loading} aria-label={label}>
      {Array.from({ length: 4 }, (_, index) => (
        <div className={styles.loadingCard} key={index} />
      ))}
    </div>
  );
}
