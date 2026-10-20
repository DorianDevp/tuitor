<?php

declare(strict_types=1);

namespace App\Tests;

use App\Entity\Line;
use App\Helper\Money;

final class MoneyTest
{
    public function testBulkRebate(): void
    {
        assert(Money::lineTotal(new Line('MS-02', 12, 2450)) === 27930);
    }
}
