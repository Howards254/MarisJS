// @runsOn client
import { BarChart } from './BarChart';
import { StatsPanel } from './StatsPanel';

type AppProps = {};

export function App(props: AppProps) {
  const sales = signal([
    { product: 'Widget', amount: 450, prior: 380 },
    { product: 'Gadget', amount: 720, prior: 650 },
    { product: 'Doodad', amount: 300, prior: 310 },
  ]);

  // Level 1 computed — depends on sales
  const totalRevenue = computed(() => sales.value.reduce((s, i) => s + i.amount, 0));
  const priorRevenue = computed(() => sales.value.reduce((s, i) => s + i.prior, 0));
  const maxAmount = computed(() => Math.max(...sales.value.map(i => i.amount)));

  // Level 2 computed — depends on Level 1
  const taxAmount = computed(() => Math.round(totalRevenue.value * 0.1));
  const avgOrderValue = computed(() => Math.round(totalRevenue.value / sales.value.length));

  // Level 3 computed — depends on Level 2 (and Level 1 indirectly)
  const netRevenue = computed(() => totalRevenue.value - taxAmount.value);
  const percentChange = computed(() =>
    priorRevenue.value === 0 ? 0 : Math.round(((totalRevenue.value - priorRevenue.value) / priorRevenue.value) * 100)
  );

  // Bar chart data — depends on Level 1's maxAmount
  const barData = computed(() => sales.value.map(s => ({
    product: s.product,
    percentage: Math.round((s.amount / maxAmount.value) * 100),
  })));

  function addSale() {
    const products = ['Widget', 'Gadget', 'Doodad', 'Thingamabob', 'Whatsit'];
    const name = products[Math.floor(Math.random() * products.length)];
    const amount = 200 + Math.floor(Math.random() * 800);
    const prior = 150 + Math.floor(Math.random() * 700);
    sales.set([...sales.value, { product: name, amount, prior }]);
  }

  return (
    <div class="dashboard">
      <h1>Sales Dashboard</h1>
      <StatsPanel
        revenue={totalRevenue}
        tax={taxAmount}
        net={netRevenue}
        avg={avgOrderValue}
        change={percentChange}
      />
      <BarChart data={barData} />
      <button class="add-btn" onClick={addSale}>Add Random Sale</button>
    </div>
  );
}
