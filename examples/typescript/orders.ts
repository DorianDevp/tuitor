// Totals a few orders; gold and silver customers get a discount. Prices are in cents.
// Node runs the JavaScript that `tsc` writes to dist/; its source map leads back here.

type Tier = 'regular' | 'silver' | 'gold';

interface Address {
  city: string;
  country: string;
}

interface Customer {
  name: string;
  tier: Tier;
  address: Address;
}

interface Line {
  sku: string;
  qty: number;
  price: number;
}

interface Order {
  id: number;
  customer: Customer;
  lines: Line[];
}

const PERCENT_OFF: Record<Tier, number> = { regular: 0, silver: 5, gold: 10 };

function lineTotal(line: Line): number {
  return line.qty * line.price;
}

function discount(customer: Customer, amount: number): number {
  return Math.trunc((amount * PERCENT_OFF[customer.tier]) / 100);
}

function orderTotal(order: Order): number {
  let total = 0;
  for (const line of order.lines) {
    total += lineTotal(line);
  }
  return total - discount(order.customer, total);
}

function money(cents: number): string {
  return `${Math.trunc(cents / 100)}.${String(cents % 100).padStart(2, '0')}`;
}

const orders: Order[] = [
  {
    id: 1001,
    customer: { name: 'Ada', tier: 'gold', address: { city: 'London', country: 'UK' } },
    lines: [
      { sku: 'KB-01', qty: 1, price: 8900 },
      { sku: 'MS-02', qty: 2, price: 2450 },
      { sku: 'CB-03', qty: 3, price: 990 },
    ],
  },
  {
    id: 1002,
    customer: { name: 'Grace', tier: 'silver', address: { city: 'Arlington', country: 'US' } },
    lines: [
      { sku: 'MN-27', qty: 1, price: 21900 },
      { sku: 'HS-01', qty: 1, price: 5900 },
    ],
  },
  {
    id: 1003,
    customer: { name: 'Linus', tier: 'regular', address: { city: 'Helsinki', country: 'FI' } },
    lines: [{ sku: 'SSD-1T', qty: 2, price: 7450 }],
  },
];

let revenue = 0;
for (const order of orders) {
  const total = orderTotal(order);
  revenue += total;
  const { customer } = order;
  const skus = order.lines.map((line) => line.sku).join(' ');
  const { city, country } = customer.address;
  console.log(`#${order.id} ${customer.name} (${city}, ${country}): ${skus} = ${money(total)}`);
}
console.log(`revenue ${money(revenue)}`);
