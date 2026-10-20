<?php

declare(strict_types=1);

namespace App\Repository;

use App\Entity\Line;
use App\Entity\Order;

/** Three orders in memory, where a real application would query a database. Prices are in cents. */
final class OrderRepository
{
    public function find(int $id): Order
    {
        return match ($id) {
            1001 => new Order(1001, 'Ada', true, [
                new Line('KB-01', 1, 8900),
                new Line('MS-02', 12, 2450),
                new Line('CB-03', 3, 990),
            ]),
            1002 => new Order(1002, 'Grace', false, [
                new Line('MN-27', 1, 21900),
                new Line('CB-03', 10, 990),
            ]),
            1003 => new Order(1003, 'Linus', false, [new Line('SSD-1T', 2, 7450)]),
            default => throw new \OutOfBoundsException("no order $id"),
        };
    }
}
