import { fireEvent } from '@testing-library/react';
import { beforeEach, expect, test, vi } from 'vitest';
import { render, screen } from '@/test/render';
import { WorldMap } from './WorldMap';

const renderSpy = vi.hoisted(() => vi.fn());

vi.mock('react-simple-maps', () => ({
  ComposableMap: ({ children }: any) => {
    renderSpy();
    return <svg>{children}</svg>;
  },
  ZoomableGroup: ({ children }: any) => <g>{children}</g>,
  Geographies: ({ children }: any) =>
    children({
      geographies: [
        { id: 'USA', rsmKey: 'us' },
        { id: 'ATA', rsmKey: 'aq' },
      ],
    }),
  Geography: ({ geography, onMouseOver, onMouseOut }: any) => (
    <path
      data-test={`geo-${geography.rsmKey}`}
      onMouseOver={onMouseOver}
      onMouseOut={onMouseOut}
    />
  ),
}));

vi.mock('@/components/hooks', async importOriginal => {
  const actual = await importOriginal<typeof import('@/components/hooks')>();
  return {
    ...actual,
    useWebsiteMetricsQuery: () => ({ data: undefined }),
    useLocale: () => ({ locale: 'en-US' }),
    useCountryNames: () => ({ countryNames: { US: 'United States' } }),
    useMessages: () => ({
      t: (v: string) => v,
      labels: { visitors: 'Visitors', unknown: 'Unknown', countries: 'Countries' },
    }),
  };
});

beforeEach(() => {
  renderSpy.mockClear();
});

test('omits Antarctica, exposes img role, and does not re-render SVG map on hover', () => {
  render(<WorldMap data={[{ x: 'US', y: 42 }]} />);

  expect(screen.getByRole('img', { name: 'Countries map' })).toBeInTheDocument();
  expect(screen.getByTestId('geo-us')).toBeInTheDocument();
  expect(screen.queryByTestId('geo-aq')).not.toBeInTheDocument();
  const initialRenders = renderSpy.mock.calls.length;

  fireEvent.mouseOver(screen.getByTestId('geo-us'));
  expect(screen.getByText('United States: 42 visitors')).toBeInTheDocument();
  expect(renderSpy).toHaveBeenCalledTimes(initialRenders);
});
