<?php

declare(strict_types=1);

namespace App\Entity;

final class Line
{
    public function __construct(
        public readonly string $sku,
        public readonly int $qty,
        public readonly int $price,
    ) {
    }
}
