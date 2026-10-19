// Totals a few orders; gold and silver customers get a discount. Prices are in cents.

'use strict';

const Tier = Object.freeze({ Regular: 'regular', Silver: 'silver', Gold: 'gold' });

class Address {
  constructor(city, country) {
    this.city = city;
    this.country = country;
  }
}

class Customer {
  constructor(name, tier, address) {
    this.name = name;
    this.tier = tier;
    this.address = address;
  }
}

class Line {
  constructor(sku, qty, price) {
    this.sku = sku;
    this.qty = qty;
    this.price = price;
  }
}

class Order {
  constructor(id, customer, lines) {
    this.id = id;
    this.customer = customer;
    this.lines = lines;
  }
}

function lineTotal(line) {
  return line.qty * line.price;
}

function discount(customer, amount) {
  const percent = { [Tier.Regular]: 0, [Tier.Silver]: 5, [Tier.Gold]: 10 }[customer.tier];
  return Math.trunc((amount * percent) / 100);
}

function orderTotal(order) {
  let total = 0;
  for (const line of order.lines) {
    total += lineTotal(line);
  }
  return total - discount(order.customer, total);
}

function money(cents) {
  return `${Math.trunc(cents / 100)}.${String(cents % 100).padStart(2, '0')}`;
}

const orders = [
  new Order(1001, new Customer('Ada', Tier.Gold, new Address('London', 'UK')), [
    new Line('KB-01', 1, 8900),
    new Line('MS-02', 2, 2450),
    new Line('CB-03', 3, 990),
  ]),
  new Order(1002, new Customer('Grace', Tier.Silver, new Address('Arlington', 'US')), [
    new Line('MN-27', 1, 21900),
    new Line('HS-01', 1, 5900),
  ]),
  new Order(1003, new Customer('Linus', Tier.Regular, new Address('Helsinki', 'FI')), [
    new Line('SSD-1T', 2, 7450),
  ]),
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
