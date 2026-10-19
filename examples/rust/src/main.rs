//! Totals a few orders; gold and silver customers get a discount. Prices are in cents.
//!
//! The data uses `&str` and slices: plain gdb shows those as text and lists, while
//! `String` and `Vec` need rust-gdb's pretty printers (see README.md).

#[derive(Debug)]
enum Tier {
    Regular,
    Silver,
    Gold,
}

#[derive(Debug)]
struct Address {
    city: &'static str,
    country: &'static str,
}

#[derive(Debug)]
struct Customer {
    name: &'static str,
    tier: Tier,
    address: Address,
}

#[derive(Debug)]
struct Line {
    sku: &'static str,
    qty: u64,
    price: u64,
}

#[derive(Debug)]
struct Order {
    id: u32,
    customer: Customer,
    lines: &'static [Line],
}

fn line_total(line: &Line) -> u64 {
    line.qty * line.price
}

fn discount(customer: &Customer, amount: u64) -> u64 {
    let percent = match customer.tier {
        Tier::Regular => 0,
        Tier::Silver => 5,
        Tier::Gold => 10,
    };
    amount * percent / 100
}

fn order_total(order: &Order) -> u64 {
    let mut total = 0;
    for line in order.lines {
        total += line_total(line);
    }
    total - discount(&order.customer, total)
}

fn main() {
    let orders = [
        Order {
            id: 1001,
            customer: Customer { name: "Ada", tier: Tier::Gold, address: Address { city: "London", country: "UK" } },
            lines: &[
                Line { sku: "KB-01", qty: 1, price: 8900 },
                Line { sku: "MS-02", qty: 2, price: 2450 },
                Line { sku: "CB-03", qty: 3, price: 990 },
            ],
        },
        Order {
            id: 1002,
            customer: Customer {
                name: "Grace",
                tier: Tier::Silver,
                address: Address { city: "Arlington", country: "US" },
            },
            lines: &[Line { sku: "MN-27", qty: 1, price: 21900 }, Line { sku: "HS-01", qty: 1, price: 5900 }],
        },
        Order {
            id: 1003,
            customer: Customer {
                name: "Linus",
                tier: Tier::Regular,
                address: Address { city: "Helsinki", country: "FI" },
            },
            lines: &[Line { sku: "SSD-1T", qty: 2, price: 7450 }],
        },
    ];

    let mut revenue = 0;
    for order in &orders {
        let total = order_total(order);
        revenue += total;
        let Customer { name, address, .. } = &order.customer;
        let skus: Vec<&str> = order.lines.iter().map(|line| line.sku).collect();
        println!("#{} {name} ({}, {}): {} = {}", order.id, address.city, address.country, skus.join(" "), money(total));
    }
    println!("revenue {}", money(revenue));
}

fn money(cents: u64) -> String {
    format!("{}.{:02}", cents / 100, cents % 100)
}
