import { ResponsiveLine } from '@nivo/line';
import { useEffect, useState } from 'react';

interface AggregationData {
  level_id: string;
  purpose: string;
  unit: string;
  resolution: string;
  timestamp: string;
  value: number;
  contributor_count: number;
}

interface ChartProps {
  startDate: string;
  endDate: string;
  levelId: string;
  resolution: string;
  purpose: string;
}

export default function AggregationChart({ startDate, endDate, levelId, resolution, purpose }: ChartProps) {
  const [data, setData] = useState<any[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const fetchData = async () => {
      if (!levelId) {
        setError('No node selected. Please select a node from the hierarchy.');
        setLoading(false);
        return;
      }

      try {
        setLoading(true);
        setError(null);

        // One request; the aggregate is keyed by purpose, so we group the returned rows
        // into one cumulative series per purpose (e.g. "Energy (Wh)", "Water (m3)").
        const params = new URLSearchParams({
          level_id: levelId,
          resolution: resolution,
          purpose: purpose,
          start: startDate,
          end: endDate,
        });
        const AGG_API_BASE_URL = import.meta.env.PUBLIC_AGG_API_BASE_URL || '';
        const url = `${AGG_API_BASE_URL}/aggregations?${params.toString()}`;
        const response = await fetch(url);
        if (!response.ok) {
          setError(`Request failed: ${response.statusText}`);
          setLoading(false);
          return;
        }
        const rows = (await response.json()) as AggregationData[];

        const byPurpose = new Map<string, AggregationData[]>();
        for (const r of rows) {
          const key = r.unit ? `${r.purpose} (${r.unit})` : r.purpose;
          const arr = byPurpose.get(key) ?? [];
          arr.push(r);
          byPurpose.set(key, arr);
        }
        const chartData = Array.from(byPurpose.entries()).map(([id, aggregations]) => {
          let cumulative = 0;
          return {
            id,
            data: aggregations.map((agg) => {
              cumulative += agg.value;
              return { x: new Date(agg.timestamp), y: cumulative };
            }),
          };
        });

        setData(chartData);
      } catch (err) {
        setError(err instanceof Error ? err.message : 'Failed to fetch data');
      } finally {
        setLoading(false);
      }
    };

    fetchData();
  }, [startDate, endDate, levelId, resolution, purpose]);

  if (loading) {
    return (
      <div style={{ display: 'grid', placeItems: 'center', height: '400px' }}>
        <div style={{ color: '#6b7280' }}>Loading chart data...</div>
      </div>
    );
  }

  if (error) {
    return (
      <div style={{ display: 'grid', placeItems: 'center', height: '400px' }}>
        <div style={{ color: '#dc2626' }}>{error}</div>
      </div>
    );
  }

  if (!data || data.length === 0 || data.every(d => d.data.length === 0)) {
    return (
      <div style={{ display: 'grid', placeItems: 'center', height: '400px' }}>
        <div style={{ color: '#6b7280' }}>No data available for the selected period</div>
      </div>
    );
  }

  return (
    <div style={{ height: '500px' }}>
      <ResponsiveLine
        data={data}
        margin={{ top: 50, right: 110, bottom: 70, left: 60 }}
        xScale={{
          type: 'time',
          useUTC: false,
        }}
        xFormat="time:%Y-%m-%d %H:%M"
        yScale={{
          type: 'linear',
          min: 'auto',
          max: 'auto',
          stacked: false,
          reverse: false,
        }}
        yFormat=" >-.2f"
        curve="monotoneX"
        theme={{
          text: { fill: '#cbd5e1', fontSize: 11 },
          axis: {
            domain: { line: { stroke: '#475569' } },
            ticks: { line: { stroke: '#475569' }, text: { fill: '#cbd5e1' } },
            legend: { text: { fill: '#e2e8f0', fontSize: 12 } },
          },
          legends: { text: { fill: '#cbd5e1' } },
          grid: { line: { stroke: '#334155', strokeWidth: 1 } },
          tooltip: { container: { background: '#1e293b', color: '#e2e8f0' } },
        }}
        axisTop={null}
        axisRight={null}
        axisBottom={{
          tickSize: 5,
          tickPadding: 5,
          tickRotation: -45,
          format: '%b %d %H:%M',
          legend: 'Time',
          legendOffset: 60,
          legendPosition: 'middle',
        }}
        axisLeft={{
          tickSize: 5,
          tickPadding: 5,
          tickRotation: 0,
          legend: 'Consumption',
          legendOffset: -50,
          legendPosition: 'middle',
        }}
        colors={{ scheme: 'category10' }}
        pointSize={8}
        pointColor={{ theme: 'background' }}
        pointBorderWidth={2}
        pointBorderColor={{ from: 'serieColor' }}
        useMesh={true}
        legends={[
          {
            anchor: 'bottom-right',
            direction: 'column',
            justify: false,
            translateX: 100,
            translateY: 0,
            itemsSpacing: 0,
            itemDirection: 'left-to-right',
            itemWidth: 80,
            itemHeight: 20,
            itemOpacity: 0.75,
            symbolSize: 12,
            symbolShape: 'circle',
            symbolBorderColor: 'rgba(0, 0, 0, .5)',
            effects: [
              {
                on: 'hover',
                style: {
                  itemBackground: 'rgba(0, 0, 0, .03)',
                  itemOpacity: 1,
                },
              },
            ],
          },
        ]}
      />
    </div>
  );
}
