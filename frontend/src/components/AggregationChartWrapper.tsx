import { useEffect, useState } from 'react';
import AggregationChart from './AggregationChart';

export default function AggregationChartWrapper() {
  const [levelId, setLevelId] = useState('');
  const [startDate, setStartDate] = useState('2025-09-24T00:00:00Z');
  const [endDate, setEndDate] = useState('2025-10-03T00:00:00Z');
  const [resolution, setResolution] = useState('hourly');

  useEffect(() => {
    const display = document.getElementById('nodeIdDisplay');

    // The aggregate is keyed by the node's full hierarchy path, so levelId is
    // "<ancestor path>#<node id>"; the display shows just the node id.
    const computeLevelId = () => {
      const id = sessionStorage.getItem('selectedNodeId') || '';
      const path = sessionStorage.getItem('selectedNodePath') || '';
      return path ? `${path}#${id}` : id;
    };
    const refreshDisplay = () => {
      const id = sessionStorage.getItem('selectedNodeId') || '';
      if (display) display.textContent = id || 'Ingen valgt';
    };

    setLevelId(computeLevelId());
    refreshDisplay();

    // Listen for sessionStorage changes (either key affects the path)
    const handleStorageChange = (e: StorageEvent) => {
      if (e.key === 'selectedNodeId' || e.key === 'selectedNodePath') {
        setLevelId(computeLevelId());
        refreshDisplay();
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
