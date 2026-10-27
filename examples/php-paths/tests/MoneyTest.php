<?php

declare(strict_types=1);

namespace App\Tests;

use App\Entity\Line;
use App\Helper\Money;

final class MoneyTest
{
    public function testBulkRebate(): void
    {
        // Not assert(): with zend.assertions = -1, as php.ini-production sets it, the call in it
        // would not run at all, and the test would check nothing.
        $total = Money::lineTotal(new Line('MS-02', 12, 2450));
        $total === 27930 || throw new \LogicException("12 × 24.50 less 5% is 279.30, not {$total} cents");
    }
}
