import { useEffect, useState } from "react";
import EChartsChart, { type EChartsSeries } from "./EChartsChart";

// client:only entry for the ECharts island (mirrors StandbyChartWrapper).
// Optionally reacts to a window CustomEvent so page controls can update it
// without any page-level JS file; today it just renders the (mock) defaults.
interface Props {
  categories?: string[];
  series?: EChartsSeries[];
  unit?: string;
  height?: number;
  stacked?: boolean;
  zoom?: boolean;
  eventName?: string;
}

export default function EChartsChartWrapper(props: Props) {
  const [categories, setCategories] = useState(props.categories);
  const [series, setSeries] = useState(props.series);
  const [unit, setUnit] = useState(props.unit ?? "kWh");

  useEffect(() => {
    if (!props.eventName) return;
    const handler = ((e: CustomEvent) => {
      if (e.detail?.categories) setCategories(e.detail.categories);
      if (e.detail?.series) setSeries(e.detail.series);
      if (e.detail?.unit) setUnit(e.detail.unit);
    }) as EventListener;
    window.addEventListener(props.eventName, handler);
    return () => window.removeEventListener(props.eventName!, handler);
  }, [props.eventName]);

  return <EChartsChart categories={categories} series={series} unit={unit} height={props.height} stacked={props.stacked} zoom={props.zoom} />;
}
