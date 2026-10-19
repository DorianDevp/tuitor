<?php

final class OrderService
{
    public function total(array $items): float
    {
        $total = 0.0;
        foreach ($items as $i => $item) {
            if ($item['cancelled']) {
                continue;
            }
            $total += $item['price'] * $item['qty'];
        }
        return $total;
    }
}

$items = [
    ['sku' => 'MS-01', 'price' => 89.99, 'qty' => 1, 'cancelled' => false],
    ['sku' => 'PAD-XL', 'price' => 74.99, 'qty' => 2, 'cancelled' => false],
    ['sku' => 'CBL-2', 'price' => 9.99, 'qty' => 3, 'cancelled' => true],
];
echo (new OrderService())->total($items), "\n";
