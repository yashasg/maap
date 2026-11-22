import { Column, type ColumnProps, Loading } from '@umami/react-zen';
import type { ReactNode } from 'react';
import { Empty } from '@/components/common/Empty';
import { ErrorMessage } from '@/components/common/ErrorMessage';

export interface LoadingPanelProps extends ColumnProps {
  data?: any;
  error?: unknown;
  isEmpty?: boolean;
  isLoading?: boolean;
  isFetching?: boolean;
  loadingIcon?: 'dots' | 'spinner';
  loadingPlacement?: 'center' | 'absolute' | 'inline';
  renderEmpty?: () => ReactNode;
  children: ReactNode;
}

export function LoadingPanel({
  data,
  error,
  isEmpty,
  isLoading,
  isFetching,
  loadingIcon = 'dots',
  loadingPlacement = 'absolute',
  renderEmpty = () => <Empty />,
  children,
  ...props
}: LoadingPanelProps): ReactNode {
  const empty = isEmpty ?? checkEmpty(data);
  const loading = isLoading || (isFetching && data == null);

  if (error && !loading) {
    return <ErrorMessage />;
  }

  return (
    <Column position="relative" width="100%" {...props} aria-busy={!!(loading || isFetching)}>
      {!loading && (empty ? renderEmpty() : children)}
      {(loading || isFetching) && (
        <Loading icon={loadingIcon} placement={loading ? loadingPlacement : 'absolute'} />
      )}
    </Column>
  );
}

function checkEmpty(data: any) {
  if (!data) return false;

  if (Array.isArray(data)) {
    return data.length <= 0;
  }

  if (typeof data === 'object') {
    return Object.keys(data).length <= 0;
  }

  return !!data;
}
