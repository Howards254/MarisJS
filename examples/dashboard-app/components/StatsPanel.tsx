// @runsOn client
type StatsProps = {
  revenue: { value: number; };
  tax: { value: number; };
  net: { value: number; };
  avg: { value: number; };
  change: { value: number; };
};

export function StatsPanel(props: StatsProps) {
  // Derived display values — recomputed whenever parent signals change.
  // The bind() wrappers on these text nodes will re-subscribe on every update.
  const trend = props.change.value >= 0 ? 'up' : 'down';
  const trendClass = 'trend-' + trend;

  return (
    <div class="stats-panel">
      <div class="stat-card">
        <span class="stat-label">Total Revenue</span>
        <span class="stat-value">${props.revenue.value}</span>
      </div>
      <div class="stat-card">
        <span class="stat-label">Tax (10%)</span>
        <span class="stat-value">${props.tax.value}</span>
      </div>
      <div class="stat-card">
        <span class="stat-label">Net Revenue</span>
        <span class="stat-value net">${props.net.value}</span>
      </div>
      <div class="stat-card">
        <span class="stat-label">Avg Order</span>
        <span class="stat-value">${props.avg.value}</span>
      </div>
      <div class={'stat-card ' + trendClass}>
        <span class="stat-label">vs Prior Period</span>
        <span class="stat-value">{props.change.value}%</span>
      </div>
    </div>
  );
}
