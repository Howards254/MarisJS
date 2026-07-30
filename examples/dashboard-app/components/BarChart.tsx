// @runsOn client
type BarChartProps = {
  data: { value: { product: string; percentage: number; }[]; };
};

export function BarChart(props: BarChartProps) {
  return (
    <div class="bar-chart">
      <h2>Sales Volume</h2>
      <div class="bars-container">
        <For each={props.data.value} key={(item) => item.product}>
          {(item) => (
            <div class="bar-wrapper">
              <div class="bar-label">{item.product}</div>
              <div
                class="bar-fill"
                style={'width:' + item.percentage + '%'}
              />
              <span class="bar-pct">{item.percentage}%</span>
            </div>
          )}
        </For>
      </div>
    </div>
  );
}
