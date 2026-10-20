import { Component, OnInit, signal } from '@angular/core';

import { ORDERS, Order, discount, lineTotal, money } from './orders';

interface Row {
  order: Order;
  skus: string;
  total: number;
}

// Totals the orders when the page loads and again on "Total again".
@Component({
  selector: 'app-root',
  template: `
    <h1>Orders</h1>
    <table>
      <thead>
        <tr><th>Order</th><th>Customer</th><th>Items</th><th>Total</th></tr>
      </thead>
      <tbody>
        @for (row of rows(); track row.order.id) {
          <tr>
            <td>#{{ row.order.id }}</td>
            <td>{{ row.order.customer.name }} ({{ row.order.customer.address.city }})</td>
            <td>{{ row.skus }}</td>
            <td>{{ money(row.total) }}</td>
          </tr>
        }
      </tbody>
    </table>
    <p>Revenue {{ money(revenue()) }}</p>
    <button (click)="render()">Total again</button>
  `,
  styles: `
    :host { display: block; font: 16px/1.5 system-ui, sans-serif; margin: 2rem; }
    td, th { padding: 0.2rem 1rem 0.2rem 0; text-align: left; }
    td:last-child { text-align: right; }
  `,
})
export class App implements OnInit {
  protected readonly rows = signal<Row[]>([]);
  protected readonly revenue = signal(0);
  protected readonly money = money;

  ngOnInit(): void {
    this.render();
  }

  protected render(): void {
    const rows: Row[] = [];
    let revenue = 0;
    for (const order of ORDERS) {
      const total = this.orderTotal(order);
      revenue += total;
      const skus = order.lines.map((line) => line.sku).join(' ');
      rows.push({ order, skus, total });
      console.log(`#${order.id} ${order.customer.name}: ${skus} = ${money(total)}`);
    }
    this.rows.set(rows);
    this.revenue.set(revenue);
    console.log(`revenue ${money(revenue)}`);
  }

  private orderTotal(order: Order): number {
    let total = 0;
    for (const line of order.lines) {
      total += lineTotal(line);
    }
    return total - discount(order.customer, total);
  }
}
