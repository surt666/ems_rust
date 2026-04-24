import { useEffect, useState } from 'react';
import AggregationChart from './AggregationChart';

export default function AggregationChartWrapper() {
  const [levelId, setLevelId] = useState('');
  const [startDate, setStartDate] = useState('2025-09-24T00:00:00Z');
  const [endDate, setEndDate] = useState('2025-10-03T00:00:00Z');
  const [resolution, setResolution] = useState('hourly');

  useEffect(() => {
    // Get initial values from sessionStorage and form inputs
    const storedLevelId = sessionStorage.getItem('selectedNodeId') || '';
    setLevelId(storedLevelId);

    // Update displayed node ID
    const display = document.getElementById('nodeIdDisplay');
    if (display) {
      display.textContent = storedLevelId || 'Ingen valgt';
    }

    // Listen for sessionStorage changes
    const handleStorageChange = (e: StorageEvent) => {
      if (e.key === 'selectedNodeId') {
        const newLevelId = e.newValue || '';
        setLevelId(newLevelId);
        if (display) {
          display.textContent = newLevelId || 'Ingen valgt';
        }
      }
    };

    window.addEventListener('storage', handleStorageChange);

    // Listen for custom event from update button
    const handleUpdate = ((e: CustomEvent) => {
      setStartDate(e.detail.startDate);
      setEndDate(e.detail.endDate);
      setResolution(e.detail.resolution);
      setLevelId(e.detail.levelId);
    }) as EventListener;

    window.addEventListener('updateChart', handleUpdate);

    return () => {
      window.removeEventListener('storage', handleStorageChange);
      window.removeEventListener('updateChart', handleUpdate);
    };
  }, []);

  return (
    <AggregationChart
      startDate={startDate}
      endDate={endDate}
      levelId={levelId}
      resolution={resolution}
    />
  );
}
