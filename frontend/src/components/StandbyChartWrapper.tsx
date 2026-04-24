import { useEffect, useState } from 'react';
import StandbyChart from './StandbyChart';

export default function StandbyChartWrapper() {
  const [startDate, setStartDate] = useState('2024-12-30T00:00:00Z');
  const [endDate, setEndDate] = useState('2025-01-13T00:00:00Z');
  const [resolution, setResolution] = useState('hourly');
  const [unit, setUnit] = useState('kWh');

  useEffect(() => {
    const handleUpdate = ((e: CustomEvent) => {
      if (e.detail.startDate) setStartDate(e.detail.startDate);
      if (e.detail.endDate) setEndDate(e.detail.endDate);
      if (e.detail.resolution) setResolution(e.detail.resolution);
      if (e.detail.unit) setUnit(e.detail.unit);
    }) as EventListener;

    window.addEventListener('updateStandbyChart', handleUpdate);
    return () => window.removeEventListener('updateStandbyChart', handleUpdate);
  }, []);

  return (
    <StandbyChart
      startDate={startDate}
      endDate={endDate}
      resolution={resolution}
      unit={unit}
    />
  );
}
