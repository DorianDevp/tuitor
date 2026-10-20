<?php

declare(strict_types=1);

namespace App\Event;

use App\Helper\Money;

/** A point for every 10.00 of each line; the kernel registers it, no code calls it by name. */
final class LoyaltyListener
{
    public int $points = 0;

    public function onOrderTotalled(OrderTotalled $event): void
    {
        foreach ($event->order->lines as $line) {
            $this->points += intdiv(Money::lineTotal($line), 1000);
        }
    }
}
