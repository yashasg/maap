import { Box, Column, type ColumnProps, FloatingTooltip, useTheme } from '@umami/react-zen';
import { colord } from 'colord';
import { type CSSProperties, memo, useCallback, useMemo, useRef, useState } from 'react';
import { ComposableMap, Geographies, Geography, ZoomableGroup } from 'react-simple-maps';
import {
  useCountryNames,
  useLocale,
  useMessages,
  useWebsiteMetricsQuery,
} from '@/components/hooks';
import { getThemeColors } from '@/lib/colors';
import { ISO_COUNTRIES, MAP_FILE } from '@/lib/constants';
import { percentFilter } from '@/lib/filters';
import { formatLongNumber } from '@/lib/format';
import styles from './WorldMap.module.css';

export interface WorldMapProps extends ColumnProps {
  websiteId?: string;
  data?: any[];
}

const WorldMapCanvas = memo(function WorldMapCanvas({
  fills,
  defaultFill,
  strokeColor,
  onHover,
  onLeave,
}: {
  fills: Map<string, string>;
  defaultFill: string;
  strokeColor: string;
  onHover: (code: string) => void;
  onLeave: () => void;
}) {
  return (
    <ComposableMap projection="geoMercator">
      <ZoomableGroup zoom={0.8} minZoom={0.7} center={[0, 40]}>
        <Geographies geography={`${process.env.basePath || ''}${MAP_FILE}`}>
          {({ geographies }) =>
            geographies.map(geo => {
              const code = ISO_COUNTRIES[geo.id];
              if (code === 'AQ') return null;

              return (
                <Geography
                  key={geo.rsmKey}
                  geography={geo}
                  fill={fills.get(code) ?? defaultFill}
                  stroke={strokeColor}
                  className={styles.geography}
                  onMouseOver={() => onHover(code)}
                  onMouseOut={onLeave}
                />
              );
            })
          }
        </Geographies>
      </ZoomableGroup>
    </ComposableMap>
  );
});

export function WorldMap({ websiteId, data, ...props }: WorldMapProps) {
  const [tooltip, setTooltipPopup] = useState<string | null>(null);
  const { theme } = useTheme();
  const { colors } = useMemo(() => getThemeColors(theme), [theme]);
  const { locale } = useLocale();
  const { t, labels } = useMessages();
  const { countryNames } = useCountryNames(locale);
  const visitorsLabel = t(labels.visitors).toLocaleLowerCase(locale);
  const unknownLabel = t(labels.unknown);

  const { data: mapData } = useWebsiteMetricsQuery(websiteId, {
    type: 'country',
  });

  const metrics = useMemo(
    () => (data || mapData ? percentFilter((data || mapData) as any[]) : []),
    [data, mapData],
  );

  const { counts, fills } = useMemo(() => {
    const counts = new Map<string, number>();
    const fills = new Map<string, string>();
    const base = colord(colors.map.baseColor);
    const adjust = theme === 'light' ? 'lighten' : 'darken';

    for (const country of metrics) {
      counts.set(country.x, country.y || 0);
      fills.set(country.x, base[adjust](0.4 * (1.0 - country.z / 100)).toHex());
    }

    return { counts, fills };
  }, [metrics, colors.map.baseColor, theme]);

  const hoverRef = useRef<(code: string) => void>(() => {});
  hoverRef.current = (code: string) => {
    if (code === 'AQ') return;
    const next = `${countryNames[code] || unknownLabel}: ${formatLongNumber(
      counts.get(code) || 0,
    )} ${visitorsLabel}`;
    setTooltipPopup(prev => (prev === next ? prev : next));
  };
  const handleHover = useCallback((code: string) => hoverRef.current(code), []);

  const handleLeave = useCallback(() => setTooltipPopup(null), []);

  return (
    <Column
      {...props}
      role="img"
      aria-label={`${t(labels.countries) || 'Countries'} map`}
      data-tip=""
      data-for="world-map-tooltip"
      style={
        {
          margin: 'auto 0',
          overflow: 'hidden',
          '--map-hover-color': colors.map.hoverColor,
        } as CSSProperties
      }
    >
      <WorldMapCanvas
        fills={fills}
        defaultFill={colors.map.fillColor}
        strokeColor={colors.map.strokeColor}
        onHover={handleHover}
        onLeave={handleLeave}
      />
      {tooltip && (
        <FloatingTooltip style={{ backgroundColor: 'transparent', padding: 0 }}>
          <Box
            style={{
              backgroundColor: 'var(--zen-surface-inverted, rgba(0, 0, 0, 0.85))',
              color: 'var(--zen-fg-inverted, #ffffff)',
            }}
            padding
            borderRadius="md"
          >
            {tooltip}
          </Box>
        </FloatingTooltip>
      )}
    </Column>
  );
}
