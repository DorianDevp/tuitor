<?php

declare(strict_types=1);

namespace App\Entity;

final class Order
{
    public int $total = 0;

    /** @param list<Line> $lines */
    public function __construct(
        public readonly int $id,
        public readonly string $customer,
        public readonly bool $gold,
        public readonly array $lines,
    ) {
    }
}
