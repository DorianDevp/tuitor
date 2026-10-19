<?php

// Totals a few orders; gold and silver customers get a discount. Prices are in cents.

declare(strict_types=1);

enum Tier
{
    case Regular;
    case Silver;
    case Gold;
}

final class Address
{
    public function __construct(public readonly string $city, public readonly string $country)
    {
    }
}

final class Customer
{
    public function __construct(
        public readonly string $name,
        public readonly Tier $tier,
        public readonly Address $address,
    ) {
    }
}

final class Line
{
    public function __construct(public readonly string $sku, public readonly int $qty, public readonly int $price)
    {
    }
}

final class Order
{
    /** @param list<Line> $lines */
    public function __construct(
        public readonly int $id,
        public readonly Customer $customer,
        public readonly array $lines,
    ) {
    }
}

function lineTotal(Line $line): int
{
    return $line->qty * $line->price;
}

function discount(Customer $customer, int $amount): int
{
    $percent = match ($customer->tier) {
        Tier::Regular => 0,
        Tier::Silver => 5,
        Tier::Gold => 10,
    };
    return intdiv($amount * $percent, 100);
}

function orderTotal(Order $order): int
{
    $total = 0;
    foreach ($order->lines as $line) {
        $total += lineTotal($line);
    }
    return $total - discount($order->customer, $total);
}

function money(int $cents): string
{
    return sprintf('%d.%02d', intdiv($cents, 100), $cents % 100);
}

$orders = [
    new Order(1001, new Customer('Ada', Tier::Gold, new Address('London', 'UK')), [
        new Line('KB-01', 1, 8900),
        new Line('MS-02', 2, 2450),
        new Line('CB-03', 3, 990),
    ]),
    new Order(1002, new Customer('Grace', Tier::Silver, new Address('Arlington', 'US')), [
        new Line('MN-27', 1, 21900),
        new Line('HS-01', 1, 5900),
    ]),
    new Order(1003, new Customer('Linus', Tier::Regular, new Address('Helsinki', 'FI')), [
        new Line('SSD-1T', 2, 7450),
    ]),
];

$revenue = 0;
foreach ($orders as $order) {
    $total = orderTotal($order);
    $revenue += $total;
    $customer = $order->customer;
    $skus = implode(' ', array_map(fn (Line $line) => $line->sku, $order->lines));
    printf(
        "#%d %s (%s, %s): %s = %s\n",
        $order->id,
        $customer->name,
        $customer->address->city,
        $customer->address->country,
        $skus,
        money($total),
    );
}
printf("revenue %s\n", money($revenue));
