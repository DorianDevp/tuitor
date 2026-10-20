// A few orders; gold and silver customers get a discount. Prices are in cents.

export type Tier = 'regular' | 'silver' | 'gold';

export interface Address {
  city: string;
  country: string;
}

export interface Customer {
  name: string;
  tier: Tier;
  address: Address;
}

export interface Line {
  sku: string;
  qty: number;
  price: number;
}

export interface Order {
  id: number;
  customer: Customer;
  lines: Line[];
}

const PERCENT_OFF: Record<Tier, number> = { regular: 0, silver: 5, gold: 10 };

export function lineTotal(line: Line): number {
  return line.qty * line.price;
}

export function discount(customer: Customer, amount: number): number {
  return Math.trunc((amount * PERCENT_OFF[customer.tier]) / 100);
}

export function money(cents: number): string {
  return `${Math.trunc(cents / 100)}.${String(cents % 100).padStart(2, '0')}`;
}

export const ORDERS: Order[] = [
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
