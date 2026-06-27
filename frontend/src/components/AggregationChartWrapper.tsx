import { useEffect, useState } from 'react';
import AggregationChart from './AggregationChart';

export default function AggregationChartWrapper() {
  const [levelId, setLevelId] = useState('');
  const [startDate, setStartDate] = useState(() => new Date(Date.now() - 7 * 24 * 60 * 60 * 1000).toISOString());
  const [endDate, setEndDate] = useState(() => new Date().toISOString());
  const [resolution, setResolution] = useState('hourly');
  const [purpose, setPurpose] = useState('Energy');
  const [refreshKey, setRefreshKey] = useState(0);

  useEffect(() => {
    const display = document.getElementById('nodeIdDisplay');

    // The aggregate is keyed by the node's full hierarchy path, so levelId is
    // "<ancestor path>#<node id>"; the display shows just the node id.
    const computeLevelId = () => {
      const id = sessionStorage.getItem('selectedNodeId') || '';
      const path = sessionStorage.getItem('selectedNodePath') || '';
      const company = sessionStorage.getItem('selectedCompanyId') || '';
      let level = path ? `${path}#${id}` : id;
      // The rollup is partitioned by the company (HN2); ensure it's in the path
      // even when the tree only stored a partial parent path.
      if (company && level && !level.includes(company)) level = `${company}#${level}`;
      return level;
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
      setPurpose(e.detail.purpose);
      setLevelId(e.detail.levelId);
      setRefreshKey((n) => n + 1); // force a re-fetch even if nothing else changed
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
      purpose={purpose}
      refreshKey={refreshKey}
    />
  );
}
