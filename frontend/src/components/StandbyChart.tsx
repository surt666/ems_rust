import { ResponsiveLine } from '@nivo/line';
import { useEffect, useState, useCallback } from 'react';

interface StandbyDataPoint {
  timestamp: string;
  value: number;
  isStandby: boolean;
}

interface StandbyChartProps {
  buildingId?: string;
  startDate?: string;
  endDate?: string;
  resolution?: string;
  unit?: string;
}

function generateMockData(start: Date, end: Date): StandbyDataPoint[] {
  const points: StandbyDataPoint[] = [];
  const current = new Date(start);

  while (current <= end) {
    const hour = current.getHours();
    const day = current.getDay();
    const isWeekend = day === 0 || day === 6;
    const isNight = hour < 7 || hour >= 18;
    const isStandby = isWeekend || isNight;

    let base: number;
    if (isStandby) {
      base = 15 + Math.random() * 25;
    } else {
      base = 80 + Math.random() * 120;
      if (hour >= 9 && hour <= 15) {
        base += 30 + Math.random() * 40;
      }
    }

    points.push({
      timestamp: current.toISOString(),
      value: Math.round(base * 100) / 100,
      isStandby: isStandby,
    });

    current.setHours(current.getHours() + 1);
  }

  return points;
}

function StandbyBackgroundLayer({ xScale, innerHeight, data }: any & { data: StandbyDataPoint[] }) {
  if (!data || data.length === 0) return null;

  const bands: { x1: number; x2: number }[] = [];
  let bandStart: number | null = null;

  for (let i = 0; i < data.length; i++) {
    const point = data[i];
    const x = (xScale as any)(new Date(point.timestamp));

    if (point.isStandby && bandStart === null) {
      bandStart = x;
    } else if (!point.isStandby && bandStart !== null) {
      bands.push({ x1: bandStart, x2: x });
      bandStart = null;
    }
  }

  if (bandStart !== null) {
    const lastX = (xScale as any)(new Date(data[data.length - 1].timestamp));
    bands.push({ x1: bandStart, x2: lastX });
  }

  return (
    <g>
      {bands.map((band, i) => (
        <rect
          key={i}
          x={band.x1}
          y={0}
          width={Math.max(band.x2 - band.x1, 1)}
          height={innerHeight}
          fill="rgba(239, 68, 68, 0.12)"
        />
      ))}
    </g>
  );
}

export default function StandbyChart({
  startDate = '2024-12-30T00:00:00Z',
  endDate = '2025-01-13T00:00:00Z',
  unit = 'kWh',
}: StandbyChartProps) {
  const [rawData, setRawData] = useState<StandbyDataPoint[]>([]);
  const [chartData, setChartData] = useState<any[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    const start = new Date(startDate);
    const end = new Date(endDate);
    const mock = generateMockData(start, end);
    setRawData(mock);

    setChartData([
      {
        id: `Total (Hovedmålere)`,
        data: mock.map((p) => ({
          x: new Date(p.timestamp),
          y: p.value,
        })),
      },
    ]);
    setLoading(false);
  }, [startDate, endDate]);

  const backgroundLayer = useCallback(
    (props: any) => <StandbyBackgroundLayer {...props} data={rawData} />,
    [rawData]
  );

  if (loading) {
    return (
      <div style={{ display: 'grid', placeItems: 'center', height: '500px' }}>
        <div style={{ color: '#6b7280' }}>Loading chart data...</div>
      </div>
    );
  }

  return (
    <div style={{ height: '500px' }}>
      <ResponsiveLine
        data={chartData}
        margin={{ top: 20, right: 30, bottom: 70, left: 70 }}
        xScale={{
          type: 'time',
          useUTC: false,
        }}
        xFormat="time:%d. %b %H:%M"
        yScale={{
          type: 'linear',
          min: 0,
          max: 'auto',
          stacked: false,
        }}
        yFormat={` >-.2f`}
        curve="monotoneX"
        colors={['#f5841f']}
        lineWidth={1.5}
        enablePoints={false}
        enableArea={false}
        useMesh={true}
        enableGridX={false}
        enableGridY={true}
        layers={[
          'grid',
          backgroundLayer,
          'markers',
          'axes',
          'areas',
          'crosshair',
          'lines',
          'slices',
          'mesh',
          'legends',
        ]}
        axisBottom={{
          tickSize: 5,
          tickPadding: 5,
          tickRotation: -45,
          format: '%d. %b %H:%M',
          tickValues: 'every 12 hours',
        }}
        axisLeft={{
          tickSize: 5,
          tickPadding: 5,
          tickRotation: 0,
          format: (v) => `${v} ${unit}`,
        }}
        tooltip={({ point }) => (
          <div
            style={{
              background: '#ffffff',
              color: '#28333d',
              padding: '8px 12px',
              borderRadius: '4px',
              fontSize: '12px',
              border: '1px solid #e5e8eb',
              boxShadow: '0 2px 8px rgba(0,0,0,0.12)',
            }}
          >
            <div style={{ marginBottom: '4px', color: '#5a6671' }}>
              {new Date(point.data.x as Date).toLocaleDateString('da-DK', {
                weekday: 'long',
                day: 'numeric',
                month: 'long',
                year: 'numeric',
                hour: '2-digit',
                minute: '2-digit',
              })}
            </div>
            <div>
              <span style={{ color: '#f5841f' }}>&#9632;</span>{' '}
              {(point as any).serieId ?? (point as any).id}: <strong>{Number(point.data.yFormatted).toFixed(0)} {unit}</strong>
            </div>
          </div>
        )}
        theme={{
          background: 'transparent',
          text: { fill: '#5a6671', fontSize: 11 },
          grid: {
            line: { stroke: '#e5e8eb', strokeWidth: 1 },
          },
          axis: {
            ticks: {
              text: { fill: '#5a6671', fontSize: 10 },
              line: { stroke: '#e5e8eb' },
            },
          },
          crosshair: {
            line: { stroke: '#8a949e', strokeWidth: 1, strokeDasharray: '4 4' },
          },
        }}
      />
    </div>
  );
}
